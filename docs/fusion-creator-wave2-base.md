# Creator wave 2 — integration base

Integration branch: `creator/wave2-base`, created from immutable `creator-wave1-descriptor` (`ef3947a836fe89eb0584fd25fc6f62edbe2ab5d6`). The three accepted prerequisites were merged in order with fast-forwards; no conflict resolution or history rewrite was needed. B/A/C code and both additive module registrations are retained.

## Accepted prerequisite handoffs

| Handoff | Published SHA | Real four-thread full QA |
| --- | --- | --- |
| B resolver / finite-profile preflight | `16d617e175347ee7ecc74da00b916f3b4bf40b67` | exit 0; 151 Node, 568 Rust library + 3 CLI; 4 existing ignored |
| A program carriers / reader / canonical | `7cf3bd7d45ef9888a66e11f466fae1283862b0b5` | exit 0; 159 Node, 575 Rust library + 3 CLI; 4 existing ignored |
| C typed project storage / private bindings / v6 migration | `3ccf682460e2479dfac1269fd78e1e3414b32af3` | exit 0; 159 Node, 586 Rust library + 3 CLI; 4 existing ignored |

All three handoffs have independent static PASS reviews and exact origin branch SHA read-back. Detailed scope, regression logs, qualifications and NOT TESTED items:

- [B handoff](fusion-creator-b-handoff.md)
- [A handoff](fusion-creator-a-carriers-handoff.md)
- [C handoff](fusion-creator-c-storage-handoff.md)

## Aggregate gate

`python .tmp/creator-agent/check.py parent-qa-wave2-aggregate` runs the actual `RUST_TEST_THREADS=4 npm run qa` in the integration worktree with serialized native-target access and a physical Python interpreter. This is a new full run after all three merges, not a reuse of an individual handoff's result.

Status: **PASS, exit 0**. Actual results: **159 Node tests, 586 Rust library tests + 3 CLI tests PASS; 4 existing ignored**. Typecheck, static-check, frontend build, Rust fmt and clippy all passed. The run was performed after all three merges, with no test exclusions or retry for this aggregate run.

Aggregate log SHA-256: `4fa0a75c6a1cd847cf6bbdf3565f64d6ae5e9904f5e2d0b8104bb42f1da67157` (local `.tmp/creator-agent/parent-qa-wave2-aggregate.log`). Before publication, the source/fixture/build-input diff against the accepted C head was verified empty; the final integration commit adds only this report. All three prerequisite SHAs are ancestors of the integration head. The annotated `creator-wave2-base` tag identifies the integration report commit and the unchanged verified code.

## Meaning of this base

- Shared pure projection resolver, legacy workspace/Packer adapters and finite legacy-Creator profile preflight are implemented; patch barriers remain ordered and cumulative path conflicts are checked before candidate/persistence mutation.
- Strict `ProgramDescriptor 1.0`, `CreatorProgramProject 1.1` and nested `ProgramCreatorManifest` Rust/Zod carriers are integrated with reader/canonical hashing. One GameManifest publication format remains; SourceIndex stays 1.0.
- SQLite v6 typed project storage uses the same A reader, exact input bytes, BLAKE3, atomic portable/private CAS, and explicit private access. There is no duplicate portable-file database write.
- These are the requested prerequisites for X, **not a declarative executor or a complete Creator product**. Program compilation/execution, normalization, descriptor semantic/verified-inventory validation and capability activation remain unavailable until implemented by X. Frozen Creator/input IPC actions remain explicitly unavailable pending their separate host integration. C jobs/recovery and the rest of A remain separate work.

No UI, controller hardware, SuperTuxKart real lifecycle, hosted CI or new visual baseline is claimed tested. The four existing ignored tests remain ignored. The unchanged native sidecar's earlier intermittent `job.is_empty()` failure is retained in the A/C history; later successful QA does not establish its root cause or fix. No native production/test behavior was changed to suppress it.

Original dirty launcher and C worktrees are preserved. Published wave 1 tags remain immutable; main is not merged or rewritten.
