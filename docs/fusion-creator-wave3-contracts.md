# Wave 3 — project IPC 1.1 handoff

Base: annotated `creator-wave3-base^{}` = `e30ee4b3950b3279dcbca7492579104968c75615`.
Branch: `creator/wave3-contracts`. Delivery tag: annotated `creator-wave3-contracts`.
The delivery commit is resolved with `git rev-parse creator-wave3-contracts^{}` (not the tag-object SHA).

## Audit: legacy references at the integration base

The following is an inventory of wire dependencies, not a request to implement C2.

1. `src-tauri/src/fusion_creator/mod.rs`: `CreatorProject`/`CreatorComponent` define the legacy
   1.0 document with top-level `adapter`, `components`, `build_profiles`, `controls_profiles`,
   `projection_rules`. `CreatorProjectSaveRequest.project` used that type and its
   `expected_revision: u64` could not express storage's insert-only `None`.
2. `src-tauri/src/commands/fusion_creator.rs`: project list/open/save response signatures all
   used `CreatorProject` (`Vec<CreatorProject>` for list); their shared unavailable-command test
   constructed the legacy save request and old profile IDs.
3. `src/types/fusionCreator.ts`: `creatorProjectSchema`/`CreatorProject` describe the same
   legacy top-level fields; `CreatorProjectSaveRequest.project` and all three `CreatorApi`
   project return types referred to it. `CreatorPrivateBindings` was only an interface, without
   a validated open response or the storage revision envelope.
4. Rust `CreatorResumeStamp.adapter` and TS `CreatorResumeStamp.adapter` required legacy
   application-adapter identity. This dependency propagates through `CreatorCheckpoint.stamp`
   and `CreatorJob.checkpoints`; a 1.1 project has an executor, not a top-level adapter.
5. Rust/TS `CreatorReport.adapter` had the same dependency. Report `buildProfileId` /
   `controlsProfileId` and job-start profile arrays implicitly referred to old project profiles.
6. Rust/TS `CreatorJobStartRequest` is shared by scan/classify/build/verify/**export**/verify_https.
   It embeds no project or adapter; its revision/profile reference semantics still needed
   alignment with `program_descriptor`. `CreatorRelease` embeds only immutable IDs/hashes and
   needs no structural change. No separate export-project DTO or existing project IPC client
   was found; no runtime bridge or legacy store is introduced.
7. `docs/schemas/fusion-creator-contracts.schema.json`: the root and `$defs/CreatorProject`,
   `CreatorComponent`, `CreatorManifest`, `BuildProfile`, `ControlsProfile`, `AdapterReference`
   retained the legacy shapes. `CreatorResumeStamp`/`CreatorReport` required `adapter`;
   checkpoint/job reference them transitively. Project request/open/save/list and bindings
   envelopes were absent. Descriptions of `CreatorProgramProject`/`ProgramCreatorManifest`
   incorrectly still said reader/carrier support was pending.
8. `docs/fusion-creator-contracts-v1.md`: “Формат данных и профили” described the project as 1.0
   and top-level components/profiles; the carrier section (base lines 106–122) still said the
   new Rust/TS/reader forms were not accepted; the IPC table returned `CreatorProject` for all
   project commands. Report and durable-resume prose referred to adapter ID/version.
9. `docs/schemas/README.md`: the root table described 1.0 as the portable project without an IPC
   distinction; reader/carrier-support and private-envelope coverage descriptions were stale.
10. `fixtures/creator/contracts.json`: `project` was 1.0, `profileSelection` used legacy IDs,
    and `manifest.creator` is a legacy published extension. Rust `fusion_creator::tests`,
    command tests, TS `fusionCreator.node-test.ts`, `fusionCreatorJsonSchema.node-test.ts`,
    `fusionProgram.node-test.ts` and `fusionCreatorProfile.node-test.ts`
    used those top-level project fields; checkpoint/report schema examples derived `adapter`
    from the old project. `fusion_source/tests.rs` also consumes the legacy published manifest
    fixture; it must keep working, not be reinterpreted as a new project export.
11. Existing `fusion_source/program_schema/contract_tests.rs` and TS schema/carrier tests use
    `CreatorProject` to prove separation/rejection of 1.1 shapes. They justify retaining the
    legacy validation type/schema, **not** a production project store or IPC branch.
12. `src-tauri/src/fusion_source/creator_schema.rs`, its TS/Zod equivalents and JSON definitions
    still model the old published `CreatorManifest` extension. Those top-level adapter,
    component and profile fields are intentional published-document compatibility, not a
    project IPC contract. The strict `ProgramCreatorManifest { program_descriptor }` alternative
    already exists. Historical plans/handoffs retain their original context; this report and
    the updated normative contract supersede their legacy project-IPC wording.

## Frozen changed contracts

- **Open:** `CreatorProjectRequest { projectId }` →
  `CreatorProjectOpenResponse { project: CreatorProgramProject /* 1.1 */,
  privateBindings: SavedPrivateBindings | null, missingInputs: string[] }`.
  Missing inputs are descriptor component IDs needing local selection/rebinding. C2 computes
  them; an empty list is not proof of byte-pin verification.
- **Binding state:** exactly the existing storage shape
  `SavedPrivateBindings { revision, bindings: { projectId, componentPaths, signingKeyRef } }`.
  Only its Rust definition is shared from the DTO module; storage functions/SQL/revision rules
  are unchanged. It is local IPC/storage state, never part of a portable project/export.
- **Save:** `{ project: CreatorProgramProject /* 1.1 */, expectedRevision: number | null }`
  → `{ revision }`. Nullable field is required: null means insert-only; n updates n to the
  supplied project.revision n+1. This matches `project_storage::check_revision`; schema parsing
  does not perform a write or optimistic-concurrency validation.
- **List:** an array of `{ projectId, title, revision, verificationStatus, modifiedAt }`.
  Status is `passed|failed|not_tested`, tied to current-revision evidence, not job state.
  `modifiedAt` is UTC Unix epoch milliseconds, integer 0..9007199254740991. No descriptor or
  private state is included; timestamp storage/status calculation remains C2 work.
- **Resume/report:** replace `adapter` with the existing strict `ProgramExecutor`
  (`fusion.declarative`, API `1.0`). Program identity/version stays in descriptor.identity;
  canonical descriptor hash and implementation/private-input digests are required before reuse
  through `inputHashes`, as in the existing normative policy. No runner is added.
- **Jobs/export:** profile IDs resolve inside the pinned project's descriptor, not top-level
  legacy profiles. The existing request shape is unchanged and now has strict Zod/JSON Schema
  definitions. New program publication uses `ProgramCreatorManifest`; legacy published
  `CreatorManifest` support is retained separately. Export never includes open/bindings state.
- **Legacy:** `CreatorProject` 1.0 is retained only for existing validation/negative tests.
  Project IPC never accepts/returns it. No 1.0 storage, implicit migration, runner, command
  implementation or new SQLite migration is added.

## Shared fixtures and validation

`contracts.json.project` is now 1.1; `legacyProject` preserves the previous 1.0 validation
sample. `manifest` intentionally remains the old published-extension fixture used by Source
reader tests, not a program-export example. New fixture values cover private binding revision,
missing component IDs, save revision, summary, descriptor executor resume/report, and job/export
request with valid nested profile IDs. Private paths are synthetic local-envelope examples only.
Fixture hashes/receipts are schema data, not successful execution/verification evidence.

Rust serde, Zod-derived TS types, JSON Schema, command signatures and the normative contract
are updated together. Tests cover unknown/missing/nullable fields, legacy and duplicate-field
rejection, safe revision/time bounds, private/portable separation, executor identity and profile
references. Existing command tests still require exact `action_unavailable` and `details.command`
for every production Creator/input command.

The full-suite first attempt also exposed two old fixture consumers in the program/profile
tests; these are migrated explicitly to `legacyProject` where the test is about legacy shape.
Review additionally identified UTF-16 versus Unicode-code-point length disagreement for IPC
IDs; the common Zod identifier is aligned with the program carrier/JSON Schema and regression
cases cover astral IDs at the 128/129 code-point boundary. Required nullable job/export fields
are explicit in serde as well as Zod/Schema; ordinary serde Option defaults are not the wire policy.

### Verification

Environment: local Windows, Node 24.14.0, Rust 1.95.0, VS2022 bundled CMake
3.31.6-msvc6 on PATH; physical `FUSION_TEST_PYTHON` (not a reparse alias), external
`CARGO_TARGET_DIR=F:/MVP/Fusion Launcher/src-tauri/target`, local ignored `src-tauri/target`
present for existing fixtures. Installed Node dependencies are reused via the existing
trusted junction; this is not a clean npm-install or sandbox claim. Shell exports did not
carry into subsequent tool invocations, so build environment variables are supplied in the
same invocation as Cargo/QA. Two preliminary Cargo attempts stopped before tests because
ordinary PATH lacked CMake; this was setup failure, not FLAKY or SDL/product failure.

| Check | Observed result |
| --- | --- |
| Initial focused Zod/JSON Schema development tests | RED before missing local envelopes/schema definitions were added; later 26/26 passed |
| First executed focused Rust suite | 92 passed / 1 failed: missing job `releaseId` accepted by plain serde Option; corrected all three nullable request fields, no gate relaxed |
| Focused Rust after correction, `RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml fusion_creator:: --lib` | **PASS: 93 passed / 0 failed / 0 ignored**, 572 filtered; `.tmp/wave3-contracts-rust-targeted.log` |
| First full `RUST_TEST_THREADS=4 npm run qa` | **FAIL**, typecheck passed, Node **163 passed / 2 failed** (165 total): stale program/profile fixture consumers listed above. Later gates NOT RUN in this attempt; `.tmp/wave3-contracts-qa.log`. These were defects corrected in this branch, **not FLAKY** |
| Conforming Draft202012Validator + meta-schema with registered URI format checker | **PASS: 17 shared fixture definitions**, portable root/open isolation and legacy-save/private-field rejection. Executed via isolated `uv run --no-project --with 'jsonschema[format]'`; no repository dependency changes |
| Corrected full `RUST_TEST_THREADS=4 npm run qa` | **FLAKY (initial FAIL, exact retry PASS)**: typecheck, **190 Node tests**, static-check and production frontend build passed. Full Rust library suite: **660 passed / 1 failed / 4 preexisting ignored**, with the sole failure `fusion_source::transforms::tests::patch_adapter_reaps_timeout_and_failed_tool_and_cleans_private_files` at `transforms.rs:1173`, residual `.fusion-patch-*`. Raw aggregate exit **101** is preserved in `.tmp/wave3-contracts-qa-final.log`; it is not relabeled PASS |
| One permitted exact retry, unchanged source/environment, `RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_source::transforms::tests::patch_adapter_reaps_timeout_and_failed_tool_and_cleans_private_files -- --exact --nocapture` | **PASS, exit 0: 1 passed / 0 failed / 0 ignored**, 664 filtered; `.tmp/wave3-contracts-cleanup-retry1.log`. No second retry was performed |
| Gates not reached after aggregate library failure: default-feature binary tests, doctests, `npm run rust:fmt`, `npm run rust:clippy` | **PASS, combined exit 0**: `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --bins` executed **3 Packer CLI tests**, all passed (launcher/local targets declare 0 tests); `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --doc` found 0 doctests; unchanged `npm run rust:fmt` and `npm run rust:clippy` (`--all-targets -- -D warnings`) passed. `.tmp/wave3-contracts-remaining-gates.log`. These gates were executed separately without suppressing/skipping the failed library test |
| Independent review after corrections | **PASS**, no security concerns or logic errors; `.tmp/wave3-contracts-final-review.json`. Reviewer inspected the Rust fix and separately executed **61/61** focused Node tests including **25** Unicode boundary regressions. This is not a claim that the reviewer ran Cargo/full QA |

The cleanup assertion fingerprint matches the previously recorded failure in
[`fusion-creator-wave0-handoff.md`, item 3](fusion-creator-wave0-handoff.md#failures-encountered-not-erased),
also recorded in `fusion-creator-harness.md` and `fusion-creator-input-native-spike.md`.
`src-tauri/src/fusion_source/transforms.rs` is unchanged from the integration base. The original
failure plus one successful exact repeat is reported as **FLAKY**, under the task's explicit
one-repeat rule, not a clean aggregate PASS. Root cause remains **UNCONFIRMED**; isolated success
does not prove absence of scheduling/cleanup defects. No cleanup implementation, timeout,
assertion, test parallelism or gate was changed to get this result.

No launch/export/storage-wiring acceptance is claimed; no new skips or ignored tests are introduced.

**Delivery verification status: FLAKY, with one exact retry PASS and every remaining QA gate
completed.** The original aggregate exit remains 101. Contract/schema checks and independent
review passed; no test, timeout, ignore set or quality gate was weakened. Only the documented
contract migration and its tests/docs are delivered; C2 must still implement command wiring,
missing-input calculation, summary metadata/status and the runner.
