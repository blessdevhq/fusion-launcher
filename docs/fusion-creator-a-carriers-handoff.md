# Creator A — program carriers, reader and canonical handoff

Parent handoff: B `16d617e175347ee7ecc74da00b916f3b4bf40b67`, based on immutable `creator-wave1-descriptor`.

## Delivered

- Strict Rust and Zod `ProgramDescriptor 1.0`, `CreatorProgramProject 1.1`, and `ProgramCreatorManifest`, including the frozen nested fields, constants, cardinalities, scalar/path/argument/environment constraints and portable-origin checks.
- `GameManifest 1.1.creator` is a strict, untagged alternative between the existing legacy Creator extension and `{ program_descriptor: ... }`. These are extensions of one GameManifest publication format, not an independent second manifest format. Mixed variants and unknown/private fields fail closed.
- Reader/upcaster and canonical serialization retain the inline typed descriptor. A regression proves descriptor changes affect the canonical hash. Existing raw-manifest hash/signature binding is not replaced.
- Program/project raw JSON readers enforce 2 MiB before parsing, reject duplicate keys recursively and trailing JSON, and sanitize errors. Source parsing retains its 16 MiB outer bound and now rejects duplicate map keys before serde can discard them. Embedded descriptor structural validation enforces its serialized 2 MiB bound. Rust-constructed public DTOs can be structurally revalidated.
- CreatorProgramProject revisions accept zero through `9007199254740991`. Portable documents do not carry local paths, signing-key references or device identity.
- URI validation is consistent between Rust and Zod: raw URI grammar and percent escapes are checked before authoritative HTTPS URL parsing. Raw U+FEFF/U+0085, controls, malformed escapes and userinfo are rejected; percent-encoded UTF-8, uppercase HTTPS and valid IPv6 syntax remain accepted. No URL value is silently repaired.
- Portable controls maps preserve own `__proto__`/`constructor` JSON keys safely rather than losing them through Zod 3 record merging. The same correction applies to the existing legacy portable controls carrier; normal valid legacy wire fields are retained.

## Scope / contracts

Public additions live in `fusion_source::program_schema` and `src/types/fusionProgram.ts` (re-exported from `fusionCreator.ts`). Rust root types expose `read_json` and `validate`. `CreatorExtension` is the new Rust union used by wire/canonical GameManifest.

No SourceIndex version, frozen JSON Schema definitions/fixtures, Cargo/package dependencies, lockfiles, IPC registration, UI or SDL changes. Program-specific logical binding numbers retain the frozen JSON numeric contract without widening existing input IPC types.

**Reading authoring metadata is not execution authority.** Descriptor-only program metadata may omit precompiled legacy execution fields, but `PlanBuilder` and workspace reconstruction still reject 1.1 execution with `action_unavailable`. Program profile compilation/preflight without the future executor is explicitly unavailable, never legacy/native fallback. Cross-references, codec/type compatibility, complete settings, normalization, verified-inventory resolution for descriptors, compiler equivalence, trust/approval and runtime capabilities remain X's obligations. Remaining A inheritance/relative URL/recipe-mode work is not delivered here.

## Actual verification

Native commands use `.tmp/creator-agent/check.py`, preserving real exit/logs and setting `RUST_TEST_THREADS=4` with serialized shared Windows build access.

- Genuine RED: missing Rust/Zod carriers; reader refusal of `creator.program_descriptor`; duplicate bindings discarded by the old reader; Zod special-key data loss in program and legacy bindings; Rust acceptance of raw U+FEFF and Node acceptance of raw U+0085.
- `node --experimental-strip-types --test src/types/fusionProgram.node-test.ts src/types/fusionCreator.node-test.ts`: **13 executed tests PASS**, followed by `npm run typecheck` PASS.
- `check.py parent-green-uri-parity cargo test --manifest-path src-tauri/Cargo.toml --lib -- program_schema packer::projection_tests --nocapture`: **14 executed tests PASS**.
- Final `check.py parent-qa-a-url-final` / `RUST_TEST_THREADS=4 npm run qa`: **exit 0; 159 Node, 575 Rust library + 3 CLI PASS; 4 existing ignored; typecheck, static-check, build, fmt, clippy PASS**.
- Independent static review `deleg_06a8dab6`: **PASS**, no security/logic blockers, after two earlier FAIL verdicts were addressed. Reviewers did not run builds/tests or claim runtime proof.
- `git diff --check`: PASS.

Final QA log SHA-256: `7423bf4b291ba15bcb974f05c2d5c3aa985bc43cb2c3a976801b0fc732926805` (`.tmp/creator-agent/parent-qa-a-url-final.log`).

### Retained failure / native qualification

The initial full A QA (`parent-qa-a-carriers`) exited 101: 573 library tests passed, one unchanged `windows_sidecar::tests::graceful_exit_preserves_final_output_and_releases_job` failed at `windows_sidecar.rs:995`, `job.is_empty()`, and 4 tests were ignored. The identical assertion was already recorded in C's pre-carrier `parent-qa-storage-internals.log`; it is not evidence of a new carrier regression.

No native production or native test code was changed. A temporary observational probe did not reproduce delayed accounting in 32 isolated and 32 loaded repetitions; its registration was removed. This does **not** establish a root cause or fix. Later loaded-library and full-QA runs succeeded. The initial FAIL remains recorded and is not relabeled PASS. This handoff does not claim native scheduling stability.

The temporary Python schema environment initially lacked an actual URI format checker. `rfc3986-validator 0.1.1` was installed only under ignored `.tmp/creator-agent/schema-validation-deps`; a Draft202012 validator with verified `uri` assertion then rejected both raw Unicode cases and malformed URI examples. No application dependency was added.

## NOT TESTED / not delivered

- Declarative executor/compiler, real SuperTuxKart lifecycle, physical controllers, GUI integration or hosted CI.
- Typed C storage facade, jobs/recovery, and the rest of A.
- Semantic host acceptance of arbitrary descriptors; strict structural parsing alone is insufficient.

`creator-wave2-base` is not created by this handoff; the C handoff and aggregate integration QA are still required.
