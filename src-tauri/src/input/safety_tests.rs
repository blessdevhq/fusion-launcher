use super::*;
use crate::fusion_creator::input::*;
use crate::fusion_source::creator_schema::LogicalBinding;
use crate::fusion_source::fs_safety;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

#[test]
fn failed_worker_start_blocks_launch_until_service_restart() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let owner = InputWorker::start().expect("real SDL owner must initialize");
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let startup_error = service
        .devices()
        .expect_err("SDL ownership must reject startup");
    assert_eq!(
        startup_error.code,
        crate::fusion_source::errors::ErrorCode::ActionUnavailable
    );
    assert!(startup_error
        .message
        .contains("SDL input worker already owned"));
    let mut launched_while_owned = false;
    let while_owned = service.with_released_for_launch(|| {
        launched_while_owned = true;
        Ok(())
    });

    let proof = owner
        .stop()
        .expect("real SDL owner must acknowledge release");
    assert!(proof.handles_closed);
    assert_eq!(proof.sdl_initialized_flags, 0);
    let mut launched_after_stop = false;
    let after_stop = service.with_released_for_launch(|| {
        launched_after_stop = true;
        Ok(())
    });
    println!("startup diagnostic: {startup_error}; owner release: {proof:?}");
    assert!(
        !launched_while_owned,
        "failed SDL startup must not run launch callback while another worker owns SDL"
    );
    assert!(
        !launched_after_stop,
        "failed SDL startup must stay fail-closed after the other worker stops"
    );
    assert_eq!(while_owned.unwrap_err(), startup_error);
    assert_eq!(after_stop.unwrap_err(), startup_error);
    assert_eq!(service.devices().unwrap_err(), startup_error);

    drop(service);
    let restarted = service::InputService::new(dir.path().into(), |_| {});
    restarted
        .devices()
        .expect("a restarted service may initialize SDL");
    restarted.with_released_for_launch(|| Ok(())).unwrap();
}

#[test]
fn failed_start_tears_down_open_handles_before_retry() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let id = service.fixture_attach().unwrap();
    service.fixture_fail_read().unwrap();
    assert!(service
        .start(InputSessionStartRequest { device_id: id })
        .is_err());
    assert!(
        !WORKER_OWNED.load(Ordering::Acquire),
        "failed open/read must close the worker"
    );
    let fresh = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: fresh })
        .unwrap();
    service
        .stop(InputSessionRequest {
            session_id: receipt.session_id,
        })
        .unwrap();
}

#[test]
fn tampered_local_identity_or_raw_sources_are_not_authority_for_d() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let id = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: id })
        .unwrap();
    service
        .save_bindings(InputBindingsSaveRequest {
            session_id: receipt.session_id.clone(),
            controls_profile_id: "pad".into(),
            bindings: BTreeMap::from([(
                "jump".into(),
                LogicalBinding::ControllerButton { button: "a".into() },
            )]),
            raw_bindings: BTreeMap::new(),
            deadzones: BTreeMap::new(),
        })
        .unwrap();
    let path = dir
        .path()
        .join(format!("{}.json", blake3::hash(b"pad").to_hex()));
    let original = service.private_binding_view("pad").unwrap();
    let mut changed = original.clone();
    changed.identity.port = Some(999);
    fs_safety::atomic_write_json(&path, &changed).unwrap();
    assert!(
        service.private_binding_view("pad").is_err(),
        "disk identity is not live selection"
    );
    changed = original;
    changed
        .raw_bindings
        .insert("jump".into(), RawInputBinding::Button { index: 1 });
    fs_safety::atomic_write_json(&path, &changed).unwrap();
    assert!(
        service.private_binding_view("pad").is_err(),
        "disk raw inputs must agree with SDL"
    );
    service
        .stop(InputSessionRequest {
            session_id: receipt.session_id,
        })
        .unwrap();
}

#[test]
fn sequence_exhaustion_still_closes_handles_and_blocks_launch() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let id = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: id })
        .unwrap();
    service.fixture_sequence_exhaustion();
    assert!(service
        .stop(InputSessionRequest {
            session_id: receipt.session_id
        })
        .is_err());
    assert!(
        !WORKER_OWNED.load(Ordering::Acquire),
        "overflow must not orphan SDL handles"
    );
    assert!(service.with_released_for_launch(|| Ok(())).is_err());
}
