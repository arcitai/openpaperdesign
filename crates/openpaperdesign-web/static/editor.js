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
// only the conflict banner's reload (or a later acknowledged save) resolves it.
const OPD_CONFLICT = new Set(['Not saved: conflict']);
// What resolves an outstanding failure or conflict: an acknowledged save, or the explicit
// reload of the version on disk (page.js `discardStep`).
const OPD_RESOLVED = new Set(['Saved', 'Reloaded the version on disk']);

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
 *   lib: { Session, frameStep, saveStep, discardStep },
 *   canvas,                          the canvas element (inside the editor's own root)
 *   status(text, error), renderer(text, error), conflictBox
 *   boot(canvas, page, webgl) -> Promise<editor>
 *   transport: { fetchPage(), put(snapshot, revision), revision() }
 *   firstPage: { text, revision }, forceWebgl, state
 *   onReady(session)                 the renderer is Ready (once per mount)
 *   onHome()                         upstream's Home/New/Open: the host's own way back to its
 *                                    canvases (which saves and drains first); without it the
 *                                    status line says where that is
 *   log(message), env                (see opdBrowserEnv)
 * }
 * Returns { session, started, drain(), resume(), dispose() }. `started` settles once the
 * renderer has booted (or failed); Ready comes later, from the frame loop.
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
    boot: opts.boot,
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
  function wire(c) {
    on(c, 'pointerdown', (e) => {
      e.preventDefault();
      session.edit((ed) => {
        c.focus();
        c.setPointerCapture(e.pointerId);
        capture = [c, e.pointerId];
        mods(ed, e);
        ed.pointer_down(...local(c, e), e.button, e.timeStamp);
      });
    });
    on(c, 'pointermove', (e) => session.edit((ed) => {
      mods(ed, e);
      ed.pointer_move(...local(c, e), e.timeStamp);
    }));
    const up = (e) => {
      if (capture && capture[1] === e.pointerId) capture = null;
      session.edit((ed) => { mods(ed, e); ed.pointer_up(...local(c, e), e.button, e.timeStamp); });
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
        const t = ed.key_down(e.key, e.code, e.repeat);
        flushClipboard(ed);
        return t;
      });
      if (taken) e.preventDefault();
    });
    on(c, 'keyup', (e) => {
      e.stopPropagation();
      session.edit((ed) => { mods(ed, e); ed.key_up(e.key, e.code); });
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

  // ---- what the editor asks the page for, after each frame
  const afterFrame = () => session.withEditor((ed) => {
    if (session.canvas && session.canvas.style) session.canvas.style.cursor = ed.cursor();
    for (let r = ed.take_request(); r !== undefined && r !== null; r = ed.take_request()) {
      if (r === 'save') lib.saveStep(session, ui);
      else if (r === 'home') {
        if (opts.onHome) opts.onHome();
        else opts.status(OPD_NO_HOME);
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
      const c = opts.conflictBox;
      c.replaceChildren();
      c.append('This canvas was changed elsewhere (a page, the page list or the active page, for '
        + 'example by an agent) since your unsaved change. Nothing was overwritten, and your '
        + 'change is not saved. ');
      const b = c.ownerDocument.createElement('button');
      b.type = 'button';
      b.textContent = 'Reload (discard my unsaved change)';
      on(b, 'click', () => lib.discardStep(session, ui));
      c.append(b);
      c.classList.add('show');
    },
    hideConflict: () => {
      const c = opts.conflictBox;
      c.classList.remove('show');
      c.replaceChildren();
    },
  };

  wire(opts.canvas);
  const started = (async () => {
    await session.start(opts.canvas, opts.firstPage, Boolean(opts.forceWebgl));
    if (disposed) return false;
    showRenderer();
    opts.status('Starting renderer…');
    // ---- saving: one request at a time, always with the revision last seen.
    timers.push(env.setInterval(() => { if (!disposed) lib.saveStep(session, ui); }, OPD_SAVE_POLL_MS));
    // ---- changes made elsewhere: reload in place when nothing is unsaved here.
    timers.push(env.setInterval(async () => {
      if (disposed) return;
      try {
        const r = await session.poll();
        if (disposed || session.state.error) return; // a terminal error stays on screen
        if (r === 'reloaded') opts.status('Updated with changes made elsewhere');
        else if (r === 'behind') opts.status('Changed elsewhere; saving your edit will report a conflict');
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
    get disposed() { return disposed; },
  };
}
