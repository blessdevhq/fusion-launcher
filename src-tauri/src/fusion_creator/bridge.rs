//! Application-owned Creator service; closing a webview does not own/cancel jobs.
use super::build::ManagedDirectoryBuild;
use super::export::ManagedReleaseExport;
use super::jobs::{JobRunner, ProgressSink};
use super::verify::ManagedReleaseVerify;
use super::verify_https::ManagedReleaseVerifyHttps;
use super::CreatorJobKind;
use crate::fusion_source::errors::FusionError;
use crate::storage::RepositoryStore;
use crate::AppState;
use std::path::Path;
use std::sync::{Arc, Mutex};
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
            runner: production_runner(state.store.clone(), &state.data_dir.join("creator"), sink)?,
        })
    }
}

pub(in crate::fusion_creator) fn production_runner(
    store: Arc<Mutex<RepositoryStore>>,
    root: &Path,
    sink: ProgressSink,
) -> Result<JobRunner, FusionError> {
    let runner = JobRunner::new(store, root, sink)?;
    runner.register(CreatorJobKind::Build, Arc::new(ManagedDirectoryBuild))?;
    runner.register(CreatorJobKind::Verify, Arc::new(ManagedReleaseVerify))?;
    runner.register(CreatorJobKind::Export, Arc::new(ManagedReleaseExport))?;
    runner.register(
        CreatorJobKind::VerifyHttps,
        Arc::new(ManagedReleaseVerifyHttps),
    )?;
    Ok(runner)
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;

#[cfg(test)]
#[path = "export_https_tests.rs"]
mod export_https_tests;

#[cfg(all(test, windows))]
#[path = "universal_portable_tests.rs"]
mod universal_portable_tests;
