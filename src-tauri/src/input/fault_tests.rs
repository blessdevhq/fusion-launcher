use super::*;
use crate::fusion_creator::input::*;
use crate::fusion_source::creator_schema::LogicalBinding;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn missing_release_ack_blocks_launch_and_remains_sticky() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let id = service.fixture_attach().unwrap();
    let session = service
        .start(InputSessionStartRequest { device_id: id })
        .unwrap();
    service.fixture_release_fault(Duration::ZERO, true).unwrap();
    assert!(service
        .release_for_launch(InputSessionRequest {
            session_id: session.session_id
        })
        .is_err());
    let called = AtomicBool::new(false);
    assert!(service
        .with_released_for_launch(|| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert!(service.devices().is_err());
}

#[test]
fn hung_release_returns_diagnostic_instead_of_blocking_join_or_launching() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let id = service.fixture_attach().unwrap();
    let session = service
        .start(InputSessionStartRequest { device_id: id })
        .unwrap();
    service
        .fixture_release_fault(Duration::from_millis(400), false)
        .unwrap();
    let timer = Instant::now();
    let error = service
        .release_for_launch(InputSessionRequest {
            session_id: session.session_id,
        })
        .unwrap_err();
    assert!(
        timer.elapsed() < Duration::from_millis(300),
        "timeout followed by unbounded join: {error}"
    );
    assert!(
        InputWorker::start().is_err(),
        "live/hung worker must keep the ownership lease"
    );
    let called = AtomicBool::new(false);
    assert!(service
        .with_released_for_launch(|| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    assert!(!called.load(Ordering::SeqCst));
    let deadline = Instant::now() + Duration::from_secs(3);
    while WORKER_OWNED.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn barrier_closes_discovery_worker_and_runs_launch_only_after_sdl_teardown() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    service.devices().unwrap();
    service
        .with_released_for_launch(|| {
            assert!(!WORKER_OWNED.load(Ordering::Acquire));
            Ok(())
        })
        .unwrap();
}

#[test]
fn identical_devices_and_changed_topology_never_reuse_private_selection() {
    let _test = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().into(), |_| {});
    let first = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: first })
        .unwrap();
    let save = InputBindingsSaveRequest {
        session_id: receipt.session_id.clone(),
        controls_profile_id: "pad".into(),
        bindings: BTreeMap::from([(
            "jump".into(),
            LogicalBinding::ControllerButton { button: "a".into() },
        )]),
        deadzones: BTreeMap::new(),
        raw_bindings: BTreeMap::new(),
    };
    service.save_bindings(save.clone()).unwrap();
    service.fixture_attach_identical().unwrap();
    assert!(service.private_binding_view("pad").is_err());
    assert!(service.save_bindings(save).is_err());
    service
        .stop(InputSessionRequest {
            session_id: receipt.session_id,
        })
        .unwrap();
}
