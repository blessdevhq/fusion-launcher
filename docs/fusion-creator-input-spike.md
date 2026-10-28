# Creator input: isolated Windows SDL spike

## Verdict: PARTIAL — native virtual-device path validated

The standalone test crate at `tools/creator-input-spike` builds and executes on
Windows with `sdl2 = "=0.38.0"`, `bundled` and `static-link`. It is an experiment,
not the production E service, an IPC implementation, or Creator v1 acceptance.
No root dependency manifest, application registration, or production module was
changed. The crate owns its lockfile; its local `target/` is ignored.

## Reproduce

From the repository root in Windows Git Bash, with the MSVC toolchain installed:

```bash
export PATH='/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin':"$PATH"
cargo test --manifest-path tools/creator-input-spike/Cargo.toml --locked --offline -- --nocapture
cargo clippy --manifest-path tools/creator-input-spike/Cargo.toml --locked --offline --all-targets -- -D warnings
cargo fmt --manifest-path tools/creator-input-spike/Cargo.toml -- --check
```

`--offline` requires the locked dependencies to be cached. On a new machine,
fetch them first with `cargo fetch --manifest-path tools/creator-input-spike/Cargo.toml --locked`.
Do not set a shared production `CARGO_TARGET_DIR` for this experiment.

## Executed evidence

- Host: Windows build 26200, `x86_64-pc-windows-msvc`; Rust/Cargo 1.95.0.
- Continued the existing local build cache rather than rebuilding SDL from scratch.
  Its native build log records `SDL2-static.lib`; runtime reports **SDL 2.26.4**.
  The Rust crate version 0.38.0 is not the native SDL version.
- Final test source passed twice consecutively: **1 passed, 0 failed, 0 ignored**
  per run. Doc-tests contain no tests. Clippy with all targets and denied warnings,
  and rustfmt check, both returned exit code 0.
- Diagnostic run: initial joystick count `0`, fixture GUID
  `ff008f6001f0000002f0000000007600`, session instances `[0, 1]`,
  SDL initialized flags after teardown `0`.
- All context, event pump, subsystem, controller and virtual-device operations
  execute on the single named worker. Only commands/replies and plain report
  data cross the thread boundary. No physical controller is opened by the harness.
- Input goes through `SDL_JoystickSetVirtual*` and actual SDL event polling;
  events are not injected with `SDL_PushEvent` and no provider is mocked.

### Assertions exercised

| Scenario | What the fixture requires |
| --- | --- |
| Buttons | Raw button 0 and mapped A down/up events, plus raw/logical state |
| Hat | Raw up, up-right and neutral values; corresponding logical D-pad transitions |
| Sticks | All four logical stick axes and matching raw axes at -32768, 32767 and 0 |
| Triggers | Both raw axes at -32768/32767 and logical rest 0 / full 32767 |
| Disconnect | Raw and controller removed events; detached state; drop followed by null native handle lookup |
| Reconnect | Same fixture GUID, different session instance; both added events and fresh raw/logical button events |
| Stop | Ready while handle is live; explicit Stop command; handle drop and null lookups; context/subsystem drop and `SDL_WasInit(0) == 0`; only then release acknowledgement; caller joins worker |

Successful report stages are ordered:
`context-created → ready-with-live-handles → stop-received → handles-closed-and-lookups-null → context-dropped-flags-zero → stop-acknowledged`.
A timeout or missing reply returns an error, never a fabricated release success.
Native null lookups and uninitialized SDL prove cleanup of this experiment's
SDL references, not release of an untested physical device's OS resources.

## Proven issues resolved

1. The preceding attempt had already replaced legacy virtual attach with a
   versioned `SDL_VirtualJoystickDesc` and explicit fixture-only VID/PID. This
   retained change makes the synthetic mapping recognizable by the bundled
   SDL GUID/CRC matcher. No physical layout is inferred.
2. The first resumed test failed waiting for `ControllerDeviceAdded` after the
   first raw attach. SDL's `SDL_GameControllerEventWatcher` classifies an added
   joystick synchronously, before this harness registers its mapping;
   `SDL_GameControllerAddMapping` does not replay that event. First attach now
   requires the raw added event and explicit successful mapping/open. Reconnect,
   with the mapping already registered, still requires both added event types.
3. The next run failed `disconnected fixture handle leaked`. In pinned
   `sdl2 0.38.0/src/sdl2/joystick.rs`, `Joystick::drop` calls
   `SDL_JoystickClose` only when `attached()` is true. A separate raw `Joystick`
   reference therefore survived disconnect until subsystem teardown. The fixture
   no longer opens that extra reference: raw reads/setters borrow the joystick
   owned by `GameController`, whose Drop closes its reference even when detached.
   The same handle-null assertions then pass before subsystem teardown. The
   dependency itself is not patched; production integration must preserve
   correct ownership for disconnected devices too.

## Limitations and integration handoff

- **Physical gamepad smoke: NOT TESTED.** USB/Bluetooth reconnect, hardware driver
  behavior, background input/focus, OS-level release before Eden, and concurrent
  device access remain unverified. Virtual fixtures are not that evidence.
- **Production service / Tauri IPC / UI / Eden handoff: NOT IMPLEMENTED or TESTED.**
  No assignment/calibration persistence, state-rate limiting, DTO contract,
  physical device selection, or launch gating is delivered here. The report is
  diagnostic test data, not a frozen production API or success stub.
- Only an explicit synthetic mapping is used. Real mapping database coverage,
  GUID normalization against Eden's pinned SDL/hints, raw-binding serialization,
  ambiguity among identical controllers and Dual Joy-Con are not validated.
  Device indices, reconnect instance IDs and GUIDs are distinct concepts;
  instance IDs must never become portable Eden bindings.
- One worker and one fixture lifecycle are exercised per test process. Multiple
  simultaneous callers, worker restart after panic, blocked native calls and
  forced cancellation are not covered. Joining is not a hard thread-kill deadline.
- Hat coverage is up/up-right/neutral, not every direction; button coverage is A,
  not every physical button. Synthetic unknown-type trigger axes initially sit
  at raw 0 (logical 16383); the test explicitly sets their rest value before
  checking range endpoints. Hardware-specific calibration/deadzones are absent.
- Full launcher QA, native UI, release packaging, clean-machine build and an
  independent code review have not been run for this isolated spike. Integration
  review and physical smoke remain required before claiming the full E/v1 path.

Recommendation: use the proven worker ownership, event semantics and release
ordering as integration constraints; do not copy the throwaway fixture as a
production service or count this result as complete Creator v1 acceptance.
