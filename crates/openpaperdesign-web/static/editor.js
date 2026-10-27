// The editor mount shared by the browser page (app.js) and the home view's in-panel editor
// (panel.js): DOM input, the frame loop, saving and polling for one Session, all scoped to that
// editor's own canvas, and one disposal that undoes all of it. The canvas shows upstream
// paper-rs's whole workspace (layers, inspector, pages, tools, palette, menus, help), drawn and
// laid out by the editor itself; this mount only forwards input to it and does what it asks
// the page for (save now, go to the canvases, a download, clipboard text).
// It imports nothing: the session, the page steps, the renderer's boot, the page transport and
// the browser environment are passed in, so the same code runs as a file (app.js imports it)
// and inlined in the home view's one module (where the launcher assembles it).

export const OPD_SAVE_POLL_MS = 300;
// How often to look for changes made elsewhere (an agent over MCP, another tab or view).
export const OPD_REVISION_POLL_MS = 1000;
// Upstream's Home, New and Open, in a page that has no canvases view of its own.
const OPD_NO_HOME = 'Open, create or import canvases from the OpenPaperDesign home in Codex. '
  + 'This canvas stays open here.';

// The editor's stable healthy states. Hosts keep them for assistive technology and diagnostics
// (a status live region) but show nothing for them: the workspace itself is the product, and
// upstream's own top bar says when a page has an unsaved edit.
const OPD_QUIET_STATUS = new Set(['Ready', 'Saved', 'Saving…', 'Updated with changes made elsewhere']);
// The way to Ready (the renderer's parts can take a while to arrive): shown as a restrained,
// temporary indication that the next healthy state (Ready) replaces, never as lasting chrome.
const OPD_LOADING_STATUS = new Set(['Starting renderer…', 'Loading the editor…', 'Opening the canvas…']);
// What a save tick reports when the save didn't happen (page.js `saveStep`): it is retried, but
// the change is not saved until a later save is acknowledged.
const OPD_SAVE_FAILED = new Set(['Save failed; retrying', 'Save failed (server unreachable); retrying']);
// A save refused because the canvas changed elsewhere (page.js `saveStep`): nothing is retried;
// only the conflict banner's reload (or a later acknowledged save) resolves it. Likewise a view
// that another, later-opened view took the editing from (issue #23, page.js
// `OPD_READONLY_STATUS`): it stays shown until this view edits again (`OPD_EDITING_STATUS`).
// A view the canvas's editor no longer knows (R78-01) stays so until the tab is reloaded.
const OPD_UNIDENTIFIED_STATUS_TEXT = 'Read-only: this view is no longer known to the canvas';
const OPD_CONFLICT = new Set(['Not saved: conflict', 'Read-only: another view is editing this canvas',
  OPD_UNIDENTIFIED_STATUS_TEXT]);
// What resolves an outstanding failure or conflict: an acknowledged save, the explicit reload of
// the version on disk (page.js `discardStep`), or this view's Edit here (`claimStep`).
const OPD_RESOLVED = new Set(['Saved', 'Reloaded the version on disk', 'Editing here again']);

/**
 * How a host shows a status: 'quiet' (live region only), 'loading' (a small temporary
 * indication until the next state), 'failure' (a save that didn't happen, being retried),
 * 'conflict', 'error' (terminal) or 'note' (shown). Every status also goes to the live region.
 * An error is never quiet or loading.
 */
export function opdStatusKind(text, error) {
  if (error) return 'error';
  const s = String(text);
  if (OPD_QUIET_STATUS.has(s)) return 'quiet';
  if (OPD_LOADING_STATUS.has(s)) return 'loading';
  if (OPD_SAVE_FAILED.has(s)) return 'failure';
  if (OPD_CONFLICT.has(s)) return 'conflict';
  return 'note';
}

/** Whether a status needs nothing visible (see `opdStatusKind`). */
export function opdQuietStatus(text, error) {
  return opdStatusKind(text, error) === 'quiet';
}

/**
 * The status shown for one editor, with the one thing it remembers (R25-F2, R26-F2): a save
 * failure, a conflict or a terminal error stays shown, with its severity, until it is resolved:
 * an acknowledged save ("Saved") or the conflict's explicit reload resolve a failure or
 * conflict; nothing resolves a terminal error but a new editor (a new display). Meanwhile
 * every other status (the retry's "Saving…", Ready, "Updated…", loading, or an informational
 * note such as a refused clipboard copy) is only in the host's live region: the problem stays
 * on screen, and no timer hides it. A newer failure or conflict replaces an older one; neither
 * replaces a terminal error. `show(text, error)` returns what the visible note shows:
 * `{ kind, text, latched }` (`kind` 'quiet' hides it; `latched`: the earlier problem, still
 * unresolved, shown instead of this status).
 */
export function opdStatusDisplay() {
  let outstanding = null;
  return {
    show(text, error) {
      const s = String(text);
      const kind = opdStatusKind(s, error);
      const terminal = Boolean(outstanding) && outstanding.kind === 'error';
      if (OPD_RESOLVED.has(s) && !terminal) outstanding = null;
      if (kind === 'error' || ((kind === 'failure' || kind === 'conflict') && !terminal)) {
        outstanding = { kind, text: s };
        return { kind, text: s, latched: false };
      }
      if (outstanding) return { kind: outstanding.kind, text: outstanding.text, latched: true };
      return { kind, text: s, latched: false };
    },
    get outstanding() { return outstanding ? outstanding.kind : null; },
  };
}

/**
 * A host's visible status note (the browser page's), by `opdStatusDisplay`: every status goes
 * to `live` (a status live region); `note` shows what the display says, with its kind as its
 * class (`loading`, `failure`, `conflict`, `error`, `note`), and only a plain note with no
 * problem outstanding goes after six seconds; each status first cancels the last note's timer,
 * so a stale timer never hides a later problem. `timers`: { setTimeout, clearTimeout }. Returns
 * `status(text, error)`.
 */
export function opdStatusNote(live, note, timers) {
  const display = opdStatusDisplay();
  let timer = 0;
  return (text, error = false) => {
    live.textContent = text;
    timers.clearTimeout(timer);
    const shown = display.show(text, Boolean(error));
    if (shown.kind === 'quiet') {
      note.hidden = true;
      return;
    }
    note.textContent = shown.text;
    note.className = shown.kind;
    note.hidden = false;
    // Only a plain note goes after a while (an unresolved problem is never a plain note here).
    if (shown.kind === 'note' && !display.outstanding) {
      timer = timers.setTimeout(() => { note.hidden = true; }, 6000);
    }
  };
}

/** The appearance preferences: System follows the OS appearance; Light and Dark override it. */
export const OPD_APPEARANCES = Object.freeze(['system', 'light', 'dark']);

/** A kept appearance preference, or 'system' for anything that isn't one (a missing or bad value). */
export function opdAppearance(value) {
  return OPD_APPEARANCES.includes(value) ? value : 'system';
}

/**
 * Whether the OS appearance is dark: the browser's colour-scheme preference (`media`, a
 * `(prefers-color-scheme: dark)` MediaQueryList), else, only where the browser can't say, the
 * host's theme (`hostTheme`, 'dark' or 'light'). `null` when neither says. Presentation only.
 */
export function opdSystemDark(media, hostTheme) {
  if (media && typeof media.matches === 'boolean') return media.matches;
  if (hostTheme === 'dark') return true;
  if (hostTheme === 'light') return false;
  return null;
}

/** The canvas backgrounds (DESIGN.md): a plain surface by default, or dots, or lines. */
export const OPD_BACKGROUNDS = Object.freeze(['none', 'dots', 'lines']);

/** A kept canvas background, or 'none' for anything that isn't one (a missing or bad value). */
export function opdBackground(value) {
  return OPD_BACKGROUNDS.includes(value) ? value : 'none';
}

/** Whether `appearance` shows dark with the OS appearance `systemDark` (see opdSystemDark). */
export function opdResolveDark(appearance, systemDark) {
  const a = opdAppearance(appearance);
  if (a !== 'system') return a === 'dark';
  return systemDark === true;
}

/** The browser environment the mount uses (timers, frames, observers, downloads, clipboard). */
export function opdBrowserEnv(win) {
  return {
    setInterval: (fn, ms) => win.setInterval(fn, ms),
    clearInterval: (id) => win.clearInterval(id),
    requestAnimationFrame: (fn) => win.requestAnimationFrame(fn),
    cancelAnimationFrame: (id) => win.cancelAnimationFrame(id),
    ResizeObserver: win.ResizeObserver,
    AbortController: win.AbortController,
    devicePixelRatio: () => win.devicePixelRatio,
    // The browser's dark colour-scheme query, if this page can ask (see opdSystemDark).
    colorScheme: () => (typeof win.matchMedia === 'function' ? win.matchMedia('(prefers-color-scheme: dark)') : null),
    // A browser download of `bytes`, if this page may start one; never says a file was saved.
    download: (name, mime, bytes) => {
      const doc = win.document;
      if (typeof win.Blob !== 'function' || !win.URL || !doc || !doc.body) return false;
      try {
        const href = win.URL.createObjectURL(new win.Blob([bytes], { type: mime }));
        const a = doc.createElement('a');
        a.href = href;
        a.download = name;
        a.hidden = true;
        doc.body.append(a);
        a.click();
        a.remove();
        win.setTimeout(() => win.URL.revokeObjectURL(href), 60000);
        return true;
      } catch {
        return false;
      }
    },
    // Best effort, outside a copy event (a menu's Copy): the browser may refuse it.
    writeClipboard: (text) => {
      const c = win.navigator && win.navigator.clipboard;
      return c && typeof c.writeText === 'function' ? c.writeText(text) : Promise.reject(new Error('no clipboard'));
    },
  };
}

/** Whether the keyboard shortcuts are the Mac ones. */
export function opdIsMac(nav) {
  const p = (nav && ((nav.userAgentData && nav.userAgentData.platform) || nav.platform)) || '';
  return /mac|iphone|ipad/i.test(p);
}

/**
 * Mount one editor. opts: {
 *   lib: { Session, frameStep, saveStep, discardStep, claimStep, offerStep },
 *                                    `claimStep` (optional) backs the read-only banner's Edit here,
 *                                    `offerStep` (optional) the handover banner's actions
 *   canvas,                          the canvas element (inside the editor's own root)
 *   status(text, error), renderer(text, error), conflictBox
 *   boot(canvas, page, webgl, systemDark, appearance) -> Promise<editor>
 *                                    the OS appearance now (true or false; false, the Light
 *                                    fallback, when nothing says) and the kept preference
 *                                    ('system', 'light' or 'dark')
 *   appearance                       the kept preference to start with (default 'system')
 *   hostTheme()                      the host's theme, the OS fallback (see opdSystemDark)
 *   onAppearance(appearance)         the person chose another one (Menu ▸ Appearance): keep it
 *   background                       the kept canvas background ('none', 'dots' or 'lines';
 *                                    default 'none'), given to each editor once it booted
 *   onBackground(background)         the person chose another one (the canvas's menu, Menu ▸
 *                                    Background, View, the palette or the grid shortcut)
 *   transport: { fetchPage(), put(snapshot, revision), revision(), claim(),
 *                answer(offer, base), applyOffer(id), keepOffer(id), dismissOffer(id),
 *                stopWaiting(id) }
 *                                    `claim()` (optional) takes the canvas's editing for this
 *                                    view; `revision()` may answer { revision, editing,
 *                                    handover, view }; the others (optional) are issue #23's
 *                                    handover (see session.js)
 *   firstPage: { text, revision }, forceWebgl, state
 *   onReady(session)                 the renderer is Ready (once per mount)
 *   onHome()                         upstream's Home/New/Open: the host's own way back to its
 *                                    canvases (which saves and drains first); without it the
 *                                    status line says where that is
 *   onName(name)                     the Canvas's name as the editor has it now (its first,
 *                                    a finished rename, a reload's), once per change: the
 *                                    host's own accessible name of the editor follows it
 *   log(message), env                (see opdBrowserEnv)
 * }
 * Returns { session, started, drain(), resume(), dispose(), hostThemeChanged(), setAppearance(a),
 * appearance, setBackground(b), background }. `setAppearance` and `setBackground`: a kept
 * preference the host learned late (not a new choice).
 * `started` settles once the renderer has booted (or failed); Ready comes later, from the frame
 * loop. `hostThemeChanged()`: the host's theme changed (it matters only where the browser can't
 * report the OS appearance).
 */
export function opdMountEditor(opts) {
  const { lib, transport, env } = opts;
  const abort = new env.AbortController();
  const signal = abort.signal;
  let disposed = false;
  let raf = 0;
  const timers = [];
  // The pointer this editor captured, released on disposal.
  let capture = null;
  const rectOf = (c) => c.getBoundingClientRect();
  // Every listener goes away with one abort.
  const on = (el, type, fn, o = {}) => el.addEventListener(type, fn, Object.assign({}, o, { signal }));

  const session = new lib.Session({
    // Each editor (the first, and a WebGL fallback's) starts in the current appearance, and is
    // given it again once booted: a change while it booted is not lost (see `appearance` below).
    boot: async (canvas, page, webgl) => {
      const ed = await opts.boot(canvas, page, webgl, systemDark(), appearance);
      if (!disposed) {
        // Always the OS appearance as of now, the Light fallback included: a dark signal lost
        // while it booted must not stay.
        ed.set_system_dark(systemDark());
        ed.set_appearance(appearance);
        // The kept background, from the next frame (an older renderer without it shows none).
        if (typeof ed.set_background === 'function') ed.set_background(background);
      }
      return ed;
    },
    fetchPage: transport.fetchPage,
    replaceCanvas: (old) => {
      const fresh = old.cloneNode(false);
      old.replaceWith(fresh);
      wire(fresh);
      return fresh;
    },
    observeResize: (canvas, onResize) => {
      const ro = new env.ResizeObserver(onResize);
      ro.observe(canvas);
      return ro;
    },
    resize: (ed, canvas) => {
      const r = rectOf(canvas);
      ed.resize(r.width, r.height, env.devicePixelRatio());
    },
    put: transport.put,
    revision: transport.revision,
    claim: transport.claim,
    answer: transport.answer,
    applyOffer: transport.applyOffer,
    keepOffer: transport.keepOffer,
    dismissOffer: transport.dismissOffer,
    stopWaiting: transport.stopWaiting,
    log: opts.log,
  }, opts.state || {});

  // ---- input: only reaches the editor while editing is allowed (see session.js). Positions
  // are CSS px in the canvas element: the editor lays out its own workspace inside it and
  // decides what is a panel and what is the canvas.
  const mods = (ed, e) => ed.modifiers(e.shiftKey, e.ctrlKey, e.altKey, e.metaKey);
  const local = (c, e) => { const r = rectOf(c); return [e.clientX - r.left, e.clientY - r.top]; };
  // Clipboard text the editor wrote outside a copy event (a menu's Copy): best effort.
  const flushClipboard = (ed) => {
    const t = ed.take_clipboard_text();
    if (t === undefined || t === null || !env.writeClipboard) return;
    env.writeClipboard(t).catch(() => {
      if (!disposed) opts.status('The browser didn\'t allow copying here; use Ctrl/⌘+C on the canvas.');
    });
  };
  // Pointers pressed and keys held while this view could edit: their moves and releases still
  // reach the editor after it became read-only (`Session.settle`, R78-03), so an interaction
  // already under way ends as the person ends it; nothing new starts.
  const pressed = new Set();
  const held = new Set();
  const settling = (id, set, fn) => (set.has(id) ? session.settle(fn) : session.edit(fn));
  function wire(c) {
    on(c, 'pointerdown', (e) => {
      e.preventDefault();
      session.edit((ed) => {
        c.focus();
        c.setPointerCapture(e.pointerId);
        capture = [c, e.pointerId];
        pressed.add(e.pointerId);
        mods(ed, e);
        ed.pointer_down(...local(c, e), e.button, e.timeStamp);
      });
    });
    on(c, 'pointermove', (e) => settling(e.pointerId, pressed, (ed) => {
      mods(ed, e);
      ed.pointer_move(...local(c, e), e.timeStamp);
    }));
    const up = (e) => {
      if (capture && capture[1] === e.pointerId) capture = null;
      const id = e.pointerId;
      settling(id, pressed, (ed) => { mods(ed, e); ed.pointer_up(...local(c, e), e.button, e.timeStamp); });
      pressed.delete(id);
    };
    on(c, 'pointerup', up);
    on(c, 'pointercancel', up);
    on(c, 'pointerleave', () => session.edit((ed) => ed.pointer_leave()));
    on(c, 'contextmenu', (e) => e.preventDefault());
    on(c, 'wheel', (e) => {
      e.preventDefault();
      session.edit((ed) => {
        mods(ed, e);
        const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? rectOf(c).height : 1;
        ed.wheel(e.deltaX * k, e.deltaY * k, ...local(c, e));
      });
    }, { passive: false });
    // The canvas's keys are the workspace's: the view around it (where Escape means Back)
    // never sees them.
    on(c, 'keydown', (e) => {
      e.stopPropagation();
      const taken = session.edit((ed) => {
        mods(ed, e);
        held.add(e.code);
        const t = ed.key_down(e.key, e.code, e.repeat);
        flushClipboard(ed);
        return t;
      });
      if (taken) e.preventDefault();
    });
    on(c, 'keyup', (e) => {
      e.stopPropagation();
      settling(e.code, held, (ed) => { mods(ed, e); ed.key_up(e.key, e.code); });
      held.delete(e.code);
    });
    on(c, 'focus', () => session.withEditor((ed) => ed.focus(true)));
    on(c, 'blur', () => session.withEditor((ed) => ed.focus(false)));
  }
  // Copy, cut and paste: the page's own clipboard events (fired at the document while the
  // canvas has focus), the supported way for a page to use the clipboard.
  const doc = opts.canvas.ownerDocument;
  if (doc && typeof doc.addEventListener === 'function') {
    const mine = () => session.canvas && doc.activeElement === session.canvas;
    const copy = (cut) => (e) => {
      if (!mine()) return;
      const t = session.edit((ed) => ed.copy(cut));
      if (typeof t === 'string' && e.clipboardData) {
        e.clipboardData.setData('text/plain', t);
        e.preventDefault();
      }
    };
    on(doc, 'copy', copy(false));
    on(doc, 'cut', copy(true));
    on(doc, 'paste', (e) => {
      if (!mine()) return;
      const t = e.clipboardData ? e.clipboardData.getData('text/plain') : '';
      if (!t) return;
      e.preventDefault();
      session.edit((ed) => ed.paste(t));
    });
  }

  // ---- appearance (presentation only). The editor resolves it itself, with egui's own theme
  // preference: the kept preference (System, Light or Dark) and the OS appearance it follows
  // in System (opdSystemDark: the browser's colour-scheme, else the host's theme). The
  // browser's changes are followed live; only the editor shown now is told (not one being
  // replaced by a fallback, which boots in the current appearance). The listener goes with the
  // mount. Nothing here reaches the canvas, its saves or its binding. With no signal at all (no
  // browser colour-scheme and no host theme) the OS appearance is the documented Light fallback
  // (as Home resolves it), delivered like any other: at startup, after boot and when a fallback
  // signal goes away, so an earlier Dark never stays.
  let appearance = opdAppearance(opts.appearance);
  // The canvas background: a view preference like the appearance, never the canvas's content.
  let background = opdBackground(opts.background);
  const media = env.colorScheme ? env.colorScheme() : null;
  const systemDark = () => opdSystemDark(media, opts.hostTheme ? opts.hostTheme() : null) === true;
  const system = () => {
    if (disposed) return;
    const d = systemDark();
    session.withEditor((ed) => ed.set_system_dark(d));
  };
  if (media && typeof media.addEventListener === 'function') on(media, 'change', system);

  // ---- what the editor asks the page for, after each frame
  // The Canvas's name last reported (`onName`), so only a change is.
  let reportedName = null;
  const afterFrame = () => session.withEditor((ed) => {
    // The name the editor has now (R61-F2: the host's region and heading kept the name the
    // editor was opened with). Read after the frame that applied a rename or a reload, from the
    // editor itself; nothing is asked of the server for it.
    if (!disposed && opts.onName && typeof ed.canvas_name === 'function') {
      const name = ed.canvas_name();
      if (typeof name === 'string' && name !== reportedName) {
        reportedName = name;
        opts.onName(name);
      }
    }
    if (session.canvas && session.canvas.style) session.canvas.style.cursor = ed.cursor();
    for (let r = ed.take_request(); r !== undefined && r !== null; r = ed.take_request()) {
      if (r === 'save') lib.saveStep(session, ui);
      else if (r === 'home') {
        if (opts.onHome) opts.onHome();
        else opts.status(OPD_NO_HOME);
      } else if (r === 'appearance') {
        // Menu ▸ Appearance: a view preference, kept by the host where it can keep one.
        appearance = opdAppearance(ed.appearance());
        if (opts.onAppearance) opts.onAppearance(appearance);
      } else if (r === 'background') {
        // A background choice: a view preference, kept by the host where it can keep one.
        background = opdBackground(typeof ed.background === 'function' ? ed.background() : 'none');
        if (opts.onBackground) opts.onBackground(background);
      }
    }
    for (let d = ed.take_download(); d !== undefined && d !== null; d = ed.take_download()) {
      const name = d.name;
      const done = env.download && env.download(name, d.mime, d.bytes);
      if (typeof d.free === 'function') d.free();
      opts.status(done
        ? `Asked the browser to download ${name}; it decides whether and where it is saved.`
        : 'This page can\'t start downloads here, so nothing was exported.');
    }
    flushClipboard(ed);
  });

  const showRenderer = () => {
    const s = session.state;
    opts.renderer(s.fallback
      ? `WebGPU failed: ${s.fallback} — using WebGL (${s.backend})`
      : `Renderer: ${s.backend}`, Boolean(s.fallback));
  };
  // What the editor shows (see page.js). Nothing is shown any more once it is disposed.
  const ui = {
    // Ready only once the renderer validated, presented a frame, and has no GPU problem.
    ready: () => {
      if (disposed) return;
      showRenderer();
      opts.status('Ready');
      session.canvas.focus();
      if (opts.onReady) opts.onReady(session);
      // Opened while another view has the editing (it opened the canvas later): say so at once.
      if (session.state.readonly) ui.readonly();
      else if (session.state.handover) ui.handover();
    },
    // Terminal: shown once, in red, and the frame loop stops.
    fatal: (msg) => {
      if (disposed) return;
      opts.status(msg, true);
      opts.renderer(`Renderer failed: ${msg}`, true);
      opts.log(msg);
    },
    status: (text) => { if (!disposed) opts.status(text); },
    tick: afterFrame,
    // The conflict banner. The unsaved state stays on screen until the person discards it;
    // the button reloads the server's version in place.
    conflict: () => {
      if (disposed) return;
      banner('conflict', 'This canvas was changed elsewhere (a Paper, the list of Papers or the '
        + 'shown Paper, for example by an agent) since your unsaved change. Nothing was '
        + 'overwritten, and your change is not saved. ',
      [['Reload (discard my unsaved change)', () => lib.discardStep(session, ui)]]);
    },
    hideConflict: () => {
      if (shown !== 'conflict') return;
      clearBanner();
      // A change handed over from another view waited behind the conflict.
      if (!session.state.readonly && session.state.handover) ui.handover();
    },
    // Issue #23: another view (opened later) has the canvas's editing. The banner says so, says
    // what happens to what is unsaved here (kept, not saved; handed over to that view, or why
    // not), and offers Edit here. Shown again only when what it says changes; the status line
    // latches the same until this view edits again.
    readonly: () => {
      if (disposed) return;
      const s = session.state;
      const unsaved = session.withEditor((ed) => !ed.can_reload()) === true;
      if (s.unidentified) {
        // R78-01: the server doesn't know this view; nothing from it is admitted.
        banner('readonly', 'This view can\'t edit this canvas any more: its editor doesn\'t know '
          + 'it (the tab was closed, the canvas reopened, or OpenPaperDesign restarted). '
          + (unsaved
            ? 'What is unsaved here stays on screen but can\'t be saved or handed over from here; '
              + 'reloading this tab drops it. '
            : 'Reload this tab to edit here. '), []);
        opts.status(OPD_UNIDENTIFIED_STATUS_TEXT);
        return;
      }
      const lead = 'Another OpenPaperDesign view (one opened after this one) is editing this canvas '
        + 'now';
      const mine = (s.handover && s.handover.offers || []).some((o) => o.from_here);
      let text;
      if (!unsaved) {
        text = `${lead}. This view is read-only and keeps showing what is saved there. `;
      } else if (s.handed === 'change' && mine) {
        text = `${lead}, so your unsaved change here wasn't saved. It was handed over to that `
          + 'view, where it can be applied (only onto the version it was made on, so nothing is '
          + 'overwritten), kept as a new canvas, or its unfinished edits continued; it is still '
          + 'here too, not lost. ';
      } else if (s.handed === 'unfinished' && mine) {
        text = `${lead}. Your unfinished edit here (not in the canvas yet) was handed over to `
          + 'that view, which can continue it there; it also stays here as it is. ';
      } else if (s.handed === 'change' || s.handed === 'unfinished') {
        text = `${lead}. What you handed over was applied, kept or dismissed there; it is still `
          + 'here too, unsaved. ';
      } else {
        text = `${lead}, so your unsaved change here wasn't saved. It is still here, not lost, and `
          + 'is handed over to that view as soon as it can be. Edit here to save it from this view '
          + 'instead; if the canvas was changed there meanwhile, you get a conflict instead of '
          + 'overwriting it. ';
      }
      // What this view keeps of a change applied here, or of an Apply that got no answer, isn't
      // in its editor: say it, and whether the record holds it now (R82-01, run 193).
      const kept = session.keptHere ? session.keptHere() : { applied: 0, unknown: 0 };
      const all = (session.appliedPending ? session.appliedPending().length : 0)
        + (session.uncertainPending ? session.uncertainPending().length : 0);
      if (kept.applied || kept.unknown) {
        text += 'This view also keeps unfinished edits of a change handed over and applied here '
          + '(or whose Apply got no answer); they go to that view, with what guards them, as soon '
          + 'as it can take them, and stay here until then. ';
      } else if (all) {
        text += 'Unfinished edits this view kept of a change applied here were handed over to that '
          + 'view, with what guards them; they also stay here until it takes them up. ';
      }
      const actions = lib.claimStep ? [['Edit here', () => lib.claimStep(session, ui)]] : [];
      // The explicit way to drop what is unsaved here and follow the editing view instead.
      if (unsaved) {
        actions.push(['Reload (discard my unsaved change)', async () => {
          await lib.discardStep(session, ui);
          if (!disposed && session.state.readonly) ui.readonly();
        }]);
      }
      banner('readonly', text, actions);
      opts.status('Read-only: another view is editing this canvas');
    },
    hideReadonly: () => {
      if (shown === 'readonly') clearBanner();
    },
    // In the editing view (R78-03): what other views handed over when they lost the editing,
    // one row each, with what can be done with it here (Apply: a save on the revision it was
    // made on, a conflict if the canvas changed since; Continue here: its unfinished edits open
    // here; Keep as a new canvas; Dismiss), and the views still waited for (Stop waiting).
    // Behind a conflict banner, it waits for that to be resolved.
    handover: () => {
      if (disposed || shown === 'conflict' || session.state.readonly) return;
      const offers = session.offers();
      // Applied here already, its unfinished edits not open here yet (R79-04).
      const kept = session.appliedPending ? session.appliedPending() : [];
      // Applied with no answer, its unfinished edits kept here (run 191); one the record still
      // lists shows on its own row instead.
      const unsure = (session.uncertainPending ? session.uncertainPending() : [])
        .filter((u) => !offers.some((o) => o.id === u.id));
      const now = Date.now();
      // A live view answers within about a second: only one that hasn't by then is shown.
      const waiting = session.waiting().filter((p) => p.ended || now - p.since > 3000);
      if (!offers.length && !kept.length && !unsure.length && !waiting.length) {
        if (shown === 'handover') clearBanner();
        return;
      }
      const step = (action, id) => () => lib.offerStep && lib.offerStep(session, ui, action, id);
      const rows = [];
      for (const k of kept) {
        const continued = session.continued && session.continued.has(k.id);
        const actions = continued ? [] : [['Continue here', step('resume', k.id)]];
        actions.push(['Dismiss', step('dismiss', k.id)]);
        rows.push([`A change handed over from another view was applied here (saved). ${continued
          ? 'Its unfinished edits were opened here; dismiss this when you are done with them'
          : `Its unfinished edits (${k.unfinished.map(describe).join(', ')}) aren't open here yet: `
            + 'continue them here, or dismiss them'}. `, actions]);
      }
      for (const u of unsure) {
        rows.push([`Applying a change handed over from another view got no answer, so it may or may `
          + `not have been saved. Its unfinished edits (${u.unfinished.map(describe).join(', ')}) are `
          + 'kept only here; they aren\'t opened, since it isn\'t known which version they belong '
          + 'to. Dismiss them when you no longer need them. ', [['Dismiss', step('dismiss', u.id)]]]);
      }
      for (const o of offers) {
        if (o.applied) {
          rows.push(['A change handed over from another view was applied here (saved); it is still '
            + 'listed only because tidying up after saving it failed. ', [['Dismiss', step('dismiss', o.id)]]]);
          continue;
        }
        const parts = [];
        if (o.changed) parts.push('an unsaved change');
        for (const c of o.unfinished || []) parts.push(describe(c));
        const stale = o.changed && o.base !== session.state.revision;
        const actions = [];
        if (o.changed && !stale) actions.push(['Apply', step('apply', o.id)]);
        const continued = session.continued && session.continued.has(o.id);
        // Opened here in part (R86-01): what opened, and what still waits to be continued here.
        const progress = !continued && session.offerProgress ? session.offerProgress(o.id) : null;
        const partly = progress && progress.opened.length && progress.waiting.length;
        // Not after an unanswered Apply of it: which version its edits belong to isn't known.
        if (!continued && !o.uncertain && (o.unfinished || []).some(movableEdit)) {
          actions.push(['Continue here', step('resume', o.id)]);
        }
        if (o.canvas) actions.push(['Keep as a new canvas', step('keep', o.id)]);
        actions.push(['Dismiss', step('dismiss', o.id)]);
        rows.push([`Another OpenPaperDesign view lost the editing of this canvas with ${parts.join(', ')
        } and handed it over here${o.uncertain ? ' (an Apply of it got no answer, so it may already '
          + 'be saved; applying it again is checked against the canvas, never saved twice)' : ''}${
          stale ? ' (made on an earlier version of the canvas: applying '
          + 'it would overwrite what was saved since, so it can only be kept as a new canvas or '
          + 'dismissed; nothing is merged)' : ''}${continued ? '. Its unfinished edits were '
          + 'opened here; dismiss this when you are done with them' : ''}${partly ? `. Opened here: ${
            progress.opened.map(describe).join(', ')}; still to continue here: ${
            progress.waiting.map(describe).join(', ')} (once what is being typed is finished or `
          + 'cancelled)' : ''}. `, actions]);
      }
      // The canvas held right now (R91-02, run 202: a change under way, or a window keeping it
      // until it records that a view of it left): nothing can be handed over meanwhile, and
      // whether that view will still answer can't be told from here.
      const held = Boolean(session.state.handover && session.state.handover.held);
      for (const p of waiting) {
        rows.push(p.ended
          ? ['The view that was editing before closed without handing anything over; what it had '
            + 'unsaved, if anything, is gone with it. ', [['OK', step('stop', p.id)]]]
          : held
            ? ['Waiting for the view that was editing before: nothing can be handed over right now, '
              + 'because another OpenPaperDesign window holds this canvas (a change under way, or a '
              + 'window keeping it until it records that a view of it left), so it can\'t be told '
              + 'from here whether that view will still answer. ', [['Stop waiting', step('stop', p.id)]]]
            : ['Waiting for the view that was editing before to hand over what it has: it hasn\'t '
              + 'answered yet (it may be hidden, asleep or disconnected, and answers when it runs '
              + 'again; if its window ended, anything unsaved there is gone). ',
            [['Stop waiting', step('stop', p.id)]]]);
      }
      banner('handover', '', [], rows);
    },
    hideHandover: () => {
      if (shown === 'handover') clearBanner();
    },
    // The keyboard back to this editor's canvas (LEAD185-01): after an unfinished edit was taken
    // up here, its field (or text) is the editor's, and keys only reach the editor through its
    // canvas. The banner button that asked is gone by then, so the page's focus fell to the body.
    // Only this mount's own current canvas, never once it was disposed or its editor replaced;
    // nothing is finished, saved or moved by this (the editor's own field keeps its draft and
    // caret, and asks for its keyboard focus inside the editor).
    focusEditor: () => {
      if (disposed || !session.ed || session.disposed || !session.canvas) return false;
      const c = session.canvas;
      if (typeof c.focus !== 'function') return false;
      c.focus({ preventScroll: true });
      return true;
    },
  };
  // What a handed-over unfinished edit is, for the banner.
  const short = (t) => (String(t).length > 40 ? `${String(t).slice(0, 40)}…` : String(t));
  function describe(c) {
    switch (c && c.kind) {
      case 'title': return `the Canvas name "${short(c.text)}" still being typed`;
      case 'page': return `a Paper's name "${short(c.text)}" still being typed`;
      case 'layer': return `a layer's name "${short(c.text)}" still being typed`;
      case 'text': return `text "${short(c.text)}" still being typed`;
      case 'inspector': return `"${short(c.text)}" still being typed in the Design panel's ${String(c.label || 'field').trim()} field`;
      case 'theme-metric': return `"${short(c.text)}" still being typed as a text style's ${{ size: 'size', 'letter-spacing': 'letter spacing' }[c.metric] || 'line height'} in Theme`;
      case 'gesture': return 'a drag still under way (it is handed over when it ends there)';
      // R82-01 (run 193): a view's kept edits of a change applied there, or of an Apply that
      // got no answer, with what guards them.
      case 'recovery': {
        const inner = Array.isArray(c.packets) ? c.packets.map(describe).join(', ') : 'unfinished edits';
        if (c.v !== 1) return `${inner}, kept by a newer editor in a form this one can't open`;
        return c.state === 'unknown'
          ? `${inner}, kept after an Apply there that got no answer (it may or may not have been saved; they aren't opened)`
          : `${inner}, kept after its change was applied there (they continue only on that version of the canvas)`;
      }
      default: return 'a value still being typed in a field there (it stays there)';
    }
  }
  const movableEdit = (c) => Boolean(c)
    && (['title', 'page', 'layer', 'text', 'inspector', 'theme-metric'].includes(c.kind)
      // A kept recovery of an applied change continues here (on its version: Session checks).
      || (c.kind === 'recovery' && c.v === 1 && c.state === 'applied'));
  // Which banner the shared box shows ('conflict', 'readonly', 'handover' or null), and its
  // text: the same banner with the same text isn't built again (its button keeps the focus).
  let shown = null;
  let shownText = null;
  function banner(kind, text, actions, rows = []) {
    const key = JSON.stringify([text, actions.map((a) => a[0]), rows.map((r) => [r[0], r[1].map((a) => a[0])])]);
    if (shown === kind && shownText === key) return;
    shown = kind;
    shownText = key;
    const c = opts.conflictBox;
    const doc = c.ownerDocument;
    const buttons = (into, list) => {
      for (const [label, fn] of list) {
        const b = doc.createElement('button');
        b.type = 'button';
        b.textContent = label;
        on(b, 'click', fn);
        into.append(b);
      }
    };
    c.replaceChildren();
    if (text) c.append(text);
    buttons(c, actions);
    for (const [t, list] of rows) {
      const row = doc.createElement('div');
      row.append(t);
      buttons(row, list);
      c.append(row);
    }
    c.classList.add('show');
  }
  function clearBanner() {
    shown = null;
    shownText = null;
    const c = opts.conflictBox;
    c.classList.remove('show');
    c.replaceChildren();
  }

  wire(opts.canvas);
  const started = (async () => {
    await session.start(opts.canvas, opts.firstPage, Boolean(opts.forceWebgl));
    if (disposed) return false;
    showRenderer();
    opts.status('Starting renderer…');
    // ---- saving: one request at a time, always with the revision last seen.
    timers.push(env.setInterval(() => { if (!disposed) lib.saveStep(session, ui); }, OPD_SAVE_POLL_MS));
    // ---- changes made elsewhere: reload in place when nothing is unsaved here. A read-only
    // view (another view has the editing) follows them too, and learns here when that changes.
    timers.push(env.setInterval(async () => {
      if (disposed) return;
      try {
        const r = await session.poll();
        if (disposed || session.state.error) return; // a terminal error stays on screen
        const st = session.state;
        if (r === 'unidentified' || r === 'readonly'
          || ((r === 'handed' || r === 'handover') && st.readonly)) ui.readonly();
        else if (r === 'editing') {
          ui.hideReadonly();
          if (st.conflict) ui.conflict();
          else if (st.handover) ui.handover();
          opts.status('Editing here again');
        } else if (r === 'reloaded') opts.status('Updated with changes made elsewhere');
        else if (r === 'behind') opts.status('Changed elsewhere; saving your edit will report a conflict');
        // The handover banner follows the record (and a waited-for view's age) on every poll.
        if (!st.readonly) ui.handover();
      } catch {
        // Unreachable; the save loop reports that.
      }
    }, OPD_REVISION_POLL_MS));
    const frame = (t) => {
      raf = 0;
      if (disposed) return;
      if (lib.frameStep(session, ui, t)) raf = env.requestAnimationFrame(frame);
    };
    raf = env.requestAnimationFrame(frame);
    return true;
  })();

  /** Undo the whole mount, once: listeners, timers, the frame loop, the pointer, the editor. */
  function dispose() {
    if (disposed) return;
    disposed = true;
    abort.abort();
    for (const t of timers) env.clearInterval(t);
    timers.length = 0;
    if (raf) env.cancelAnimationFrame(raf);
    raf = 0;
    if (capture) {
      const [c, id] = capture;
      capture = null;
      try {
        if (c.hasPointerCapture && c.hasPointerCapture(id)) c.releasePointerCapture(id);
      } catch {
        // The canvas is already gone.
      }
    }
    session.dispose();
  }

  return {
    session,
    started,
    drain: () => session.drain(),
    resume: () => session.resume(),
    dispose,
    hostThemeChanged: system,
    // The host learned the kept preference only now (a late read): the editor shown now takes
    // it (one booting later starts in it); it isn't reported back as a new choice.
    setAppearance(a) {
      if (disposed) return;
      appearance = opdAppearance(a);
      session.withEditor((ed) => ed.set_appearance(appearance));
    },
    get appearance() { return appearance; },
    // The host learned the kept background only now (a late read), as `setAppearance`.
    setBackground(b) {
      if (disposed) return;
      background = opdBackground(b);
      session.withEditor((ed) => { if (typeof ed.set_background === 'function') ed.set_background(background); });
    },
    get background() { return background; },
    get disposed() { return disposed; },
  };
}
