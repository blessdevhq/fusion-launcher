//! Typed Creator/input IPC. Unimplemented host operations remain explicitly unavailable.
use crate::fusion_creator::bridge::CreatorBridge;
use crate::fusion_creator::input::*;
use crate::fusion_creator::project_service;
use crate::fusion_creator::*;
use crate::fusion_source::errors::{ErrorCode, FusionError};

fn unavailable(command: &str) -> FusionError {
    FusionError::new(
        ErrorCode::ActionUnavailable,
        "This action is unavailable in Creator wave zero.",
    )
    .with_detail("command", command)
}

macro_rules! unavailable_command {
    ($name:ident () -> $response:ty) => {
        #[tauri::command]
        pub async fn $name() -> Result<$response, FusionError> {
            Err(unavailable(stringify!($name)))
        }
    };
    ($name:ident ($request:ty) -> $response:ty) => {
        #[tauri::command]
        pub async fn $name(request: $request) -> Result<$response, FusionError> {
            let _ = request;
            Err(unavailable(stringify!($name)))
        }
    };
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, FusionError> + Send + 'static,
) -> Result<T, FusionError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| crate::fusion_creator::jobs::io())?
}

macro_rules! creator_command {
    ($name:ident () -> $response:ty, |$runner:ident| $body:block) => {
        #[tauri::command]
        pub async fn $name(bridge: tauri::State<'_, CreatorBridge>) -> Result<$response, FusionError> {
            let $runner = bridge.runner.clone();
            blocking(move || $body).await
        }
    };
    ($name:ident ($request:ty) -> $response:ty, |$runner:ident, $value:ident| $body:block) => {
        #[tauri::command]
        pub async fn $name(request: $request, bridge: tauri::State<'_, CreatorBridge>) -> Result<$response, FusionError> {
            let $runner = bridge.runner.clone();
            let $value = request;
            blocking(move || $body).await
        }
    };
}

creator_command!(fusion_creator_project_list() -> Vec<CreatorProjectSummary>, |runner| {
    let shared = runner.store();
    let store = shared.lock().map_err(|_| crate::fusion_creator::jobs::io())?;
    project_service::list(&store)
});
creator_command!(fusion_creator_project_open(CreatorProjectRequest) -> CreatorProjectOpenResponse, |runner, request| {
    let shared = runner.store();
    let store = shared.lock().map_err(|_| crate::fusion_creator::jobs::io())?;
    project_service::open(&store, &request.project_id)
});
creator_command!(fusion_creator_project_save(CreatorProjectSaveRequest) -> CreatorProjectSaveResponse, |runner, request| {
    let shared = runner.store();
    let mut store = shared.lock().map_err(|_| crate::fusion_creator::jobs::io())?;
    project_service::save(&mut store, request)
});
unavailable_command!(fusion_creator_scan(CreatorJobStartRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_classify(CreatorJobStartRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_build(CreatorJobStartRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_verify(CreatorJobStartRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_export(CreatorJobStartRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_verify_https(CreatorJobStartRequest) -> CreatorJobReceipt);
creator_command!(fusion_creator_job_get(CreatorJobRequest) -> CreatorJob, |runner, request| {
    runner.get(&request.job_id)
});
creator_command!(fusion_creator_job_cancel(CreatorJobRequest) -> CreatorJob, |runner, request| {
    runner.cancel(&request.job_id)
});
unavailable_command!(fusion_creator_job_resume(CreatorJobRequest) -> CreatorJobReceipt);
unavailable_command!(fusion_creator_release_list(CreatorProjectRequest) -> Vec<CreatorRelease>);
unavailable_command!(fusion_creator_report_get(CreatorReportRequest) -> CreatorReport);
unavailable_command!(fusion_input_devices() -> Vec<InputDevice>);
unavailable_command!(fusion_input_session_start(InputSessionStartRequest) -> InputSessionReceipt);
unavailable_command!(fusion_input_session_stop(InputSessionRequest) -> InputReleaseReceipt);
unavailable_command!(fusion_input_session_get(InputSessionRequest) -> InputState);
unavailable_command!(fusion_input_bindings_save(InputBindingsSaveRequest) -> InputBindingsReceipt);
unavailable_command!(fusion_input_release_for_launch(InputSessionRequest) -> InputReleaseReceipt);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion_source::errors::ErrorCode;

    #[tokio::test]
    async fn unimplemented_commands_never_report_mock_success() {
        let error = fusion_input_devices().await.unwrap_err();
        assert_eq!(error.code, ErrorCode::ActionUnavailable);
        assert_eq!(error.details["command"], "fusion_input_devices");
        macro_rules! rejects {
            ($command:ident, $request:expr) => {
                let error = $command($request).await.unwrap_err();
                assert_eq!(error.code, ErrorCode::ActionUnavailable);
                assert_eq!(error.details["command"], stringify!($command));
            };
        }
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
        let project = CreatorProjectRequest {
            project_id: "fixture-project".into(),
        };

        let start: CreatorJobStartRequest =
            serde_json::from_value(fixture["jobStart"].clone()).unwrap();
        rejects!(fusion_creator_scan, start.clone());
        rejects!(fusion_creator_classify, start.clone());
        rejects!(fusion_creator_build, start.clone());
        rejects!(fusion_creator_verify, start.clone());
        rejects!(fusion_creator_export, start.clone());
        rejects!(fusion_creator_verify_https, start);
        let job = CreatorJobRequest {
            job_id: "job-1".into(),
        };
        rejects!(fusion_creator_job_resume, job);
        rejects!(fusion_creator_release_list, project);
        rejects!(
            fusion_creator_report_get,
            CreatorReportRequest {
                report_id: "report-1".into()
            }
        );
        rejects!(
            fusion_input_session_start,
            InputSessionStartRequest {
                device_id: "device-1".into()
            }
        );
        let session = InputSessionRequest {
            session_id: "session-1".into(),
        };
        rejects!(fusion_input_session_stop, session.clone());
        rejects!(fusion_input_session_get, session.clone());
        rejects!(
            fusion_input_bindings_save,
            InputBindingsSaveRequest {
                session_id: session.session_id.clone(),
                controls_profile_id: "keyboard".into(),
                bindings: Default::default(),
                deadzones: Default::default(),
                raw_bindings: Default::default(),
            }
        );
        rejects!(fusion_input_release_for_launch, session);
    }
}
