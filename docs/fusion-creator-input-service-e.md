# Fusion Creator — E input service handoff

## Scope and integration boundaries

Worktree: `F:/MVP/Fusion-Creator-E`, branch `creator/input-service-e`, based on
`creator-wave2-base` / `5b3d611`. The service remains a module of the launcher,
not a separate crate. Windows SDL GameController v1 supports one selected
controller. Dual Joy-Con and motion are outside this implementation.

Frozen wire DTOs, JSON Schema, shared fixtures and
`docs/fusion-creator-contracts-v1.md` are unchanged. Cargo/package manifests,
lockfiles and i18n are unchanged. Shared-file changes are additive:

- `src-tauri/src/lib.rs`: initialize the lazy input service with a private local
  application-data directory; this does not initialize SDL or open a device.
- `src-tauri/src/commands/fusion_creator.rs`: specialized input dispatch arms
  connect the six existing command names to `InputProvider`. The historical
  `unavailable_command!` declarations are retained because the frozen namespace
  test scans that syntax. Other Creator actions are not changed.
- `.github/workflows/input-service-windows.yml`: a new E-only Windows workflow;
  existing workflows are not modified. Only the exact known sidecar test may
  trigger an automatic single retry. FLAKY requires that retry to succeed, and
  does not turn the original CI failure green.

No sidecar, torrent, supervisor, Eden ParamPackage writer, UI or navigation
implementation is modified by E.

## Service and private data

`input::service::InputService` implements the frozen `InputProvider` interface.
The IPC adapter performs blocking device, filesystem and release operations on
Tauri blocking tasks. The service is initialized once; before initialization,
handlers return a real `action_unavailable` diagnostic with `details.command`.
No successful preview/mock provider is installed in production.

Exactly one native SDL owner owns the context, event pump, subsystems and selected
GameController. Only owned commands, scalar values and snapshots cross its
channel. No SDL handles/pointers are placed in Tauri State or sent to the UI.
The native owner also samples the session; no second native reader is started.

Discovery returns opaque, fresh selection tokens, not GUIDs or SDL instance IDs.
It does not auto-select or auto-open a controller. Starting a session checks the
current instance, normalized GUID, port and SDL mapping. Unknown mapping is
unavailable, never a guessed button-number layout. A disconnected session cannot
be revived by late snapshots. Reconnection requires a new discovery, explicit
selection and binding confirmation. Device-topology changes, including a second
identical device, invalidate use of the old private selection.

`fusion-input:state` contains only the frozen logical wire state. Sticks are
normalized to `[-1,1]`, triggers to `[0,1]`. Active observations are limited to at
most 30 Hz; disconnect and final release events bypass that active-state limit.
Session IDs are fresh and sequence values increase within each session. Old
session requests cannot stop the current session. Sequence exhaustion is an
error and must not prevent native cleanup.

`fusion_input_bindings_save` validates finite per-axis deadzones in `[0,1)`,
logical names, raw input bounds, and agreement between submitted raw sources and
the selected controller's explicit SDL mapping. Omitted raw sources are derived
from that mapping, not from assumed controller button numbers. Writes use the
existing guarded atomic filesystem writer beneath the local application data
root `fusion-input-private`, outside any publishable project/workspace.

D's host-only API is `InputService::private_binding_view(controls_profile_id)`.
Its owned read-only result contains logical/raw bindings, the normalized GUID,
local port and instance, calibration, original SDL mapping, driver pin, binding
revision and topology. `PrivateBindingView::fingerprint()` provides a **local**
cache/resume stamp; neither the view nor fingerprint belongs in public profiles
or exports. An in-memory confirmation fingerprint plus live session/topology and
mapping checks reject modified-on-disk or stale private data. The separate
`local_binding_view` loader is diagnostic/local restoration data, not authority
for a live physical selection after restart.

GUID and hints were checked against Eden's pinned
`src/input_common/drivers/sdl_driver.cpp`, commit
`58c1e20ee58efa3900ba616207d460886214480b`, and its settings defaults. GUID bytes
2–3 (controller-name CRC) are cleared without reordering other bytes. The service
uses Windows defaults `enable_raw_input=false`, `enable_joycon_driver=true`,
`enable_procon_driver=false`, positional button labels and disabled combined
Joy-Con. The application-name hint intentionally names Fusion Launcher. D must
use compatible driver settings. SDL mapping half-axis/inversion semantics remain
in the private original mapping for D; E does not serialize ParamPackage strings.

## B/D launch integration requirement

`InputService::with_released_for_launch` is the synchronous host pre-spawn
barrier, including for `application_defaults`: it closes any session/discovery
worker and invokes the launch callback only after checked handle closure, SDL
teardown acknowledgement and join. The command gate stays held through the
callback to prevent capture starting between the barrier and the actual spawn.
The callback must perform the real synchronous spawn operation, not merely
return an unpolled future or schedule a future launch. A cached IPC receipt is
not a substitute for using the barrier at the actual launch boundary.

Missing ACK, failed cleanup, timeout or panic is a blocking diagnostic, not
release evidence. A timed-out join detaches only the thread client; the native
thread retains its ownership lease, so a replacement SDL worker cannot open.
Any failed SDL-worker initialization also latches an unconfirmed-release error:
absence of a worker client is not proof of native teardown. Discovery and launch
remain blocked with the same diagnostic until the service/application restarts.

**The actual executor/Eden spawn call is owned by B/D/the integrator and is not
wired in this E-only change.** This handoff is not evidence of an end-to-end Eden
launch or a completed Creator v1 acceptance.

## F: pause shell navigation during assignment

The inspected baseline `src/hooks/useGamepad.ts` subscribes to `window.keydown`;
`src/lib/focusManager.ts` defines `GamepadA/B/Y` action aliases but does not read
hardware itself. E does not synthesize shell keydown events.

F should acquire an explicit capture/focus owner **before** starting an assignment
session. While it is held, the shell hook must not collect global focus targets,
move focus or route activate/back/menu actions for captured input. Gate both any
future native/browser gamepad routing and keyboard-emulation events from external
remappers. Scope focus targets to the assignment dialog. Merely calling
`preventDefault()` in a dialog handler is insufficient: the existing shell hook
does not check ownership or `defaultPrevented` before dispatch. Stop must be
requested on dialog completion/cancellation; keep a release error visible and
launch blocked if ACK is not confirmed. F owns restoration of navigation and the
modal error/cancel UX. E does not modify either frontend module.

## Verification boundary

Virtual devices and injected release faults exist only under `cfg(test)`.
Fixtures use real native SDL virtual hardware rather than fabricated production
state. The user-requested isolated sidecar series executed exactly ten commands:

```text
cargo test --lib windows_sidecar::tests::graceful_exit_preserves_final_output_and_releases_job -- --exact
```

Working directory was this worktree's `src-tauri`. Results: **10 passed, 0 failed,
0 infrastructure failures**. Logs are
`.tmp/input-service-e/sidecar-ten/run-01.log` through `run-10.log`, and
`results.json`. Process checks preceded every invocation. Other agents started
native tests before the series and between runs 1 and 2; the runner paused
instead of starting a conflicting next invocation. This proves absence in the
recorded preflight snapshots, not absence of all concurrent activity throughout
every test's lifetime.

The subsequent full `RUST_TEST_THREADS=4 npm run qa` is preserved as
`.tmp/input-service-e/qa-after-ten.log`: **exit 101, 597 Rust library passed,
1 failed, 4 existing ignored**. The sole failure is the unchanged known
`windows_sidecar::tests::graceful_exit_preserves_final_output_and_releases_job`,
`src/windows_sidecar.rs:995`, `assertion failed: job.is_empty().unwrap()`.
Typecheck, 159 Node tests, static-check and frontend build passed in that run;
fmt/clippy were not reached. It is not relabeled PASS.

A later precommit review identified a separate input startup failure-path blocker.
Its real-SDL ownership regression was executed with
`RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --lib input::safety_tests::failed_worker_start_blocks_launch_until_service_restart -- --exact --nocapture`:

- RED (`startup-red.log`): exit 101; 0 passed, 1 failed, at the expected assertion
  that failed initialization must not invoke the launch callback.
- GREEN (`startup-green.log`): exit 0; 1 passed, 0 failed. The same test and inputs
  now reject launch both before and after the independent owner's confirmed stop;
  only recreating the service clears the failure. No production mock is involved.
- Independent static re-review (`final-review.json`) accepted the sticky error
  fix; it is not runtime evidence for an injected initialization-timeout thread.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` passed after fixing
  the formatting of E's own initialization call. Static parsing of all embedded
  PowerShell CI blocks and retry-policy assertions passed (`verify-ci-policy.py`).

The final post-fix full `RUST_TEST_THREADS=4 npm run qa` completed with **exit 0**:
`.tmp/input-service-e/qa-post-startup-fix.log` and its `-result.json`. Typecheck,
159 Node tests, static-check, frontend build, 599 Rust library tests, 3 packer CLI
tests, rustfmt and clippy (`--all-targets -- -D warnings`) passed. Four existing
Rust tests remain ignored. All 16 `input::` fixtures passed, including buttons,
hats, axes, triggers, stale events, disconnect/reconnect and release/startup faults.
The exact sidecar test also passed inside this full run. Three consecutive
process snapshots were empty before invoking npm; this is not a promise that
other agents could not start later.

**Sequence status: FLAKY, not an unqualified PASS.** The earlier full QA failed
only on the known sidecar assertion; the next executed full QA succeeded.
Between them E's independently identified startup blocker/regression and its own
initialization-call formatting were fixed. These are two different source
snapshots, not a controlled identical-source sidecar repeat. No extra full QA
was needed after the post-fix run, and the original failure log is preserved.
A process-preflight refusal and a waiting runner killed before invoking npm are
not additional QA attempts. Complete logs must distinguish actual QA execution
from those guards.

## NOT TESTED / remaining acceptance

- Physical USB/Bluetooth controller behavior, hardware-specific mapping/hints and
  OS-level release before real Eden; physical smoke belongs to the integrator.
- Assignment dialog, actual shell suppression and WebView/controller/remapper
  interactions; F implements and tests these.
- Real executor/Eden spawn-barrier wiring, ParamPackage generation and playback;
  these belong to B/D/integration.
- Hosted Windows CI and end-to-end Creator readiness are not implied by local
  fixtures, static review or a branch push.
