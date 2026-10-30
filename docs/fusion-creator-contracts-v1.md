# Fusion Creator v1 — wave-zero contract

Status: **FROZEN — wave-zero integrated QA, native lifecycle and E/H foundation checks passed**.
Base: `creator-base` (`287dc37`); integration worktree: `creator/integrator`.
This is the wave-zero contract freeze, not an announcement that Creator v1 works.
Acceptance evidence, known failures and NOT TESTED scope: `fusion-creator-wave0-handoff.md`.

## Versioning and authority

- SourceIndex wire schema remains `1.0` (`schema::SCHEMA_VERSION` remains unchanged).
- Client metadata reader accepts GameManifest `1.0` and `1.1`. `1.0` cannot carry `creator` or inherit from `1.1`; `1.1` can inherit from `1.0`/`1.1`.
- `GameManifest.creator` is the typed 1.1 extension (`fusion_source/creator_schema.rs`). It carries adapter reference, component provenance, finite build/control profiles and projection decisions. It is retained in canonical/storage forms.
- Legacy Packer recipes still emit 1.0; absent extension/profile IDs are omitted, preserving legacy serialization/hash behavior.
- In wave zero, 1.1 execution and non-null profile selections are **explicitly unavailable**. Parsing is not installing; no profile application/resolution/build/export implementation is advertised. A/B remove the execution gate only with complete behavioral tests.
- CreatorProject portable schema is separately versioned `1.0`; project revision is a monotonic safe JSON integer. Its public data has snake_case keys. CamelCase IPC envelopes may contain the unchanged portable project object.
- Local source paths, key file references, device GUID/serial/path/port, calibration and raw bindings live in private storage, not CreatorProject/Source/releases. A `managed` origin requires an explicit true redistribution approval before export; a boolean field alone is not permission enforcement. All export is unavailable until G implements the allowlist.
- No SigningProvider or ProjectionResolver trait is introduced. Existing Ed25519/TOFU and native installer remain authoritative.

## Data contracts

Portable definitions: `src-tauri/src/fusion_source/creator_schema.rs` and Zod/types in `src/types/fusionCreator.ts`.

- `AdapterReference { id, version }` pins `eden_portable`/`0.2.1` for the initial adapter; it is not a claim that D is implemented.
- Component origins are tagged `managed { artifact_ref, redistribution_allowed }`, `external { url, transport_blake3, inventory_blake3 }`, `user_provided { requirement_id, inventory_blake3, instructions }`.
- A project component has its own `id` and `layer_id`; the published extension maps `layer_id` to origin without private author bindings.
- `BuildProfile { id, title, layer_ids, controls_profile_ids }` is a finite exact set with finite valid control pairings. Dependencies must already be valid; no arbitrary addon mixing with profile selection.
- `ControlsProfile { id, title, device, bindings }` carries logical keyboard/mouse or SDL GameController assignments. D alone serializes to validated Eden ParamPackage. Physical instance IDs never become portable controller identity.
- `GameOptions`, `PlanOptions`, `ExecutionPlan` carry optional `buildProfileId` / `controlsProfileId`; snapshots retain the actual selection in their plan. Absent fields deserialize from old snapshots and are not emitted. Wave zero rejects selected profile IDs before any candidate mutation.
- Reports are separate kinds: `structure_integrity`, `clean_install`, `observed_launch`, `manual_scenario`, `https_acquisition`; status `passed|failed|not_tested`. Each report binds release ID/hash, project revision, adapter version, profile pair and environment. One report must not imply the other kinds passed.
- Shared public fixtures: `fixtures/creator/contracts.json`; used by Rust serde and Node/Zod tests. Preview/test bridges may reuse these; production cannot return them as real operation results.

JSON Schema definitions: `docs/schemas/fusion-creator-contracts.schema.json`. These validate the extension/project/DTOs; they do not replace all existing GameManifest path, hash, graph and runtime checks.

## Pure projection resolver signature (owner B)

`fusion_creator/projection.rs` freezes `ResolveProjection = for<'a> fn(ProjectionRequest<'a>) -> Result<ResolvedProjection, FusionError>`; the implementation name will be `resolve_projection`. This is a type signature only, not a successful executable placeholder.

Inputs: explicit `Legacy10|Explicit11` semantics; selected layers with layer ID, mount path and verified file inventories; explicit `{ path, winning_layer }` decisions. No IO, network, dependency inference or mutable global state.

Rules for 1.1:

1. Validate slash-separated workspace-relative paths using the existing Windows path safety policy; case-fold for collision keys without changing the retained spelling. Reject traversal, absolute/UNC/device/ADS/reserved names, invalid file/parent collisions and duplicate conflicting decisions.
2. Same resulting path **and same verified hash** from multiple layers is compatible. Keep the equivalent provider list and a deterministic representative; no dependency-based priority. Different sizes for the same hash are invalid inventory evidence.
3. Different hashes at the same path require an explicit author winner. The winner must be selected and actually provide that path; missing, excluded or unknown winners are errors. Decisions must be validated for each declared profile, not only the default.
4. Resolved output maps path to `ProjectionFile { path, winning_layer, source_relative_path, blake3, size, equivalent_layers }`. Integrity and executable approval use that actual provider and expected bytes, not the overwritten layer.
5. Source 1.0 keeps its existing legacy semantics. The common implementation must expose the mode explicitly; no silent migration of existing installations.
6. Resolve/validate before candidate creation. Patch input/output and generated config checks remain separate. The resolved map is not permission to write outside the inactive workspace.

Related signatures: `NormalizeDirectory` (same deterministic streaming normalization for author/player) and `PrepareCandidate` (inactive workspace only). The latter cannot change active config, saves, shared persistence or local bindings before activation. Implementation belongs to A/B/D, not this DTO-only wave.

## IPC and event ordering

All argument-taking commands accept a single `{ request: DTO }` Tauri argument. Registration belongs to the integrator. Exact command name arrays are in Rust and TypeScript and tested against registration.

| Command | Request | Result |
| --- | --- | --- |
| fusion_creator_project_list | — | CreatorProject[] |
| fusion_creator_project_open | CreatorProjectRequest | CreatorProject |
| fusion_creator_project_save | CreatorProjectSaveRequest | CreatorProject |
| fusion_creator_scan / classify / build / verify / export / verify_https (each with fusion_creator_ prefix) | CreatorJobStartRequest | CreatorJobReceipt |
| fusion_creator_job_get / job_cancel | CreatorJobRequest | CreatorJob |
| fusion_creator_job_resume | CreatorJobRequest | CreatorJobReceipt |
| fusion_creator_release_list | CreatorProjectRequest | CreatorRelease[] |
| fusion_creator_report_get | CreatorReportRequest | CreatorReport |
| fusion_input_devices | — | InputDevice[] |
| fusion_input_session_start | InputSessionStartRequest | InputSessionReceipt |
| fusion_input_session_stop / release_for_launch | InputSessionRequest | InputReleaseReceipt |
| fusion_input_session_get | InputSessionRequest | InputState |
| fusion_input_bindings_save | InputBindingsSaveRequest | InputBindingsReceipt |

**Wave-zero availability:** every listed production command returns structured `action_unavailable` with its command name; no files written, job created, handles closed or success fabricated. Event names are reserved, not emitted by the unavailable bridge.

- `fusion-creator:job-progress`: required `jobId`, `projectId`, `projectRevision`, positive safe `sequence`, operation kind, status, phase, units, nullable checkpoint/release/error.
- `fusion-input:state`: required `sessionId`, positive safe `sequence`, nullable `deviceId`, status, logical buttons/axes, nullable structured error. Stick axes -1..1; triggers 0..1. E throttles state events (maximum 30 Hz contract); disconnect/terminal notifications are immediate. No raw device identity in public state.
- Sequence scope is **per job** (persist across restart) or **per input session** (new session ID on reopen), range 1..2^53-1. Never reset/reuse an existing scope; overflow is an error.
- Consumers accept only the active ID/revision and strictly greater sequence. Terminal jobs/sessions cannot return to running/active because of late events; closing UI does not cancel a job. Reopening fetches current state before consuming buffered progress.
- Worker stop/release receipt means actual handles closed after explicit acknowledgement. Timeout/disconnect ambiguity is an error, not proof of release; Eden launch remains blocked until acknowledgement. Binding save receipt does not assert handles were closed.

## Resume boundaries (owner C; G uses this runner)

Only these durable checkpoints are reusable:

| Boundary enum | Required committed evidence |
| --- | --- |
| scan_classification_saved | Scan **and classification** snapshot stored atomically, with classification revision/hash. A completed scan alone is not this checkpoint. |
| artifact_committed | One complete artifact verified against expected bytes/inventory and atomically published. `artifactId` required for this boundary. One checkpoint per artifact. |
| release_built | Immutable assembled release and its manifest/artifact inventory committed; scratch recipe construction is not sufficient. |
| verification_completed | Complete immutable report for its release/profile pair and environment. No partial acquisition/launch counts as verified. |
| export_completed | Complete allowlisted export atomically published and its inventory/hash committed. |

Every checkpoint carries `checkpointId`, `jobId`, boundary, result BLAKE3 and a stamp of **projectRevision + every input hash + adapter ID/version**. Before reuse, C/G revalidate the stamp and committed result. Wrong/missing/changed results are discarded or produce a diagnostic; interrupted files/partially applied operations are never checkpoints. A resumed job preserves history/sequence; no stale result applies to a new project revision. Startup marks unfinished jobs interrupted. One writer owns each workspace. Cancellation/error preserves the previous immutable release and active installation.

## Ownership and integration

| Owner | Exclusive module responsibility | Shared-file rule | i18n namespace proposed to F |
| --- | --- | --- | --- |
| Integrator | Contract freeze, common wiring, wave boundaries, baseline snapshot | Sole writer of root Cargo.toml/Cargo.lock, package.json/package-lock.json, commands/mod.rs + lib.rs IPC registration, common scripts; additive scoped changes only | none |
| A | fusion_source schema/canonical/evolution/Packer and origin-aware URL normalization | Other streams submit wire changes to A, not parallel edits | creator.source.* |
| B | Installer plans/options, workspace/snapshots, pure projection resolver and candidate API | A/D/G call the common resolver; no competing implementation | creator.install.* |
| C | fusion_creator project contracts/IPC implementation, jobs/storage/release/report registry | Single next SQLite migration; owns schema-version recheck before migration, G uses storage API | creator.project.*, creator.job.* |
| D | eden_portable adapter, allowlists, normalization/config ownership, ParamPackage | No changes to active/shared config before activation | creator.adapter.eden.* |
| E | native input worker/provider under fusion_creator input boundary | Supplies dependency/IPC additions to integrator, no root Cargo overwrite | input.* |
| F | Creator/input UI, stores, frontend API and preview fixtures | Sole editor of existing src/lib/i18n.ts for all streams; no empty parallel locale files; both ru/en | creator.ui.*, creator.common.* and other streams' submitted namespaces |
| G | recipe compilation + build/verify/export/HTTPS job handlers | Uses C runner/storage, A Packer and existing signatures; no extra migration or key abstraction | creator.release.*, creator.verify.* |
| H/H2 | Synthetic fixtures and common integration/native harness | Unit tests remain with owners A–G; no production fixture backdoor | no production keys |

Frozen types are owned by their module owners above after freeze; changes go through owner + integrator, update all Rust/TS/JSON Schema/shared fixtures, and notify consumers. Root dependency additions from E are integrated deliberately; a standalone technical spike does not install a production SDL service. No UI or i18n key is added in this wave; namespaces reserve ownership only.
