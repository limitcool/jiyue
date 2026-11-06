//! Moves results of background threads into the app on the UI thread, and starts and stops the
//! vector index runs. Nothing here blocks: threads send messages, and a `cx.spawn` loop drains
//! them every few tens of milliseconds.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use gpui::Context;
use jiyue_ai::CancelToken;

use super::{effective_config, indexer, snapshot_notes, worker};
use crate::RustdianApp;
use crate::ui::ext::ToastKind;

/// How often the UI thread looks at the result channels.
const POLL: Duration = Duration::from_millis(40);

/// Drains `rx` on the UI thread. `apply` returns false when the job is finished, which ends the
/// loop. The loop also ends when the sending side is gone.
pub fn pump<M: Send + 'static>(
    cx: &mut Context<RustdianApp>,
    rx: Receiver<M>,
    mut apply: impl FnMut(&mut RustdianApp, M, &mut Context<RustdianApp>) -> bool + 'static,
) {
    let task = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let keep = this.update(cx, |app, cx| {
                let mut keep = true;
                loop {
                    match rx.try_recv() {
                        Ok(msg) => {
                            if !apply(app, msg, cx) {
                                keep = false;
                                break;
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            keep = false;
                            break;
                        }
                    }
                }
                cx.notify();
                keep
            });
            if !matches!(keep, Ok(true)) {
                break;
            }
        }
    });
    task.detach();
}

/// Runs `work` on a thread and applies its single result on the UI thread.
pub fn once<T: Send + 'static>(
    cx: &mut Context<RustdianApp>,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut RustdianApp, T, &mut Context<RustdianApp>) + 'static,
) {
    let rx = worker::spawn(work);
    let mut done = Some(done);
    pump(cx, rx, move |app, value, cx| {
        if let Some(f) = done.take() {
            f(app, value, cx);
        }
        false
    });
}

/// Starts an index run. `only` lists the notes to refresh; `None` means every note, and also
/// drops indexed notes that no longer exist. A full run replaces one that is still going.
pub fn start_index(
    app: &mut RustdianApp,
    cx: &mut Context<RustdianApp>,
    only: Option<Vec<String>>,
) {
    let cfg = effective_config(&app.ext.ai);
    let Some(entry) = cfg.embedding_provider().cloned() else {
        return;
    };
    if only.is_some() && app.ext.ai.index.running.is_some() {
        return;
    }
    if let Some(prev) = app.ext.ai.index.running.take() {
        prev.cancel();
    }
    let reconcile = only.is_none();
    let notes: Vec<_> = snapshot_notes(app, &cfg)
        .into_iter()
        .filter(|n| match &only {
            Some(paths) => paths.contains(&n.path),
            None => true,
        })
        .collect();
    let key = match worker::api_key(&entry.id, app.ext.ai.test.as_ref()) {
        Ok(k) => k,
        Err(e) => {
            app.ext.ai.index.error = Some(super::errors::describe(&e));
            return;
        }
    };
    app.ext.ai.index.generation += 1;
    let generation = app.ext.ai.index.generation;
    let cancel = CancelToken::new();
    app.ext.ai.index.running = Some(cancel.clone());
    app.ext.ai.index.done = 0;
    app.ext.ai.index.total = notes.len();
    app.ext.ai.index.error = None;

    let job = indexer::IndexJob {
        dir: app.vault.root_dir.join(".jiyue").join("index"),
        entry,
        key,
        notes,
        reconcile,
        cancel,
    };
    let (tx, rx) = channel();
    std::thread::spawn(move || indexer::run(job, &tx));
    pump(cx, rx, move |app, msg, _cx| {
        if app.ext.ai.index.generation != generation {
            return false;
        }
        match msg {
            indexer::IndexMsg::Progress { done, total } => {
                app.ext.ai.index.done = done;
                app.ext.ai.index.total = total;
                true
            }
            indexer::IndexMsg::Finished(result) => {
                app.ext.ai.index.running = None;
                match result {
                    Ok(stats) => {
                        app.ext.ai.index.stats = Some(stats);
                        app.ext.ai.index.error = None;
                    }
                    Err(indexer::IndexFailure::Paused) => {}
                    Err(indexer::IndexFailure::Error(m)) => {
                        app.toast(ToastKind::Error, format!("AI 索引失败：{m}"));
                        app.ext.ai.index.error = Some(m);
                    }
                }
                false
            }
        }
    });
    cx.notify();
}

/// Stops the index run in progress (AI turned off, or the vault changed).
pub fn stop_index(app: &mut RustdianApp) {
    if let Some(cancel) = app.ext.ai.index.running.take() {
        cancel.cancel();
    }
    app.ext.ai.index.generation += 1;
    app.ext.ai.index.queue = indexer::DebouncedQueue::new(indexer::REEMBED_DELAY);
}

/// Deletes the stored index and builds it again from every note.
pub fn rebuild_index(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    stop_index(app);
    let file = app
        .vault
        .root_dir
        .join(".jiyue")
        .join("index")
        .join("index.bin");
    let _ = std::fs::remove_file(file);
    app.ext.ai.index.stats = None;
    start_index(app, cx, None);
}

#[cfg(test)]
mod tests {
    #[test]
    fn progress_label_is_used_by_the_status_bar() {
        assert_eq!(super::super::indexer::progress_label(1, 2), "AI 索引 1/2");
    }
}
