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

export class Session {
  /**
   * deps: {
   *   boot(canvas, page, webgl) -> Promise<editor>   page = { text, revision }
   *   fetchPage() -> Promise<page>                    the page as the server has it now
   *   replaceCanvas(old) -> canvas                    a fresh canvas in place of `old`
   *   observeResize(canvas, onResize) -> { disconnect() }
   *   put(snapshot, revision) -> Promise<{ ok, status, revision }>
   *   log(message)                                    console.error
   * }
   */
  constructor(deps, state = {}) {
    this.deps = deps;
    this.state = Object.assign(state, {
      ready: false, error: null, fallback: null, backend: null, revision: null,
      saves: 0, saving: false, conflict: false, switching: false,
    });
    this.ed = null;
    this.canvas = null;
    this.observer = null;
    this.forceWebgl = false;
    this.saveDone = Promise.resolve();
  }

  /** May the user edit right now? */
  editable() {
    const s = this.state;
    return Boolean(this.ed) && s.ready && !s.switching && !s.error;
  }

  /** Run `fn(editor)` only when editing is allowed (input, toolbar). */
  edit(fn) {
    if (!this.editable()) return undefined;
    return fn(this.ed);
  }

  /** Run `fn(editor)` when there is an editor that isn't being replaced (resize, frames). */
  withEditor(fn) {
    if (!this.ed || this.state.switching) return undefined;
    return fn(this.ed);
  }

  async start(canvas, page, forceWebgl) {
    this.canvas = canvas;
    this.forceWebgl = forceWebgl;
    this.state.revision = page.revision;
    try {
      await this.open(page, forceWebgl);
    } catch (e) {
      if (forceWebgl) throw e;
      await this.fallBack(e && e.message ? e.message : String(e));
    }
  }

  async open(page, webgl) {
    this.ed = await this.deps.boot(this.canvas, page, webgl);
    this.state.revision = page.revision;
    this.state.backend = this.ed.backend();
    this.observer = this.deps.observeResize(this.canvas, () => this.resized());
  }

  resized() {
    this.withEditor((ed) => this.deps.resize && this.deps.resize(ed, this.canvas));
  }

  /** WebGPU failed before Ready: reopen with WebGL2 on a fresh canvas, visibly. */
  async fallBack(reason) {
    const s = this.state;
    s.switching = true;
    try {
      this.deps.log(`WebGPU failed, falling back to WebGL: ${reason}`);
      s.fallback = reason;
      await this.saveDone;
      const page = await this.deps.fetchPage();
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
   * when a GPU problem ends editing, 'fallback' when a fallback was started, else undefined.
   */
  frame(t) {
    const s = this.state;
    if (s.switching || s.error || !this.ed) return undefined;
    this.ed.frame(t);
    const gpuError = this.ed.gpu_error();
    if (gpuError) {
      if (!s.ready && !this.forceWebgl && !s.fallback) {
        this.fallBack(gpuError).catch((e) => this.fail(`Renderer failed: ${e && e.message ? e.message : e}`));
        return 'fallback';
      }
      this.fail(`Renderer error: ${gpuError}`);
      return 'error';
    }
    if (!s.ready && this.ed.ready()) {
      s.ready = true;
      return 'ready';
    }
    return undefined;
  }

  fail(msg) {
    this.state.ready = false;
    this.state.error = msg;
    this.deps.log(msg);
  }

  /** Save the editor's pending change, if any. Returns what happened, for the status line. */
  async save() {
    const s = this.state;
    if (!this.editable() || s.saving || s.conflict) return undefined;
    const ed = this.ed;
    const snapshot = ed.save_snapshot();
    if (snapshot === undefined) return undefined;
    s.saving = true;
    let done;
    this.saveDone = new Promise((r) => { done = r; });
    try {
      const res = await this.deps.put(snapshot, s.revision);
      if (this.ed !== ed) return 'stale';
      if (res.ok) {
        s.revision = res.revision;
        s.saves += 1;
        ed.save_succeeded();
        return 'saved';
      }
      ed.save_failed();
      if (res.status === 409) {
        s.conflict = true;
        return 'conflict';
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
}
