use super::*;
use crate::fusion_creator::input::*;
use crate::fusion_source::creator_schema::{AxisDirection, LogicalBinding};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn guid_crc_is_removed_without_reordering_bytes_and_hints_match_pin() {
    assert_eq!(
        eden::normalize_guid("0300ABCD7e0500000920000000000000").unwrap(),
        "030000007e0500000920000000000000"
    );
    assert!(eden::normalize_guid("not-a-guid").is_err());
    assert!(eden::HINTS.contains(&("SDL_GAMECONTROLLER_USE_BUTTON_LABELS", "0")));
    assert!(eden::HINTS.contains(&("SDL_JOYSTICK_HIDAPI_COMBINE_JOY_CONS", "0")));
    assert!(eden::HINTS.contains(&("SDL_JOYSTICK_RAWINPUT", "0")));
}

#[test]
fn native_service_streams_normalized_buttons_hats_axes_triggers_and_stops() {
    let _owner = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let service = service::InputService::new(dir.path().to_owned(), move |state| {
        sink.lock().unwrap().push(state);
    });
    let device = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: device })
        .unwrap();
    let request = InputSessionRequest {
        session_id: receipt.session_id.clone(),
    };
    service.fixture_button(0, true).unwrap();
    service.fixture_hat(3).unwrap();
    service.fixture_axis(0, i16::MIN).unwrap();
    service.fixture_axis(1, i16::MAX).unwrap();
    service.fixture_axis(4, i16::MAX).unwrap();
    let state = service.wait_state(&request, |s| {
        s.buttons.get("a") == Some(&true) && s.axes.get("leftx") == Some(&-1.0)
    });
    assert!(state.buttons["dpup"] && state.buttons["dpright"]);
    assert_eq!(state.axes["lefty"], 1.0);
    assert_eq!(state.axes["lefttrigger"], 1.0);
    let proof = service.release_for_launch(request.clone()).unwrap();
    assert!(proof.handles_closed);
    let stopped = service.state(request.clone()).unwrap();
    assert_eq!(stopped.status, InputSessionStatus::Stopped);
    assert!(stopped.sequence > state.sequence);
    assert!(service.stop(request).unwrap().handles_closed);
    let values = events.lock().unwrap();
    assert_eq!(values.last().unwrap().status, InputSessionStatus::Stopped);
    assert!(values.windows(2).all(|w| w[0].sequence < w[1].sequence));
    assert!(!serde_json::to_string(&*values)
        .unwrap()
        .contains("\"guid\":"));
}

#[test]
fn capture_rejects_stale_sessions_and_reconnect_requires_new_selection() {
    let _owner = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().to_owned(), |_| {});
    let old_device = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest {
            device_id: old_device.clone(),
        })
        .unwrap();
    let old = InputSessionRequest {
        session_id: receipt.session_id,
    };
    service.fixture_detach().unwrap();
    let disconnected = service.wait_state(&old, |s| s.status == InputSessionStatus::Disconnected);
    assert!(disconnected.error.is_some());
    assert!(service.private_binding_view("missing").is_err());
    service.stop(old.clone()).unwrap();
    assert!(service
        .start(InputSessionStartRequest {
            device_id: old_device
        })
        .is_err());
    let new_device = service.fixture_attach().unwrap();
    let new = service
        .start(InputSessionStartRequest {
            device_id: new_device,
        })
        .unwrap();
    assert_ne!(old.session_id, new.session_id);
    assert!(service.state(old.clone()).is_err());
    assert!(
        service.stop(old).is_err(),
        "stale stop must not kill current session"
    );
    service
        .stop(InputSessionRequest {
            session_id: new.session_id,
        })
        .unwrap();
}

#[test]
fn calibration_and_raw_binding_persist_only_locally_and_validate_mapping() {
    let _owner = tests::TEST_OWNER.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let service = service::InputService::new(dir.path().to_owned(), |_| {});
    let device = service.fixture_attach().unwrap();
    let receipt = service
        .start(InputSessionStartRequest { device_id: device })
        .unwrap();
    let mut save = InputBindingsSaveRequest {
        session_id: receipt.session_id.clone(),
        controls_profile_id: "pad".into(),
        bindings: BTreeMap::from([(
            "jump".into(),
            LogicalBinding::ControllerButton { button: "a".into() },
        )]),
        raw_bindings: BTreeMap::new(),
        deadzones: BTreeMap::from([("leftx".into(), 0.2)]),
    };
    service.save_bindings(save.clone()).unwrap();
    let view = service.private_binding_view("pad").unwrap();
    assert!(matches!(
        view.raw_bindings["jump"],
        RawInputBinding::Button { index: 0 }
    ));
    assert_eq!(view.identity.port, Some(0));
    assert_eq!(view.deadzones["leftx"], 0.2);
    assert_eq!(view.bindings, save.bindings);
    save.raw_bindings
        .insert("jump".into(), RawInputBinding::Button { index: 1 });
    assert!(
        service.save_bindings(save.clone()).is_err(),
        "raw must agree with known SDL mapping"
    );
    save.raw_bindings.clear();
    save.deadzones.insert("leftx".into(), f64::NAN);
    assert!(service.save_bindings(save.clone()).is_err());
    save.deadzones.clear();
    save.bindings.insert(
        "jump".into(),
        LogicalBinding::ControllerButton {
            button: "unknown".into(),
        },
    );
    assert!(service.save_bindings(save).is_err());
    service
        .stop(InputSessionRequest {
            session_id: receipt.session_id,
        })
        .unwrap();
    let stored = service::InputService::new(dir.path().to_owned(), |_| {});
    assert!(
        stored.private_binding_view("pad").is_err(),
        "restart cannot silently select a physical identity"
    );
    assert_eq!(
        stored.local_binding_view("pad").unwrap().deadzones["leftx"],
        0.2
    );
    assert_eq!(eden::calibrate(-0.1, 0.2), 0.0);
    assert_eq!(eden::calibrate(1.0, 0.2), 1.0);
    assert!(eden::resolve_raw(
        "guid,unknown,",
        &LogicalBinding::ControllerButton { button: "a".into() }
    )
    .is_err());
    assert!(eden::resolve_raw(
        "guid,test,leftx:a0~,",
        &LogicalBinding::ControllerAxis {
            axis: "leftx".into(),
            direction: AxisDirection::Positive
        }
    )
    .is_ok());
}

#[test]
fn rate_limit_and_stale_observations_do_not_reopen_terminal_session() {
    let now = Instant::now();
    let mut session = service::Session::fixture("session-1", now);
    let sample = ControllerState {
        instance_id: 1,
        buttons: BTreeMap::from([("a".into(), true)]),
        sticks: [0; 4],
        triggers: [0; 2],
        raw: RawState {
            buttons: vec![true],
            axes: vec![0; 6],
            hats: vec![0],
        },
    };
    assert!(session
        .observe(
            "old-session",
            Ok(sample.clone()),
            now + Duration::from_secs(1)
        )
        .is_none());
    assert!(session
        .observe(
            "session-1",
            Ok(sample.clone()),
            now + Duration::from_millis(10)
        )
        .is_none());
    assert!(session
        .observe(
            "session-1",
            Ok(sample.clone()),
            now + Duration::from_millis(34)
        )
        .is_some());
    assert!(session
        .observe(
            "session-1",
            Err("unplugged".into()),
            now + Duration::from_millis(35)
        )
        .is_some());
    assert_eq!(session.state.status, InputSessionStatus::Disconnected);
    assert!(session
        .observe("session-1", Ok(sample), now + Duration::from_secs(2))
        .is_none());
    assert_eq!(session.state.status, InputSessionStatus::Disconnected);
}
