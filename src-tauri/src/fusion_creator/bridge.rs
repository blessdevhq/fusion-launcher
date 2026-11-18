//! Application-owned Creator service; closing a webview does not own/cancel jobs.
use super::jobs::JobRunner;
use crate::fusion_source::errors::FusionError;
use crate::AppState;
use std::sync::Arc;
use tauri::Emitter;

#[derive(Clone)]
pub struct CreatorBridge {
    pub(crate) runner: JobRunner,
}

impl CreatorBridge {
    pub(crate) fn new(app: &tauri::AppHandle, state: &AppState) -> Result<Self, FusionError> {
        let app = app.clone();
        let sink = Arc::new(move |event| {
            // Delivery is best-effort; the committed SQLite state is authoritative.
            let _ = app.emit(super::JOB_PROGRESS_EVENT, event);
        });
        Ok(Self {
            runner: JobRunner::new(state.store.clone(), &state.data_dir.join("creator"), sink)?,
        })
    }
}
