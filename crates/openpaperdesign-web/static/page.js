// What the page shows for each animation frame of the session (see session.js). Kept apart
// from app.js (which needs the DOM and the wasm module) so it is tested in plain Node.

/**
 * Run one frame and update the page. `ui` = { ready(), fatal(message), status(text), tick() }.
 * Returns whether the frame loop should keep running: a terminal error stops it, after being
 * shown once in red (status line and footer).
 */
export function frameStep(session, ui, t) {
  const r = session.frame(t);
  if (r === 'error') {
    ui.fatal(session.state.error);
    return false;
  }
  if (r === 'stopped') return false;
  if (r === 'ready') ui.ready();
  else if (r === 'fallback') ui.status('WebGPU failed; switching to WebGL…');
  ui.tick();
  return true;
}

/**
 * One save tick. `ui` also needs `conflict(ids)` (shows the conflict banner). The save's
 * bookkeeping (ack, revision, conflict state) always happens in `session.save()`. Once the
 * session has hit a terminal error, the status line and footer keep showing it: a save that
 * was in flight when the error happened doesn't replace it with "Saved" or "retrying".
 */
export async function saveStep(session, ui) {
  const st = session.state;
  if (session.editable() && !st.saving && !st.conflict && session.ed.dirty()) {
    ui.status('Saving…');
  }
  const r = await session.save();
  if (r === 'conflict') ui.conflict();
  if ((r === 'readonly' || r === 'unidentified') && ui.readonly) ui.readonly();
  if (st.error) return r;
  if (r === 'saved') ui.status('Saved');
  else if (r === 'conflict') ui.status('Not saved: conflict');
  else if (r === 'readonly') ui.status(OPD_READONLY_STATUS);
  else if (r === 'unidentified') ui.status(OPD_UNIDENTIFIED_STATUS);
  else if (r === 'failed') ui.status('Save failed; retrying');
  else if (r === 'unreachable') ui.status('Save failed (server unreachable); retrying');
  return r;
}

/** What a view that lost the canvas's editing says (issue #23; see session.js). */
export const OPD_READONLY_STATUS = 'Read-only: another view is editing this canvas';
/** What a view the canvas's editor no longer knows says (released, never issued; R78-01). */
export const OPD_UNIDENTIFIED_STATUS = 'Read-only: this view is no longer known to the canvas';
/** What it says once it has the editing again (Edit here). */
export const OPD_EDITING_STATUS = 'Editing here again';

/**
 * The read-only banner's "Edit here": take the canvas's editing back for this view. `ui` also
 * needs `hideReadonly()`. A refusal changes nothing; the view stays read-only and says why.
 */
export async function claimStep(session, ui) {
  const r = await session.claim();
  if (r === 'editing') {
    if (ui.hideReadonly) ui.hideReadonly();
    // A conflict this view was in stays one: its banner (and its explicit reload) comes back.
    if (session.state.conflict && ui.conflict) ui.conflict();
    else if (session.state.handover && ui.handover) ui.handover();
    const note = session.lastClaim && session.lastClaim.note;
    if (!session.state.error) ui.status(note ? `${OPD_EDITING_STATUS}. ${note}` : OPD_EDITING_STATUS);
  } else if (r === 'refused' && !session.state.error) {
    // The server's own reason (busy, closed, released, fenced…), not a guess.
    const why = session.lastClaim && session.lastClaim.error;
    ui.status(why && !/^claim \d+$/.test(why)
      ? `Editing wasn't taken here: ${why}` : 'Editing couldn\'t be taken back here; try again');
  }
  return r;
}

/**
 * The handover banner's actions, in the editing view (issue #23; see session.js): `action` is
 * 'apply', 'resume' (Continue here), 'keep' (Keep as a new canvas), 'dismiss' or 'stop' (stop
 * waiting for a view), on offer or pending view `id`. `ui` also needs `handover()` (shows the
 * banner for the record as it is now). Returns the session's answer.
 */
export async function offerStep(session, ui, action, id) {
  const run = {
    apply: () => session.applyOffer(id),
    resume: () => session.resumeOffer(id),
    keep: () => session.keepOffer(id),
    dismiss: () => session.dismissOffer(id),
    stop: () => session.stopWaiting(id),
  }[action];
  const r = run ? await run() : undefined;
  if (r === undefined) return r;
  const say = (t) => { if (!session.state.error) ui.status(t); };
  const kept = session.lastBody && session.lastBody.kept;
  const said = {
    applied: 'Applied the change handed over from another view',
    'applied-left': 'Applied the change handed over (it is still listed: dismiss it)',
    'applied-partial': 'Applied the change handed over (saved); part of its unfinished edit is open here, the rest is kept to continue',
    'applied-unopened': 'Applied the change handed over (saved); its unfinished edit isn\'t open here yet and is kept: continue it here when you can',
    unknown: 'Codex didn\'t answer, so it isn\'t known whether that was done',
    unsupported: 'Not applied: it carries edits kept by a newer editor that this one can\'t hold to their guards; it stays listed',
    resumed: 'Its unfinished edit is open here: finish it (Enter) or cancel it (Escape)',
    partial: 'Part of its unfinished edit is open here; the rest couldn\'t be opened here',
    untaken: 'Its unfinished edit wasn\'t opened here; it is still listed',
    kept: `Kept it as a new canvas${kept && kept.name ? `, "${kept.name}"` : ''}; open it from the library`,
    dismissed: 'Dismissed the change handed over',
    stopped: 'Stopped waiting for that view; what it has stays there',
    gone: 'That is no longer waiting',
    conflict: 'Not applied: the canvas changed since that change was made; nothing was overwritten. Keep it as a new canvas, or dismiss it',
    unsaved: 'Your own change here is saved first; then try again',
    readonly: OPD_READONLY_STATUS,
  }[r] || 'That didn\'t work; try again';
  if (r === 'readonly' && ui.readonly) ui.readonly();
  else if (ui.handover) ui.handover();
  // An unfinished edit now open here (Continue here, or Apply then its continuation) gets the
  // keyboard at once, as the status promises (Enter, Escape): the action's button is gone.
  const opened = r === 'resumed' || r === 'partial' || r === 'applied-partial'
    || ((r === 'applied' || r === 'applied-left') && session.lastTaken > 0);
  if (opened && ui.focusEditor) ui.focusEditor();
  const why = ['partial', 'untaken', 'applied-partial', 'applied-unopened', 'unknown',
    'unsupported'].includes(r);
  say(why && session.lastRefusals && session.lastRefusals.length
    ? `${said}: ${session.lastRefusals.join('; ')}` : said);
  // A field's value opens over the editor's next frames: if its field never showed here, that
  // is said, and its row (still listed, also after an Apply: R79-04) can be continued again or
  // dismissed.
  if (opened && session.lastFieldTaken && session.fieldOutcome) {
    const why = await session.fieldOutcome(id);
    if (why) {
      if (ui.handover) ui.handover();
      say(`Its unfinished value wasn't opened here: ${why}. It is still listed: continue it again or dismiss it`);
    }
  }
  return r;
}

/**
 * The conflict banner's "Reload (discard my unsaved change)": load the server's version in
 * place. `ui` also needs `hideConflict()`. Nothing is discarded any other way.
 */
export async function discardStep(session, ui) {
  let r;
  try {
    r = await session.discard();
  } catch {
    if (!session.state.error) ui.status('Could not load the version on disk; try again');
    return undefined;
  }
  if (r === 'discarded') {
    ui.hideConflict();
    if (!session.state.error) ui.status('Reloaded the version on disk');
  }
  return r;
}
