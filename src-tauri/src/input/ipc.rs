//! IPC adapter. Stores a channel client + value store, never SDL handles in State.
use super::service::InputService;
use crate::fusion_creator::input::INPUT_STATE_EVENT;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::Emitter;

static SERVICE: OnceLock<InputService> = OnceLock::new();

pub fn initialize(app: tauri::AppHandle, local_root: PathBuf) -> Result<(), FusionError> {
    SERVICE
        .set(InputService::new(local_root, move |state| {
            if let Err(error) = app.emit(INPUT_STATE_EVENT, state) {
                tracing::warn!(
                    "Input event delivery failed; session_get remains authoritative: {error}"
                );
            }
        }))
        .map_err(|_| {
            FusionError::new(
                ErrorCode::OperationInProgress,
                "Input service already initialized.",
            )
        })
}

/// Internal host access for D/B; no physical-identity IPC getter is registered.
pub fn service() -> Result<&'static InputService, FusionError> {
    SERVICE.get().ok_or_else(|| {
        FusionError::new(
            ErrorCode::ActionUnavailable,
            "Native input service is not initialized.",
        )
    })
}

/// Enumeration/open/read/save/ACK can block. Never run them on Tauri's UI thread.
pub async fn run<T: Send + 'static>(
    command: &'static str,
    action: impl FnOnce(&InputService) -> Result<T, FusionError> + Send + 'static,
) -> Result<T, FusionError> {
    let service = service().map_err(|error| error.with_detail("command", command))?;
    tauri::async_runtime::spawn_blocking(move || action(service))
        .await
        .map_err(|error| {
            FusionError::new(
                ErrorCode::ActionUnavailable,
                format!("Input task failed: {error}"),
            )
            .with_detail("command", command)
        })?
        .map_err(|error| error.with_detail("command", command))
}
