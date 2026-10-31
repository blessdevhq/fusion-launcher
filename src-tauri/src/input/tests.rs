use super::*;
use std::sync::Mutex;

// SDL process-global ownership is intentional; unrelated callers are rejected,
// not serialized silently. Only these independent test cases share this lock.
static TEST_OWNER: Mutex<()> = Mutex::new(());

#[test]
fn virtual_controller_reads_real_sdl_and_acknowledges_release() {
    let _test = TEST_OWNER.lock().unwrap();
    let worker = InputWorker::start().expect("SDL must initialize on its owner worker");
    assert!(
        InputWorker::start().is_err(),
        "a second worker must be rejected"
    );
    assert!(
        worker.read().is_err(),
        "reading without selection is not success"
    );
    assert!(
        worker.open(u32::MAX).is_err(),
        "unknown instances cannot be opened"
    );
    let id = worker.fixture_attach().unwrap();
    let devices = worker.devices().unwrap();
    let device = devices
        .iter()
        .find(|device| device.instance_id == id)
        .unwrap();
    assert!(device.is_game_controller);
    assert_eq!(worker.open(id).unwrap(), id);
    assert!(
        worker.open(id).is_err(),
        "only one controller handle may be opened"
    );

    for pressed in [true, false] {
        worker.fixture_button(0, pressed).unwrap();
        let state = worker.read().unwrap();
        assert_eq!(state.instance_id, id);
        assert_eq!(state.buttons["a"], pressed);
        assert_eq!(state.raw.buttons[0], pressed);
    }
    for hat in [1, 3, 2, 6, 4, 12, 8, 9, 0] {
        worker.fixture_hat(hat).unwrap();
        let state = worker.read().unwrap();
        assert_eq!(state.raw.hats, vec![hat]);
        for (name, bit) in [("dpup", 1), ("dpright", 2), ("dpdown", 4), ("dpleft", 8)] {
            assert_eq!(state.buttons[name], hat & bit != 0, "hat={hat}, {name}");
        }
    }
    for index in 0..4 {
        for value in [i16::MIN, -1, 0, 1, i16::MAX] {
            worker.fixture_axis(index, value).unwrap();
            let state = worker.read().unwrap();
            assert_eq!(state.sticks[index as usize], value);
            assert_eq!(state.raw.axes[index as usize], value);
        }
    }
    for index in 4..6 {
        for (raw, logical) in [(i16::MIN, 0), (0, 16383), (i16::MAX, 32767), (i16::MIN, 0)] {
            worker.fixture_axis(index, raw).unwrap();
            let state = worker.read().unwrap();
            assert_eq!(state.triggers[(index - 4) as usize], logical);
            assert_eq!(state.raw.axes[index as usize], raw);
        }
    }
    let proof = worker
        .stop()
        .expect("release must be acknowledged after teardown and join");
    assert!(proof.handles_closed);
    assert_eq!(proof.sdl_initialized_flags, 0);
    println!("release proof: {proof:?}");
    // The ownership lease must have been released too.
    let restarted = InputWorker::start().unwrap();
    assert!(restarted.stop().unwrap().handles_closed);
}

#[test]
fn disconnected_controller_still_closes_before_acknowledgement() {
    let _test = TEST_OWNER.lock().unwrap();
    let worker = InputWorker::start().unwrap();
    let id = worker.fixture_attach().unwrap();
    worker.open(id).unwrap();
    worker.fixture_detach().unwrap();
    assert!(!worker
        .devices()
        .unwrap()
        .iter()
        .any(|d| d.instance_id == id));
    assert!(
        worker.read().is_err(),
        "detached input must not look like neutral input"
    );
    let proof = worker.stop().unwrap();
    assert!(proof.handles_closed);
    assert_eq!(proof.sdl_initialized_flags, 0);
}

#[test]
fn dropping_worker_joins_and_releases_sdl_ownership() {
    let _test = TEST_OWNER.lock().unwrap();
    {
        let worker = InputWorker::start().unwrap();
        let id = worker.fixture_attach().unwrap();
        worker.open(id).unwrap();
        // Dropping a client must not orphan the worker or native references.
    }
    let report = probe().unwrap();
    assert!(report.release.handles_closed);
    assert_eq!(report.release.sdl_initialized_flags, 0);
    println!("native app module probe: {report:?}");
}
