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
  if (st.error) return r;
  if (r === 'saved') ui.status('Saved');
  else if (r === 'conflict') ui.status('Not saved: conflict');
  else if (r === 'failed') ui.status('Save failed; retrying');
  else if (r === 'unreachable') ui.status('Save failed (server unreachable); retrying');
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
