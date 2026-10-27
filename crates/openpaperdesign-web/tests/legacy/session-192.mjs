// The page's editor lifecycle: startup, WebGPU-to-WebGL fallback, input gating and saving.
// No DOM or wasm specifics: everything it touches is passed in, so it is tested in plain Node
// (crates/openpaperdesign-web/tests/session.test.mjs).
//
// Rules:
// - Editing input, toolbar actions and saves only reach the editor once it is Ready (validated,
//   a frame presented, no GPU problem) and no fallback is under way. A fallback can only happen
//   before Ready, so no edit made in this tab can be lost or resurrected by it.
// - A fallback waits for a save in flight, then reopens the page as the server has it now
//   (content and revision), on a fresh canvas with its own resize observer.
// - A save's answer is only applied to the editor that took the snapshot.
// - Changes made elsewhere (an agent over MCP, another tab) are polled for. They replace the
//   page in place only when this tab has nothing unsaved (`can_reload`). Otherwise nothing is
//   reloaded: this tab's next save is based on an older revision, the server refuses it (409,
//   nothing written), and the unsaved state stays on screen until the person explicitly
//   discards it (`discard`), which loads the server's version in place. Editing is suspended
//   while that load is under way, so no edit can be made that it would then replace.
// - Leaving (`drain`) first stops input and polling, waits for a save in flight and saves what
//   is left; it reports 'clean' only when the editor has nothing unsaved and no gesture or text
//   edit under way. Anything else (a conflict, a failed or unanswered save, an unfinished edit)
//   keeps the editor; `resume` gives it back to the person.
// - A resize redraws at once (`resized`): reconfiguring the canvas to its new size clears it,
//   and the page would paint that before the next animation frame (a white flash per resize
//   step while a panel divider is dragged).
// - `dispose` ends the session for good, at any point: every later step of a boot, fallback,
//   poll, discard or save finds it disposed and does nothing more. An editor that finishes
//   booting afterwards is freed at once: no observer, canvas, Ready or save comes from it.
// - Read-only (issue #23): the same canvas may be open in several views (tabs, panel mounts,
//   other chats), and only the one opened last edits it. The server says whether this view has
//   the editing (with the canvas and with each revision poll), and refuses a save from a view
//   that doesn't (423, nothing written). Then `readonly` is set: input stops, no save is tried,
//   and an unsaved change stays in the editor as it is (never saved, reloaded, finished or
//   discarded by this; its undo history stays). The revision poll runs in every state (unsaved,
//   unfinished, conflict), so a view always learns that it lost the editing; it only loads the
//   canvas when nothing is unsaved. A clean read-only view keeps showing what the editing view
//   saves. A read-only view answers the handover (`handOver`, R78-03): with what it has unsaved
//   (the editor's own `handover()`: its whole canvas as it stands without an edit still under
//   way, whether that is a change, and each unfinished edit as a continuation: a rename
//   field's text, a text layer being typed), or that it has nothing; never into the canvas.
//   Its own editor keeps all of it as it is. An interaction already under way when the editing
//   moved (a pointer pressed, a key held) still gets its release (`settle`), so it ends as the
//   person ends it and is handed over then; nothing else reaches the editor.
//   The editing view sees the handover record (`handover`: the offers waiting, and the views
//   it still waits for, which may be hidden, asleep, disconnected or ended) and acts on one
//   offer by its id: Apply (`applyOffer`: a save on the revision it was made on, so a change
//   saved since is a conflict, never overwritten; its unfinished edits are then taken up
//   here), Continue here (`resumeOffer`: its unfinished edits only, onto this canvas), Keep as
//   a new canvas (`keepOffer`), Dismiss (`dismissOffer`), or stops waiting for a view
//   (`stopWaiting`). `claim` (the view's Edit here) takes the editing back; saving then
//   resumes, revision-checked as always. A view the server can't identify any more
//   (`unidentified`: released, never issued, or a request without its view) can't edit or
//   claim at all; it says so (reload).

/**
 * The continuations another editor can take up: rename fields, a text being typed, and an
 * unfinished value in a Design panel or Theme field (R78-03; not a gesture's, or a bare `field`
 * notice from an older renderer).
 */
const FIELD_KINDS = ['inspector', 'theme-metric'];
function movable(c) {
  return Boolean(c) && ['title', 'page', 'layer', 'text', ...FIELD_KINDS].includes(c.kind);
}

export class Session {
  /**
   * `page` is what the editor edits as one unit: the whole canvas (every page, the page list and
   * the active page) as the server sends it, and its one canvas revision.
   * deps: {
   *   boot(canvas, page, webgl) -> Promise<editor>   page = { text, revision }
   *   fetchPage() -> Promise<page>                    the canvas as the server has it now
   *   replaceCanvas(old) -> canvas                    a fresh canvas in place of `old`
   *   observeResize(canvas, onResize) -> { disconnect() }
   *   put(snapshot, revision) -> Promise<{ ok, status, revision }>
   *   revision() -> Promise<string | { revision, editing, handover, view }>
   *                                                   the page's current revision (polling),
   *                                                   whether this view has the editing, the
   *                                                   handover record (null or { offers:
   *                                                   [{ id, from_here, changed, unfinished,
   *                                                   base, at }], pending: [{ id, from_here,
   *                                                   since, ended }] }) and why this view
   *                                                   isn't usable (`view`, or null)
   *   claim() -> Promise<{ ok, error }>               optional: take the editing (Edit here)
   *   answer(offer, base) -> Promise<{ ok, status }>  optional: answer the handover; offer is
   *                                                   null or { canvas, changed, unfinished }
   *   applyOffer(id), keepOffer(id), dismissOffer(id), stopWaiting(id)
   *                       -> Promise<{ ok, status, body }>
   *                                                   optional: act on an offer or pending view
   *   log(message)                                    console.error
   *   wait(ms) -> Promise                             optional: a timer (`fieldOutcome`)
   * }
   * A page may carry `editing` (a boolean) too; `false` starts the editor read-only.
   */
  constructor(deps, state = {}) {
    this.deps = deps;
    this.state = Object.assign(state, {
      ready: false, error: null, fallback: null, backend: null, revision: null,
      saves: 0, saving: false, conflict: false, switching: false,
      reloads: 0, behind: false, discarding: false, readonly: false, claiming: false,
      // Issue #23's handover: the record as last polled, what this view handed over
      // ('change', 'unfinished', 'nothing' or null), whether an answer or an offer action is
      // under way, and why the server doesn't know this view (null while it does).
      handover: null, handed: null, handing: false, unidentified: null,
    });
    // Exactly what was handed over last, so the same is answered once.
    this.handedKey = null;
    // An interaction's release reached the editor while read-only (`settle`): what it holds
    // may have changed, so it is looked at again before the next answer.
    this.settled = false;
    // Offers whose unfinished edits were taken up here (not taken up twice), and what the last
    // take-up did (`takeUp`).
    this.continued = new Set();
    this.lastTaken = 0;
    this.lastRefusals = [];
    this.lastFieldTaken = false;
    // The continuations not taken up by the last `takeUp`.
    this.lastLeft = [];
    // Offers applied here (their change is committed on the server) whose unfinished edits
    // aren't open here yet: id -> { unfinished, revision }, kept until they are taken up here
    // or dismissed (R79-04). Never applied again.
    this.applied = new Map();
    // Offers applied here (committed), also without continuations: never offered for Apply
    // again, even while the record still lists one (its removal failed).
    this.committed = new Set();
    // Offer actions (and the load and take-up after an Apply) started and not ended: leaving
    // waits for their outcome (`drain`, R80-02), and the page's leave guard counts them.
    this.actionDone = Promise.resolve();
    this.acting = 0;
    // Offers whose Apply got no answer (run 191): it may or may not have been saved, and
    // nothing here can tell which. id -> { unfinished }: their unfinished edits exactly as
    // they were offered, kept until the person dismisses them. No poll settles that (R81-01,
    // run 192): an answer read before the Apply, or while it still runs on the server (reads
    // aren't fenced against writes), can still list the offer the Apply then consumes.
    this.uncertain = new Map();
    this.ed = null;
    this.canvas = null;
    this.observer = null;
    this.forceWebgl = false;
    this.errorReported = false;
    // Bumped whenever something that can change the editor's page starts (a save, a poll, a
    // discard): an answer only applies if nothing started after its own request.
    this.generation = 0;
    this.polling = false;
    // The page clock of the last animation frame drawn (a resize redraws at it, see `resized`).
    this.lastFrameTime = undefined;
    this.saveDone = Promise.resolve();
    this.discardDone = Promise.resolve();
    // Set by `drain` (no input or polling while leaving) and `dispose` (for good).
    this.closing = false;
    this.disposed = false;
    // Once Ready, edits were possible: a later error (which clears `ready`) can't make an
    // editor with unsaved work look clean (`drain`).
    this.wasReady = false;
  }

  /** May the user edit right now? Not while read-only (another view has the editing). */
  editable() {
    return this.viewable() && !this.state.readonly;
  }

  /** Is there an editor showing the page that may follow the server (poll, reload)? */
  viewable() {
    const s = this.state;
    return Boolean(this.ed) && !this.disposed && !this.closing && s.ready && !s.switching
      && !s.error && !s.discarding;
  }

  /** What the server says about this view's editing, if it says (`editing`, a boolean). */
  noteEditing(editing) {
    if (typeof editing !== 'boolean') return;
    this.state.readonly = !editing;
    // A view that edits saves its own change; what it handed over before is no longer its news.
    if (editing) {
      this.state.handed = null;
      this.handedKey = null;
    }
  }

  /** Run `fn(editor)` only when editing is allowed (input, toolbar). */
  edit(fn) {
    if (!this.editable()) return undefined;
    return fn(this.ed);
  }

  /**
   * Run `fn(editor)` for the continuation or release of an interaction that started while this
   * view could edit (a pressed pointer's moves and release, a held key's release), also while
   * read-only: it ends as the person ends it, and is handed over then (R78-03). The caller
   * decides what started before; nothing new starts this way.
   */
  settle(fn) {
    if (!this.ed || this.disposed || this.state.switching || this.state.discarding) {
      return undefined;
    }
    if (this.state.readonly) this.settled = true;
    return fn(this.ed);
  }

  /** Run `fn(editor)` when there is an editor that isn't being replaced (resize, frames). */
  withEditor(fn) {
    if (!this.ed || this.disposed || this.state.switching) return undefined;
    return fn(this.ed);
  }

  async start(canvas, page, forceWebgl) {
    this.canvas = canvas;
    this.forceWebgl = forceWebgl;
    this.state.revision = page.revision;
    try {
      await this.open(page, forceWebgl);
    } catch (e) {
      if (this.disposed) return;
      if (forceWebgl) throw e;
      await this.fallBack(e && e.message ? e.message : String(e));
    }
  }

  async open(page, webgl) {
    const ed = await this.deps.boot(this.canvas, page, webgl);
    if (this.disposed) {
      // Left while it booted: this editor never becomes the session's.
      ed.free();
      return;
    }
    this.ed = ed;
    this.state.revision = page.revision;
    this.noteEditing(page.editing);
    this.state.backend = ed.backend();
    this.observer = this.deps.observeResize(this.canvas, () => this.resized());
  }

  resized() {
    this.withEditor((ed) => {
      if (!this.deps.resize) return;
      this.deps.resize(ed, this.canvas);
      // A new backing size (the editor reconfigures its GPU surface to it) clears what the
      // canvas shows, and the page paints right after resize observers run, before the next
      // animation frame: without a picture now, every resize step of a continuous resize would
      // show a cleared (white) canvas once. So the picture is drawn again here, in the same task
      // as the reconfigure, at the last frame's time (no animation advances). Only after a first
      // frame, and never over a terminal error; Ready, errors and fallback stay the frame loop's
      // (`frame` reports them on the next animation frame, as before).
      if (this.lastFrameTime !== undefined && !this.state.error) ed.frame(this.lastFrameTime);
    });
  }

  /** WebGPU failed before Ready: reopen with WebGL2 on a fresh canvas, visibly. */
  async fallBack(reason) {
    const s = this.state;
    s.switching = true;
    try {
      this.deps.log(`WebGPU failed, falling back to WebGL: ${reason}`);
      s.fallback = reason;
      await this.saveDone;
      if (this.disposed) return;
      const page = await this.deps.fetchPage();
      if (this.disposed) return;
      if (this.observer) { this.observer.disconnect(); this.observer = null; }
      if (this.ed) { this.ed.free(); this.ed = null; }
      this.canvas = this.deps.replaceCanvas(this.canvas);
      await this.open(page, true);
    } finally {
      s.switching = false;
    }
  }

  /**
   * One animation frame. Returns 'ready' the first time the editor becomes Ready, 'error'
   * once when a problem has ended editing (including a fallback that failed asynchronously),
   * 'stopped' on every frame after that, 'fallback' when a fallback was started, else
   * undefined.
   */
  frame(t) {
    const s = this.state;
    if (this.disposed) return 'stopped';
    if (s.error) {
      if (this.errorReported) return 'stopped';
      this.errorReported = true;
      return 'error';
    }
    if (s.switching || !this.ed) return undefined;
    this.lastFrameTime = t;
    this.ed.frame(t);
    const gpuError = this.ed.gpu_error();
    if (gpuError) {
      if (!s.ready && !this.forceWebgl && !s.fallback) {
        this.fallBack(gpuError).catch((e) => this.fail(`Renderer failed: ${e && e.message ? e.message : e}`));
        return 'fallback';
      }
      this.fail(`Renderer error: ${gpuError}`);
      this.errorReported = true;
      return 'error';
    }
    if (!s.ready && this.ed.ready()) {
      s.ready = true;
      this.wasReady = true;
      return 'ready';
    }
    return undefined;
  }

  fail(msg) {
    if (this.disposed) return;
    this.state.ready = false;
    this.state.error = msg;
    this.deps.log(msg);
  }

  /**
   * Look for a newer revision on the server, and learn this view's editing and the handover
   * record. Returns, the first that applies: 'unidentified' when the server no longer knows
   * this view, 'readonly' or 'editing' when this view's editing changed, 'handed' when it
   * answered the handover just now (see `handOver`), 'handover' when the record changed,
   * 'reloaded' when the page was replaced in place, 'behind' once when it changed but this view
   * has something unsaved (its next save will be a conflict), else undefined.
   */
  async poll() {
    const s = this.state;
    // It polls in every state, unsaved, unfinished or in conflict too: whatever this view
    // holds, it learns that it lost the editing (issue #23). Nothing is loaded over an unsaved
    // change (`pollOnce`).
    if (this.polling || !this.viewable() || s.saving || s.claiming || s.handing) {
      return undefined;
    }
    this.polling = true;
    try {
      return await this.pollOnce();
    } finally {
      this.polling = false;
    }
  }

  async pollOnce() {
    const s = this.state;
    const gen = ++this.generation;
    const ed = this.ed;
    // Apply an answer only to the editor that asked, if no save, discard or other load
    // started since this request.
    const current = () => this.ed === ed && gen === this.generation && this.viewable()
      && !s.saving;
    const answer = await this.deps.revision();
    if (!current()) return undefined;
    const meta = answer && typeof answer === 'object';
    const rev = meta ? answer.revision : answer;
    const wasReadonly = s.readonly;
    const wasHandover = JSON.stringify(s.handover);
    const wasUnidentified = s.unidentified;
    if (meta) {
      this.noteEditing(answer.editing);
      if ('handover' in answer) s.handover = answer.handover || null;
      if ('view' in answer) s.unidentified = answer.view || null;
    }
    if (s.unidentified) s.readonly = true;
    // A change of this view's editing is reported first (the host shows or hides read-only).
    let note;
    if (s.unidentified && !wasUnidentified) note = 'unidentified';
    else if (wasReadonly !== s.readonly) note = s.readonly ? 'readonly' : 'editing';
    if (s.readonly && !s.unidentified) {
      const h = await this.handOver();
      if (!current()) return note;
      note = note || h;
    }
    if (!note && JSON.stringify(s.handover) !== wasHandover) note = 'handover';
    if (rev === s.revision) {
      s.behind = false;
      return note;
    }
    // In conflict, that was said already; otherwise "behind" is said once per change.
    if (s.conflict || !ed.can_reload()) {
      const said = s.behind || s.conflict;
      s.behind = true;
      return note || (said ? undefined : 'behind');
    }
    const page = await this.deps.fetchPage();
    if (!current() || s.conflict || !ed.can_reload()) {
      const said = s.behind;
      s.behind = true;
      return note || (said || !current() ? undefined : 'behind');
    }
    ed.reload(page.text);
    s.revision = page.revision;
    this.noteEditing(page.editing);
    s.reloads += 1;
    s.behind = false;
    return note || 'reloaded';
  }

  /** Whether the editing view still waits for this view's answer. */
  awaited() {
    const h = this.state.handover;
    return Boolean(h && h.pending && h.pending.some((p) => p.from_here && !p.ended));
  }

  /**
   * A read-only view answers the handover (issue #23, R78-03), once per content: what it has
   * unsaved, as its editor hands it over (`handover()`: its whole canvas without an edit still
   * under way, whether that is a change, its unfinished edits as continuations), or that it
   * has nothing (only while the editing view waits for it). Its editor keeps everything as it
   * is: undo history, open fields and all. Returns 'handed', or undefined when nothing was
   * answered (nothing new, refused or unreachable: tried again on the next poll).
   */
  async handOver() {
    const s = this.state;
    const ed = this.ed;
    if (!this.deps.answer || !ed || this.disposed || !s.readonly || s.unidentified || s.handing
      || s.saving || s.claiming) {
      return undefined;
    }
    let offer = null;
    if (!ed.can_reload()) {
      // Looked at again only when something may have changed (the first time, or after an
      // interaction settled), not on every poll.
      if (this.handedKey !== null && !this.settled && !this.awaited()) return undefined;
      this.settled = false;
      offer = this.handoverOf(ed);
    }
    // What was applied here and isn't open here yet, or whose Apply got no answer, is never
    // handed on (R81-02, run 192): an answer carries plain unfinished edits on this view's
    // revision, which would drop the revision they require or that their outcome is unknown,
    // and the view receiving them couldn't enforce either. It stays kept here, with its reason
    // (`drain`), to continue or dismiss once this view edits again.
    const key = JSON.stringify([s.revision, offer]);
    if (key === this.handedKey && !this.awaited()) return undefined;
    // Nothing to hand over and nobody waiting: nothing to say.
    if (!offer && !this.awaited()) {
      this.handedKey = key;
      return undefined;
    }
    s.handing = true;
    try {
      const r = await this.deps.answer(offer, s.revision);
      if (this.ed !== ed || this.disposed || !s.readonly || !(r && r.ok)) return undefined;
      this.handedKey = key;
      s.handed = !offer ? 'nothing' : offer.changed ? 'change' : 'unfinished';
      return 'handed';
    } catch {
      return undefined;
    } finally {
      s.handing = false;
    }
  }

  /** What `ed` has unsaved, as it hands it over (null: nothing it can say). */
  handoverOf(ed) {
    if (typeof ed.handover !== 'function') return null;
    let h;
    try {
      h = JSON.parse(ed.handover());
    } catch {
      return null;
    }
    const unfinished = Array.isArray(h.unfinished) ? h.unfinished : [];
    if (!h.changed && unfinished.length === 0) return null;
    return { canvas: h.changed ? h.canvas : '', changed: Boolean(h.changed), unfinished };
  }

  /**
   * Whether leaving now would lose something (the page's leave guard, R80-02): the editor's
   * unsaved or unfinished work, a committed Apply's continuations not open here yet, or an
   * offer action whose outcome isn't known yet.
   */
  holdsWork() {
    const ed = this.ed;
    const editor = Boolean(ed && typeof ed.unsaved_work === 'function' && ed.unsaved_work());
    return editor || this.applied.size > 0 || this.uncertainPending().length > 0
      || this.acting > 0;
  }

  /** `p`, an offer action started now: followed until it ends (`drain` waits for it). */
  track(p) {
    this.acting += 1;
    const ended = p.then(() => {}, () => {}).then(() => { this.acting -= 1; });
    this.actionDone = Promise.all([this.actionDone, ended]).then(() => {});
    return p;
  }

  /** Until every offer action started so far has ended (also one started meanwhile). */
  async actionsEnded() {
    let seen;
    while (seen !== this.actionDone) {
      seen = this.actionDone;
      await seen;
    }
  }

  /**
   * The offers other views handed over, as the editing view sees them. One applied here
   * already that its record still lists (its removal failed) shows as applied (`applied`,
   * nothing to apply: only to dismiss), and one whose continuations are still kept here is
   * listed by `appliedPending` instead.
   */
  offers() {
    const h = this.state.handover;
    if (!h || !Array.isArray(h.offers)) return [];
    return h.offers
      .filter((o) => !o.from_here && !this.applied.has(o.id))
      .map((o) => (this.committed.has(o.id) ? { ...o, changed: false, applied: true }
        : this.uncertain.has(o.id) ? { ...o, uncertain: true } : o));
  }

  /** Whether the record as last polled lists offer `id` (a hint, never proof of an outcome). */
  listed(id) {
    const h = this.state.handover;
    return Boolean(h && Array.isArray(h.offers) && h.offers.some((o) => o.id === id));
  }

  /**
   * Offers whose Apply got no answer (run 191): `{ id, unfinished, uncertain: true }`. Their
   * change may or may not have been saved; their unfinished edits are kept here exactly until
   * the person dismisses them, never opened or applied again here, whatever the record shows
   * (R81-01).
   */
  uncertainPending() {
    return [...this.uncertain].map(([id, u]) => ({ id, unfinished: u.unfinished, uncertain: true }));
  }

  /**
   * Offers applied here whose unfinished edits aren't open here yet (R79-04): `{ id,
   * unfinished, applied: true }`, to continue here or dismiss.
   */
  appliedPending() {
    return [...this.applied].map(([id, p]) => ({ id, unfinished: p.unfinished, applied: true }));
  }

  /** The views the editing view waits for (`ended`: they went without answering). */
  waiting() {
    const h = this.state.handover;
    return h && Array.isArray(h.pending) ? h.pending.filter((p) => !p.from_here) : [];
  }

  /**
   * The editing view applies offer `id` (R78-04): a save on the revision it was made on
   * (server-checked, admitted with this view's editing), so one saved since is a conflict and
   * nothing is overwritten. Only while this view has nothing unsaved itself ('unsaved': it
   * saves first). Then it is loaded here and its unfinished edits are taken up here, as they
   * were there, once that is safe (R79-04): only onto the canvas it was applied to, and only
   * while nothing is unsaved or being typed here (a draft begun while the Apply was answered
   * is this view's own, newer, and is never replaced). Whatever isn't taken up then is kept,
   * exactly, until it is (`appliedPending`, Continue here) or dismissed; the change stays
   * committed and is never applied again. Returns 'applied' ('applied-left' when the server
   * couldn't remove the offer after saving it: it still waits, to dismiss) when its unfinished
   * edits, if any, are open here; 'applied-partial' or 'applied-unopened' when some or all of
   * them aren't yet (`lastRefusals` says why); 'conflict', 'readonly', 'gone', 'failed', or
   * undefined (also when this editor was replaced or disposed meanwhile). Leaving waits for
   * its outcome (`track`).
   */
  applyOffer(id) {
    return this.track(this.applyOfferNow(id));
  }

  async applyOfferNow(id) {
    const s = this.state;
    const offer = this.offers().find((o) => o.id === id);
    if (!this.deps.applyOffer || !this.editable() || s.handing || !offer || !offer.changed) {
      return undefined;
    }
    if (s.saving || s.conflict || !this.ed.can_reload()) return 'unsaved';
    const ed = this.ed;
    // What this Apply takes up here, and nothing an earlier action took up.
    this.lastTaken = 0;
    this.lastRefusals = [];
    this.lastFieldTaken = false;
    this.lastLeft = [];
    // Its unfinished edits exactly as offered, copied before it is sent (run 191): if its
    // answer never comes, the record may have dropped them with a commit nobody here saw.
    const intent = JSON.parse(JSON.stringify((offer.unfinished || []).filter(movable)));
    const r = await this.offerAction(() => this.deps.applyOffer(id), id);
    if (r === 'unknown') {
      // Not applied again, not opened, not called saved or unsaved: kept, with why.
      if (intent.length) this.uncertain.set(id, { unfinished: intent });
      this.lastRefusals = [`Codex didn't answer, so it isn't known whether it was applied (it may have been saved)${
        intent.length ? '; its unfinished edits are kept here until you dismiss them' : ''}`];
      return 'unknown';
    }
    if (r !== 'ok') return r;
    this.uncertain.delete(id);
    // Committed: from here on it is never applied again, and its continuation is kept until
    // it is open here.
    const body = this.lastBody || {};
    this.committed.add(id);
    const wanted = (offer.unfinished || []).filter(movable);
    if (wanted.length) this.applied.set(id, { unfinished: wanted, revision: body.revision });
    if (this.disposed || this.ed !== ed) return undefined;
    const opened = wanted.length ? await this.openApplied(id) : 'resumed';
    if (this.disposed || this.ed !== ed) return undefined;
    if (opened === 'resumed') return body.left ? 'applied-left' : 'applied';
    return opened === 'partial' ? 'applied-partial' : 'applied-unopened';
  }

  /**
   * Take up applied offer `id`'s kept continuations (`applied`) here: onto the canvas it was
   * applied to (loaded first, when this view still shows an older one), only while this view
   * edits, and while nothing is unsaved or being typed here. Every await is checked against
   * this editor and that entry; a refusal keeps it as it was. Whatever is taken up leaves it,
   * except a field's value, which opens over the editor's next frames: it stays until dismissed
   * (`continued`, `fieldOutcome`). Returns 'resumed', 'partial', 'unopened' (`lastRefusals`
   * says why), or undefined (nothing kept, or this editor went meanwhile).
   */
  async openApplied(id) {
    const s = this.state;
    const ed = this.ed;
    const p = this.applied.get(id);
    this.lastTaken = 0;
    this.lastRefusals = [];
    this.lastFieldTaken = false;
    if (!p || !ed) return undefined;
    const mine = () => !this.disposed && this.ed === ed && this.applied.get(id) === p;
    const unopened = (why) => {
      this.lastRefusals = [why];
      return 'unopened';
    };
    const busy = 'something is unsaved or being typed here; finish it, then continue this';
    // Only onto exactly the version it was applied to (R80-01, run 190): a newer accepted
    // change (an agent's, or one saved here since) is never typed over, nor reloaded over.
    const newer = 'a newer change was saved since it was applied, so it isn\'t opened over it '
      + '(it stays kept here: dismiss it when you no longer need it)';
    const leaving = 'this view is being left; it stays kept here';
    const notEditing = 'this view isn\'t editing the canvas now';
    if (this.closing) return unopened(leaving);
    if (!this.editable() || s.handing) return unopened(notEditing);
    if (s.saving || !ed.can_reload()) return unopened(busy);
    if (p.revision === undefined) {
      return unopened('it isn\'t known which version of the canvas it was applied to');
    }
    if (s.revision !== p.revision) {
      const loaded = await this.reloadNow();
      if (!mine()) return undefined;
      if (loaded !== 'reloaded') {
        return unopened(loaded === 'unsaved' ? busy
          : 'the canvas it was applied to couldn\'t be loaded here yet; continue it once it is');
      }
      if (s.revision !== p.revision) return unopened(newer);
    }
    if (this.closing) return unopened(leaving);
    if (!this.editable()) return unopened(notEditing);
    if (s.saving || !ed.can_reload()) return unopened(busy);
    const taken = this.takeUp(p.unfinished);
    const refusals = this.lastRefusals;
    // Still to do here: what wasn't taken up, and a field value until it is dismissed.
    const left = [...this.lastLeft, ...p.unfinished.filter((c) => FIELD_KINDS.includes(c.kind)
      && !this.lastLeft.includes(c))];
    if (left.length) this.applied.set(id, { ...p, unfinished: left });
    else this.applied.delete(id);
    if (this.lastFieldTaken) this.continued.add(id);
    this.lastRefusals = refusals;
    if (taken === 0) return 'unopened';
    return this.lastLeft.length ? 'partial' : 'resumed';
  }

  /**
   * Continue here: take up offer `id`'s unfinished edits in this editor, onto this canvas, as
   * edits under way (a rename field with its text, a text layer being typed): nothing is
   * finished or saved by this. An offer with nothing else is then dismissed (it was taken up);
   * one with a change stays (to apply or keep). Returns 'resumed', 'partial' (some couldn't be
   * taken up here: their Paper or layer isn't in this canvas), 'untaken' (none could: the offer
   * stays as it was, to continue again or dismiss; `lastRefusals` says why), 'unsaved', or
   * undefined.
   */
  resumeOffer(id) {
    return this.track(this.resumeOfferNow(id));
  }

  async resumeOfferNow(id) {
    const s = this.state;
    // Its Apply got no answer: which version its edits belong to isn't known, so they aren't
    // opened here (run 191), also while the record still lists it (R81-02: that proves neither
    // outcome, nor which version they may safely be typed over).
    if (this.uncertain.has(id)) {
      if (!this.editable()) return undefined;
      this.lastTaken = 0;
      this.lastFieldTaken = false;
      this.lastRefusals = ['its Apply got no answer, so it isn\'t known which version of the canvas '
        + 'its edits belong to; it stays kept here (dismiss it when you no longer need it)'];
      return 'untaken';
    }
    // One applied here already: its kept continuations, onto the canvas it was applied to.
    if (this.applied.has(id)) {
      if (!this.editable() || s.handing || this.continued.has(id)) return undefined;
      const r = await this.openApplied(id);
      if (r === undefined) return undefined;
      return r === 'unopened' ? 'untaken' : r;
    }
    const offer = this.offers().find((o) => o.id === id);
    if (!this.editable() || s.handing || !offer || !offer.unfinished || !offer.unfinished.length
      || this.continued.has(id)) {
      return undefined;
    }
    if (!this.ed.can_reload()) return 'unsaved';
    const wanted = offer.unfinished.filter(movable);
    const taken = this.takeUp(offer.unfinished);
    if (taken > 0) this.continued.add(id);
    // A field's value is taken up over the editor's next frames (it opens, then gets the typed
    // text): its offer stays until the person dismisses it, so nothing is lost if that field
    // doesn't open here. One with nothing else, all taken up at once, is dismissed.
    const later = wanted.some((c) => FIELD_KINDS.includes(c.kind));
    if (!offer.changed && taken === wanted.length && !later) {
      await this.dismissOffer(id);
    }
    if (taken === 0) return 'untaken';
    return taken === wanted.length ? 'resumed' : 'partial';
  }

  /**
   * Take up `continuations` in this editor; returns how many were. `lastTaken` and
   * `lastRefusals` (the editor's reason for each one it didn't take up: its target is missing,
   * locked or changed, say) say what happened.
   */
  takeUp(continuations) {
    const ed = this.ed;
    this.lastTaken = 0;
    this.lastRefusals = [];
    this.lastFieldTaken = false;
    this.lastLeft = [];
    if (!ed || typeof ed.resume_unfinished !== 'function' || !Array.isArray(continuations)) return 0;
    let n = 0;
    for (const c of continuations.filter(movable)) {
      try {
        // The renderer answers '' (taken up) or why not; an older one true or false.
        const r = ed.resume_unfinished(JSON.stringify(c));
        if (r === true || r === '') {
          n += 1;
          if (FIELD_KINDS.includes(c.kind)) this.lastFieldTaken = true;
        } else {
          this.lastRefusals.push(typeof r === 'string' ? r : 'it couldn\'t be taken up here');
          this.lastLeft.push(c);
        }
      } catch {
        this.lastRefusals.push('it couldn\'t be taken up here');
        this.lastLeft.push(c);
      }
    }
    this.lastTaken = n;
    return n;
  }

  /**
   * What became of a field value just taken up (`takeUp`), which opens over the editor's next
   * frames: '' once it is in its field here, else the editor's reason (its field didn't show
   * here), and then offer `id` can be continued again (its offer was left listed). Undefined
   * when no field value was taken up, when it is still opening after the wait, or once this
   * editor was replaced or disposed meanwhile (never reported for another one).
   */
  async fieldOutcome(id, tries = 40) {
    const ed = this.ed;
    if (!this.lastFieldTaken || !ed || typeof ed.resume_outcome !== 'function') return undefined;
    const wait = this.deps.wait || ((ms) => new Promise((r) => { setTimeout(r, ms); }));
    for (let i = 0; i < tries; i += 1) {
      if (this.disposed || this.ed !== ed) return undefined;
      let why;
      try {
        why = ed.resume_outcome();
      } catch {
        return undefined;
      }
      if (why === '') return '';
      if (typeof why === 'string' && why !== 'still opening') {
        this.continued.delete(id);
        this.lastRefusals = [why];
        return why;
      }
      await wait(50);
    }
    return undefined;
  }

  /** Keep offer `id` as a new library canvas ('kept', with `lastBody.kept`, else as Apply). */
  async keepOffer(id) {
    if (!this.deps.keepOffer || !this.editable() || this.state.handing
      || !this.offers().some((o) => o.id === id)) {
      return undefined;
    }
    const r = await this.offerAction(() => this.deps.keepOffer(id), id);
    return r === 'ok' ? 'kept' : r;
  }

  /**
   * Dismiss offer `id` ('dismissed', else as Apply). One applied here already: its kept
   * continuations go here (the person's explicit choice), and its record's listing too when the
   * record kept it; its committed change stays.
   */
  async dismissOffer(id) {
    // An unanswered Apply's kept edits: the person's explicit choice drops them (run 191).
    if (this.uncertain.has(id)) {
      if (!this.editable() || this.state.handing) return undefined;
      const listed = this.listed(id);
      this.uncertain.delete(id);
      if (!listed) return 'dismissed';
    }
    if (this.applied.has(id)) {
      if (!this.editable() || this.state.handing) return undefined;
      this.applied.delete(id);
      this.continued.delete(id);
      const h = this.state.handover;
      const listed = h && Array.isArray(h.offers) && h.offers.some((o) => o.id === id);
      if (listed && this.deps.dismissOffer) {
        const r = await this.offerAction(() => this.deps.dismissOffer(id), id);
        return r === 'ok' || r === 'gone' ? 'dismissed' : r;
      }
      return 'dismissed';
    }
    if (!this.deps.dismissOffer || !this.editable() || this.state.handing
      || !this.offers().some((o) => o.id === id)) {
      return undefined;
    }
    const r = await this.offerAction(() => this.deps.dismissOffer(id), id);
    return r === 'ok' ? 'dismissed' : r;
  }

  /** Stop waiting for pending view `id` ('stopped', else as Apply). */
  async stopWaiting(id) {
    if (!this.deps.stopWaiting || !this.editable() || this.state.handing
      || !this.waiting().some((p) => p.id === id)) {
      return undefined;
    }
    const r = await this.offerAction(() => this.deps.stopWaiting(id), id);
    return r === 'ok' ? 'stopped' : r;
  }

  offerAction(call, id) {
    return this.track(this.offerActionNow(call, id));
  }

  async offerActionNow(call, id) {
    const s = this.state;
    s.handing = true;
    this.lastBody = null;
    const drop = () => {
      if (!s.handover) return;
      s.handover = {
        offers: (s.handover.offers || []).filter((o) => o.id !== id),
        pending: (s.handover.pending || []).filter((p) => p.id !== id),
      };
    };
    try {
      const r = await call();
      if (this.disposed) return undefined;
      this.lastBody = r && r.body;
      if (r && r.ok) {
        if (!(r.body && r.body.left)) drop();
        return 'ok';
      }
      const status = r && r.status;
      // No answer (status 0: Codex didn't answer or run it, or its answer was unreadable):
      // whether it happened isn't known (run 191). Every answered status keeps its meaning.
      if (!status) return 'unknown';
      if (status === 409) return 'conflict';
      if (status === 423) {
        s.readonly = true;
        return 'readonly';
      }
      if (status === 404) {
        drop();
        return 'gone';
      }
      return 'failed';
    } catch {
      // Sent, and no answer came back (the network, a timeout): not known either.
      return 'unknown';
    } finally {
      s.handing = false;
    }
  }

  /**
   * Load the canvas as the server has it now, if nothing is unsaved here (after an Apply).
   * Returns 'reloaded', 'unsaved' (something is unsaved or under way here: nothing loaded),
   * 'stale' (another load, save or editor started meanwhile) or 'failed' (no answer: the next
   * poll loads it).
   */
  async reloadNow() {
    const s = this.state;
    const ed = this.ed;
    if (!ed || this.disposed) return 'stale';
    if (!ed.can_reload() || s.saving) return 'unsaved';
    const gen = ++this.generation;
    try {
      const page = await this.deps.fetchPage();
      if (this.disposed || this.ed !== ed || gen !== this.generation) return 'stale';
      if (!ed.can_reload() || s.saving) return 'unsaved';
      ed.reload(page.text);
      s.revision = page.revision;
      this.noteEditing(page.editing);
      s.reloads += 1;
      s.behind = false;
      return 'reloaded';
    } catch {
      return 'failed';
    }
  }

  /**
   * The view's Edit here: take the canvas's editing back for this view. Returns 'editing'
   * (saving resumes; a change saved elsewhere meanwhile is a conflict on the next save, never
   * overwritten), 'refused' with nothing changed, or undefined when there is nothing to do.
   */
  async claim() {
    const s = this.state;
    if (!this.ed || this.disposed || !s.readonly || s.unidentified || s.claiming
      || !this.deps.claim) {
      return undefined;
    }
    s.claiming = true;
    this.lastClaim = null;
    try {
      const r = await this.deps.claim();
      if (this.disposed) return undefined;
      // Its note (taken, and what of its bookkeeping didn't happen) or its refusal's own reason
      // and status, as the server said them (R185 CLAIM contract).
      this.lastClaim = r || null;
      if (r && r.ok) {
        this.noteEditing(true);
        // What it had handed over left the record with the claim (the server's): it saves
        // that change itself now, and nobody waits for it.
        if (s.handover) {
          s.handover = {
            offers: (s.handover.offers || []).filter((o) => !o.from_here),
            pending: (s.handover.pending || []).filter((p) => !p.from_here),
          };
        }
        return 'editing';
      }
      return 'refused';
    } catch {
      return 'refused';
    } finally {
      s.claiming = false;
    }
  }

  /**
   * The person chose "Reload (discard my unsaved change)": load the page as the server has it
   * now, in place (no navigation, so no leave-page guard), and resume saving and polling.
   * Returns 'discarded', or undefined when there is nothing to do or the answer is stale.
   */
  async discard() {
    const s = this.state;
    const ed = this.ed;
    if (!ed || s.switching || s.error || s.saving || s.discarding) return undefined;
    const gen = ++this.generation;
    // No input, toolbar action or save reaches the editor until this is done.
    s.discarding = true;
    let done;
    this.discardDone = new Promise((r) => { done = r; });
    try {
      const page = await this.deps.fetchPage();
      // Apply only to the editor that asked, and only if nothing else started meanwhile.
      if (this.ed !== ed || gen !== this.generation || s.switching || s.error || s.saving) {
        return undefined;
      }
      ed.reload(page.text);
      s.revision = page.revision;
      this.noteEditing(page.editing);
      s.conflict = false;
      s.behind = false;
      s.reloads += 1;
      return 'discarded';
    } finally {
      s.discarding = false;
      done();
    }
  }

  /** Save the editor's pending change, if any. Returns what happened, for the status line. */
  async save() {
    if (!this.editable()) return undefined;
    return this.saveNow();
  }

  /** `save`, also while leaving (`drain`): never after `dispose`, an error or before Ready. */
  async saveNow() {
    const s = this.state;
    if (!this.ed || this.disposed || !s.ready || s.switching || s.error || s.discarding
      || s.saving || s.conflict || s.readonly || s.claiming) {
      return undefined;
    }
    const ed = this.ed;
    const snapshot = ed.save_snapshot();
    if (snapshot === undefined) return undefined;
    s.saving = true;
    this.generation += 1;
    let done;
    this.saveDone = new Promise((r) => { done = r; });
    try {
      const res = await this.deps.put(snapshot, s.revision);
      if (this.ed !== ed) return 'stale';
      if (res.ok) {
        s.saves += 1;
        ed.save_succeeded();
        s.revision = res.revision;
        return 'saved';
      }
      ed.save_failed();
      if (res.status === 409) {
        s.conflict = true;
        return 'conflict';
      }
      if (res.status === 423) {
        // Another view has the editing: nothing was written, and the change stays here.
        s.readonly = true;
        return 'readonly';
      }
      if (res.status === 428 || res.status === 410) {
        // The server doesn't know this view (released, never issued, or not named): nothing
        // was written, the change stays here, and this view can't edit (R78-01).
        s.readonly = true;
        s.unidentified = res.status === 428 ? 'missing' : 'released';
        return 'unidentified';
      }
      return 'failed';
    } catch {
      if (this.ed === ed) ed.save_failed();
      return 'unreachable';
    } finally {
      s.saving = false;
      done();
    }
  }

  /**
   * Leaving the editor (Back, Close, the view going away): stop input and polling, wait for a
   * save or discard under way, then save any change that is left. Returns 'clean' only when
   * nothing unsaved remains and no gesture or text edit is under way (`can_reload`); a missing
   * snapshot alone doesn't mean clean. Otherwise the reason: 'conflict', 'failed',
   * 'unreachable' (no answer: it may or may not have been written), 'busy' (an edit still
   * under way), 'pending' (a change applied here whose unfinished edits aren't open here yet:
   * this view holds their only copy, R80-02), 'uncertain' (an Apply got no answer, so it may
   * or may not have been saved, and this view holds the only copy of its unfinished edits now,
   * run 191), 'error' (the renderer stopped with unsaved
   * changes) or 'stale'. An offer action already started (an Apply, its load and take-up) is
   * waited for first, so its outcome is known; nothing new starts while leaving. Input stays
   * stopped until `resume`.
   */
  async drain() {
    const s = this.state;
    this.closing = true;
    // A poll or discard answer that arrives now no longer applies.
    this.generation += 1;
    await this.actionsEnded();
    await this.saveDone;
    await this.discardDone;
    // Edits are only possible once Ready: an editor that never got there has none. One that
    // was Ready and then stopped (an error clears `ready`) is judged by what it still holds.
    if (this.disposed || !this.ed || !this.wasReady) return 'clean';
    // A kept continuation counts as unfinished work here, and so do an unanswered Apply's
    // kept edits (run 191: 'uncertain'), in a read-only view too: they are never handed on
    // (R81-02), and no poll settles an unanswered Apply (R81-01). Only their dismissal, or
    // leaving without saving, lets them go.
    const left = () => {
      const unknown = this.uncertainPending().length > 0;
      if (!unknown && this.applied.size === 0) return 'clean';
      return unknown ? 'uncertain' : 'pending';
    };
    for (let i = 0; i < 2; i++) {
      // Another view has the editing: an unsaved change can't be saved from here; it is
      // handed over (if it wasn't yet; a conflict's too), and still kept here.
      if (s.readonly) {
        this.settled = true;
        await this.handOver();
        if (this.disposed || !this.ed) return 'stale';
        if (this.ed.can_reload()) return left();
        return s.conflict ? 'conflict' : 'readonly';
      }
      if (s.conflict) return 'conflict';
      if (s.error || !s.ready) {
        if (!this.ed.can_reload()) return 'error';
        return left();
      }
      const r = await this.saveNow();
      if (this.disposed || !this.ed) return 'stale';
      if (r === undefined) break;
      if (r !== 'saved') return r;
    }
    if (!this.ed.can_reload()) return 'busy';
    return left();
  }

  /** After a `drain` that didn't come out clean: editing (and polling) go on. */
  resume() {
    if (!this.disposed) this.closing = false;
  }

  /**
   * End this session for good: no input, frame, poll, save or answer reaches an editor any
   * more, and the editor and its resize observer are released, once. An editor still booting
   * is freed as soon as it arrives (`open`). Unsaved changes are not saved by this: `drain`
   * first.
   */
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.closing = true;
    this.generation += 1;
    this.state.ready = false;
    if (this.observer) { this.observer.disconnect(); this.observer = null; }
    const ed = this.ed;
    this.ed = null;
    if (ed) ed.free();
  }
}
