# Creator B — resolver and finite profile preflight

Base: immutable `creator-wave1-descriptor` (`ef3947a836fe89eb0584fd25fc6f62edbe2ab5d6`).
This is the first B handoff, before the separately requested Rust/Zod program carriers and executor.

## Implemented

- One pure `resolve_projection` for verified inventories, with explicit `Legacy10` and `Explicit11` semantics. Legacy keeps ordered last-provider-wins, including case-only path overrides. Explicit profiles require real winning providers for differing bytes; equal providers are order-independent. Invalid paths, file/parent and Windows casing conflicts, duplicate inventories/rules/providers, redundant or unused rules, and inconsistent sizes are rejected.
- Legacy workspace assembly and Packer use that resolver for ordinary-layer segments. Patch layers remain ordered barriers, not fictitious verified output inventories. Workspace checks the cumulative path tree, including patch outputs, before reading/reconciling active persistence, taking a backup, or creating a candidate. Packer retains existing patch input/output integrity, critical-file, and config-output guards.
- `fusion_creator::profiles` validates finite build/control identity, references, uniqueness, required layers, dependency closure, declared runtime selection, exact allowed pairs, and rejection of arbitrary addon overrides. `PlanBuilder` calls the metadata guard before its existing execution-unavailable gate.
- Packer inventory preflight resolves every declared build profile with `Explicit11`, not only a default profile, using supplied verified inventories. All allowed control pairings refer to those validated projections; device state does not change file resolution. Actual resolved winners supply critical-file digests and executable roles. Conflicting locator-key/critical declarations fail closed.

## Boundaries and unchanged contracts

- No JSON Schema/wire fields, legacy hash serialization, dependencies, lockfiles, IPC registration, UI, SDL, or frozen descriptor fixtures changed.
- `ProgramDescriptor 1.0`, `CreatorProgramProject 1.1`, and `ProgramCreatorManifest` production carriers are the next, separate A handoff.
- **This does not enable GameManifest 1.1 execution.** `PlanBuilder` still returns `action_unavailable`; workspace reconstructs the plan contract through that gate. No fallback to the legacy/native executor was added.
- A selected profile containing patch transforms explicitly returns `action_unavailable`; no verified output bytes/sizes or successful transform execution are fabricated. The later executor/compiler owns that execution capability.
- The profile result is preflight data, not executable approval, Source trust, or a claim of runtime lifecycle acceptance.

## Verification

Native checks use `.tmp/creator-agent/check.py`, which serializes the shared Windows native target and sets `RUST_TEST_THREADS=4`, `CARGO_BUILD_JOBS=4`, and a physical `FUSION_TEST_PYTHON`.

| Check | Actual result |
| --- | --- |
| Focused initial profile collision RED | exit 101, missing profile preflight reproduced |
| Profile selection RED | exit 101, addon override without profile IDs reproduced |
| Winning executable-role RED | exit 101, wrong actual provider role reproduced |
| Cross-patch-boundary assembly RED | exit 101, reached `MissingPatchTool` instead of rejecting `InvalidManifest` before mutation |
| Contradictory provider declarations RED | exit 101, incorrect successful preflight reproduced |
| `check.py parent-green-patch-boundary cargo test --manifest-path src-tauri/Cargo.toml --lib -- patch_barrier_path_conflicts profile_integrity projection --nocapture` | exit 0; 22 executed tests PASS before the additional contradictory-declaration assertion |
| `check.py parent-qa-b-reviewed-fixes` (`RUST_TEST_THREADS=4 npm run qa`) | exit 0; 151 Node tests, 568 Rust library tests, 3 Packer CLI tests PASS; 4 existing ignored; typecheck, static-check, build, fmt and clippy PASS |
| `check.py parent-confirm-b-boundaries cargo test --manifest-path src-tauri/Cargo.toml --lib -- packer::projection_tests projection::tests patch_barrier_path_conflicts --nocapture` | exit 0; 15 executed tests PASS after the final full QA, including contradictory provider declarations |
| `git diff --check` | PASS |

The cross-barrier regression exercises six file/parent conflict directions through real public workspace assembly using local synthetic CAS inputs. It asserts unchanged persistence bytes, active snapshot ID, and candidate count; patch tools do not run on the rejected inputs. Winning-role coverage includes launch, runtime, orchestrator, and contradictory locator-key metadata.

Initial independent review: FAIL on winning executable role and cumulative cross-patch path validation. Both findings were reproduced and fixed. Independent static re-review `deleg_56f147e8`: **PASS**, no security concerns or logic errors; it did not run tests or claim runtime verification. A positive patch-before-later-overlay regression remains a non-blocking coverage suggestion, not claimed tested here. Earlier RED/FAIL logs are retained, not relabeled PASS.

Final full-QA log SHA-256: `81d582a16201c5f538ed5ac80846441153d1e425819b14fa8df73054accec7ad` (local `.tmp/creator-agent/parent-qa-b-reviewed-fixes.log`). Added-lines static security scan: no findings. The commit pins the source and this report; hosted CI is not implied.

## NOT TESTED / not delivered here

- Declarative compiler/executor, normalizer execution, real SuperTuxKart lifecycle, native GUI/gamepad interaction, or physical controllers.
- New typed program project persistence and migration (C), jobs/recovery, program carrier reader/canonical/Zod integration (A).
- New profile patch execution; selected patch transforms fail explicitly rather than succeed speculatively.
- Hosted CI for this commit; the reported QA is real local Windows verification.

No `creator-wave2-base` is created by this handoff. It requires the three separate accepted SHA/QA handoffs and aggregate integration QA.
