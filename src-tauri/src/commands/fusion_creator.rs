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

// Additive input dispatch. Keep the frozen registration declarations below:
// their historical macro name is also consumed by the shared namespace test.
macro_rules! input_command {
    ($name:ident () -> $response:ty, $method:ident) => {
        #[tauri::command]
        pub async fn $name() -> Result<$response, FusionError> {
            #[cfg(windows)]
            {
                crate::input::ipc::run(stringify!($name), |service| service.$method()).await
            }
            #[cfg(not(windows))]
            {
                Err(unavailable(stringify!($name)))
            }
        }
    };
    ($name:ident ($request:ty) -> $response:ty, $method:ident) => {
        #[tauri::command]
        pub async fn $name(request: $request) -> Result<$response, FusionError> {
            #[cfg(windows)]
            {
                crate::input::ipc::run(stringify!($name), move |service| service.$method(request))
                    .await
            }
            #[cfg(not(windows))]
            {
                let _ = request;
                Err(unavailable(stringify!($name)))
            }
        }
    };
}

macro_rules! history_command {
    ($name:ident, $request:ty, $response:ty, $method:ident, $field:ident) => {
        creator_command!($name($request) -> $response, |runner, request| {
            let shared = runner.store();
            let store = shared.lock().map_err(|_| crate::fusion_creator::jobs::io())?;
            project_service::$method(&store, &request.$field)
        });
    };
}

macro_rules! unavailable_command {
    (fusion_creator_release_list ($request:ty) -> $response:ty) => {
        history_command!(fusion_creator_release_list, $request, $response, release_list, project_id);
    };
    (fusion_creator_report_get ($request:ty) -> $response:ty) => {
        history_command!(fusion_creator_report_get, $request, $response, report_get, report_id);
    };
    (fusion_input_devices () -> $response:ty) => { input_command!(fusion_input_devices() -> $response, devices); };
    (fusion_input_session_start ($request:ty) -> $response:ty) => { input_command!(fusion_input_session_start($request) -> $response, start); };
    (fusion_input_session_get ($request:ty) -> $response:ty) => { input_command!(fusion_input_session_get($request) -> $response, state); };
    (fusion_input_session_stop ($request:ty) -> $response:ty) => { input_command!(fusion_input_session_stop($request) -> $response, stop); };
    (fusion_input_bindings_save ($request:ty) -> $response:ty) => { input_command!(fusion_input_bindings_save($request) -> $response, save_bindings); };
    (fusion_input_release_for_launch ($request:ty) -> $response:ty) => { input_command!(fusion_input_release_for_launch($request) -> $response, release_for_launch); };
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
        pub async fn $name(
            bridge: tauri::State<'_, CreatorBridge>,
        ) -> Result<$response, FusionError> {
            let $runner = bridge.runner.clone();
            blocking(move || $body).await
        }
    };
    ($name:ident ($request:ty) -> $response:ty, |$runner:ident, $value:ident| $body:block) => {
        #[tauri::command]
        pub async fn $name(
            request: $request,
            bridge: tauri::State<'_, CreatorBridge>,
        ) -> Result<$response, FusionError> {
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
fn decode_private_bindings(
    body: &serde_json::Value,
) -> Result<CreatorPrivateBindingsSaveRequest, FusionError> {
    let request = body
        .as_object()
        .filter(|body| body.len() == 1)
        .and_then(|body| body.get("request"))
        .filter(|request| {
            request.is_object()
                && request
                    .get("bindings")
                    .is_some_and(serde_json::Value::is_object)
        })
        .ok_or_else(invalid_private_bindings_request)?;
    serde::Deserialize::deserialize(request).map_err(|_| invalid_private_bindings_request())
}

fn invalid_private_bindings_request() -> FusionError {
    FusionError::new(
        ErrorCode::InvalidManifest,
        "Invalid Creator private bindings request.",
    )
}

#[tauri::command]
pub async fn fusion_creator_private_bindings_save(
    ipc: tauri::ipc::Request<'_>,
    bridge: tauri::State<'_, CreatorBridge>,
) -> Result<CreatorPrivateBindingsSaveResponse, FusionError> {
    let tauri::ipc::InvokeBody::Json(body) = ipc.body() else {
        return Err(invalid_private_bindings_request());
    };
    let request = decode_private_bindings(body)?;
    let runner = bridge.runner.clone();
    blocking(move || {
        let shared = runner.store();
        let store = shared
            .lock()
            .map_err(|_| crate::fusion_creator::jobs::io())?;
        let revision = crate::fusion_creator::project_storage::save_private_bindings(
            &store,
            request.project_revision,
            request.expected_bindings_revision,
            &request.bindings,
        )?;
        Ok(CreatorPrivateBindingsSaveResponse { revision })
    })
    .await
}
unavailable_command!(fusion_creator_classify(CreatorJobStartRequest) -> CreatorJobReceipt);
creator_command!(fusion_creator_build(CreatorJobStartRequest) -> CreatorJobReceipt, |runner, request| {
    runner.start(CreatorJobKind::Build, request)
});
creator_command!(fusion_creator_verify(CreatorJobStartRequest) -> CreatorJobReceipt, |runner, request| {
    runner.start(CreatorJobKind::Verify, request)
});
creator_command!(fusion_creator_export(CreatorJobStartRequest) -> CreatorJobReceipt, |runner, request| {
    runner.start(CreatorJobKind::Export, request)
});
creator_command!(fusion_creator_verify_https(CreatorJobStartRequest) -> CreatorJobReceipt, |runner, request| {
    runner.start(CreatorJobKind::VerifyHttps, request)
});
creator_command!(fusion_creator_job_get(CreatorJobRequest) -> CreatorJob, |runner, request| {
    runner.get(&request.job_id)
});
creator_command!(fusion_creator_job_cancel(CreatorJobRequest) -> CreatorJob, |runner, request| {
    runner.cancel(&request.job_id)
});
creator_command!(fusion_creator_job_resume(CreatorJobRequest) -> CreatorJobReceipt, |runner, request| {
    runner.resume(&request.job_id)
});
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

    #[test]
    fn private_bindings_ipc_only_accepts_objects_and_redacts_bad_input() {
        use serde_json::json;
        let bindings = json!({"projectId":"project", "componentPaths":{}, "signingKeyRef":null});
        let valid =
            json!({"projectRevision":0, "expectedBindingsRevision":null, "bindings": bindings});
        assert_eq!(
            decode_private_bindings(&json!({"request": valid}))
                .unwrap()
                .project_revision,
            0
        );
        for malformed in [
            json!({"request":[0, null, bindings]}),
            json!({"request":{"projectRevision":0,"expectedBindingsRevision":null,"bindings":["project",{},null]}}),
            json!({"request":{"projectRevision":"synthetic-private-input","expectedBindingsRevision":null,"bindings":bindings}}),
            json!({"request":{"projectRevision":0,"bindings":bindings}}),
            json!({"request":valid,"synthetic-private-input":true}),
            json!({"synthetic-private-input":valid}),
        ] {
            let error = decode_private_bindings(&malformed).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidManifest);
            assert_eq!(error.message, "Invalid Creator private bindings request.");
            assert!(error.details.is_empty());
        }
    }

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
        let start: CreatorJobStartRequest =
            serde_json::from_value(fixture["jobStart"].clone()).unwrap();
        rejects!(fusion_creator_scan, start.clone());
        rejects!(fusion_creator_classify, start);
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
