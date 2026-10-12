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
