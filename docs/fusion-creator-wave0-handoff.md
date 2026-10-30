# Fusion Creator v1 — wave-zero handoff

Status: **WAVE ZERO COMPLETE — CONTRACTS FROZEN**. Native lifecycle and integrated aggregate QA passed after E/H integration. This is not acceptance of the complete Creator v1 product.

## Workspace and integration

- Worktree: `F:/MVP/Fusion-Creator-Integrator`, branch `creator/integrator`.
- Baseline: `creator-base` / `287dc370b891577c5c5ca27ac9126766daf89920`.
- Source snapshot excludes private signing keys, game payload, local gameplay evidence and unrelated third-party tools. The original dirty worktree/index was not reset, switched or staged.
- E technical spike integrated as `b1e13fa` (original `ad709f4419dae24b1ade86c4f77d6a3f6cd1ab6c`).
- H foundation integrated as `1f3116f` (original `92127e6e704ec431c562d54745e73bafe91dc7a1`). H module is registered only under `cfg(test)`.
- No push and no integration into the original user's working branch.

## Delivered contracts

- `docs/fusion-creator-contracts-v1.md`: versioning, projection decisions, IPC signatures/events/ordering, resume boundaries and ownership/i18n namespaces.
- Rust portable manifest extension: `src-tauri/src/fusion_source/creator_schema.rs`; project/job/report/input/signature modules: `src-tauri/src/fusion_creator/`.
- `GameManifest.creator` and canonical retention; metadata reader accepts 1.0/1.1, SourceIndex remains 1.0; inheritance allows 1.1 from 1.0/1.1 and rejects 1.0 from 1.1. Legacy Packer recipes still emit 1.0.
- Optional build/control profile IDs are carried through options/plan/snapshot contracts without changing absent-field legacy serialization. Actual 1.1 installation/profile application is explicitly unavailable until A/B implement it.
- `src/types/fusionCreator.ts`: TypeScript interfaces and strict Zod schemas. `docs/schemas/fusion-creator-contracts.schema.json`: draft-2020-12 project/extension/job/input and related DTO definitions, not the complete existing GameManifest validator.
- `fixtures/creator/contracts.json`: shared contract fixtures, not a production bridge.
- All 20 Creator/input command names are typed and registered; unavailable commands return `action_unavailable`. No job, export, input release or file write is falsely reported successful. Reserved event names are `fusion-creator:job-progress` and `fusion-input:state`, scoped by jobId/sessionId and positive monotonic safe sequence.
- `ResolveProjection`, `NormalizeDirectory` and `PrepareCandidate` are signatures only. No alternative installer, no successful resolver stub, no signing abstraction.
- SQLite remains version 5; C owns the next free single migration (currently 6) for project/private-bindings/job/release/report storage. G uses C's runner/storage. Root dependencies/locks and i18n are unchanged relative to creator-base; only additive IPC/module registration changes were made to shared wiring.

## Lifecycle and hard links

The existing service install/update/rollback test passed natively. Its readiness check is not a launch check.

Foundation H separately passed **3 tests**, including `native_desktop_install_launch_update_rollback_preserves_latest_save` on Windows. It uses real Packer/FusionService/CAS/SQLite/supervisor and ordinary DesktopSourceTransport with explicit local-directory authority. Full InstallRequest fields are supplied; the author folder is removed before install. Per-hash executable approval, real child launch, runtime update/fresh approval, rollback and latest saves are exercised.

`Access denied` on immutable hard links did not reproduce in these runs. No copy fallback or new cross-volume test was added on an unproven volume hypothesis. No production filesystem fix is claimed. H's initial configuration corrections were test-owned explicit raw destinations, activation manifest and directory saves; see `docs/fusion-creator-harness.md`.

## Checks and evidence

All commands run in the integrator worktree unless stated otherwise. Shared build-cache prefixes do not skip tests.

- `cargo test --manifest-path src-tauri/Cargo.toml --lib fusion_source::creator_harness_tests -- --nocapture`: **3 passed**, run on the H worktree before integration; repeated by the aggregate suite after integration.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib fusion_source::service_tests::install_exact_trust_runtime_update_and_rollback_keep_latest_saves -- --exact --nocapture`: **1 passed**.
- `node --experimental-strip-types --test src/types/fusionCreator.node-test.ts src/types/fusionCreatorJsonSchema.node-test.ts`: **13 passed**, including all command declaration/registration/constant and event-name parity.
- Python `jsonschema.Draft202012Validator`: schema and four public shared fixture contracts **PASS**.
- SDL after integration: `cargo test --manifest-path tools/creator-input-spike/Cargo.toml --locked --offline`: **1 passed**; `cargo clippy --manifest-path tools/creator-input-spike/Cargo.toml --locked --offline --all-targets -- -D warnings` and crate fmt check: **PASS**. Uses an external spike-only target cache. Pinned Rust crate 0.38.0 reports native SDL 2.26.4; final SDL initialized flags are zero after virtual device/worker teardown.
- Independent read-only review: **PASS**, no security concerns or blocking logic errors. Reviewed contracts/schema/fixtures, actual wiring, H source and E spike. Positive artifact checkpoint example corrected to carry artifactId. E must enforce handlesClosed=true on successful release; false/timeout/ambiguity is an error, as specified, not a wave-zero success path.
- `git diff --check`: **PASS** during review/integration.
- Final `CARGO_TARGET_DIR='F:/MVP/Fusion Launcher/src-tauri/target' npm run qa`: **PASS, exit 0**. Typecheck, **142 Node tests**, static-check, production Next.js build, **544 library Rust tests + 3 Packer CLI tests**, cargo fmt check and all-targets clippy with `-D warnings` passed. **4 preexisting tests ignored**, not counted as passing. Ordinary parallel test execution retained. Exact aggregate log: `.tmp/creator-wave0-qa-retry1.log`.

## Failures encountered, not erased

1. `npm ci --no-audit --no-fund` failed with registry EIDLETIMEOUT and cleanup EPERM warnings. Its partial directory was isolated; trusted existing dependencies are used via a junction. This does not prove clean dependency installation or filesystem sandboxing.
2. An initial Rust contract round-trip test compared JSON integer 0 and f64 0.0 as raw serde_json::Value. It failed; the test now compares typed InputState round-trip. Production DTOs were not changed to accommodate the assertion.
3. One full parallel Rust suite failed `patch_adapter_reaps_timeout_and_failed_tool_and_cleans_private_files` because its existing cleanup assertion saw `.fusion-patch-*`. The exact isolated test passed, followed by **25 consecutive isolated passes**. The root cause of that single full-suite cleanup failure is **UNCONFIRMED**. No unrelated transforms code, timeout, test concurrency or assertions were changed. A successful later aggregate run does not retroactively make that failure a PASS. Failed log: `.tmp/creator-wave0-qa.log`.
4. Early wrong `--exact` short-name invocation selected zero tests; it is not acceptance evidence. Earlier RED version/planner tests failed before their respective fixes and later passed. These delayed background notifications must not be interpreted as new regressions.

## Explicit NOT TESTED / remaining scope

- Physical SDL controller, USB/Bluetooth reconnect and OS-level release before real Eden; virtual fixtures are not hardware evidence.
- External HTTPS full streaming acquisition with an empty cache; H's explicit local-directory source does not establish it.
- Real Eden/TOTK gameplay and FPS presets, production input service, Creator jobs/recovery/build/verify/export; these belong to later waves. The current production actions are unavailable.
- New v1.1 projection/profile/install behavior and privacy allowlist export are not implemented in wave zero.
- Real cross-volume hardware acceptance; no cross-volume diagnosis was established.
- `npm run mvp:visual` / native UI acceptance: not run for this DTO/backend/test-only delta. No UI component, CSS, visual token, baseline screenshot or i18n text changed relative to creator-base.
- Four preexisting native sidecar/onefile tests remain ignored because they require generated seeder/production PyInstaller artifacts. They are not counted as passing; the ignore set was not changed.
- Fresh-machine install/package acceptance and clean dependency reinstall after the npm registry failure.

Baseline discrepancies are delivered in `docs/fusion-creator-wave0-audit.md`. Ownership and namespaces are authoritative in the frozen contract document. Subsequent contract changes require owner + integrator coordination and synchronized Rust/TS/schema/fixture updates.

The frozen contract delta is left in the integrator working tree, not silently committed or pushed. E/H integration commits are already present at HEAD `1f3116f`. A reproducible patch of the remaining delta is provided at `.tmp/creator-wave0-frozen.patch`; apply it on that HEAD before starting downstream worktrees. Do not branch downstream agents from the old creator-base alone: it omits the frozen contract delta and E/H integration.
