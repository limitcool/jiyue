//! Background jobs for the UI: run blocking work off the UI thread, then apply its result to the
//! app on the UI thread. Used by the wave-2 modules (sync, search indexing, AI requests).

use gpui::Context;

use crate::RustdianApp;

/// Runs `work` on gpui's background executor and then calls `done` on the UI thread with its
/// result. `done` is skipped when the app entity has been dropped. The task is detached, so
/// the caller does not keep it alive.
pub fn spawn_blocking<T: Send + 'static>(
    cx: &mut Context<RustdianApp>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut RustdianApp, T, &mut Context<RustdianApp>) + 'static,
) {
    cx.spawn(async move |this, cx| {
        let value = cx.background_executor().spawn(async move { work() }).await;
        let _ = this.update(cx, move |app, cx| done(app, value, cx));
    })
    .detach();
}
