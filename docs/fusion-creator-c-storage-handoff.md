# Creator C — typed project storage phase 1

Parent A handoff: `7cf3bd7d45ef9888a66e11f466fae1283862b0b5` (includes accepted B `16d617e175347ee7ecc74da00b916f3b4bf40b67`). Branch: `creator/backend-typed`.

## Delivered

- `fusion_creator::project_storage` consumes the single A `CreatorProgramProject 1.1` reader and shared size/revision bounds. No duplicate descriptor DTO or public opaque-byte bypass was introduced.
- `save_project_bytes` / `save_project_with_bindings` validate UTF-8, strict portable shape, duplicate keys, size and revision before database mutation. The authoritative SQLite row stores the exact supplied bytes and BLAKE3; no second mutable portable JSON file exists.
- One `BEGIN IMMEDIATE` transaction checks portable and independent private CAS revisions, writes both tables, and returns success only after COMMIT. `None` is insert-only; `Some(n)` requires an existing row at n and a new document at n+1. Zero is valid; overflow is refused. Omitted bindings preserve the existing private row and its independent revision.
- Typed private updates bind to the exact project and known component IDs. Paths/key references are stored as private metadata only: this layer never reads, normalizes, executes, approves or exports them.
- `open_project` rechecks hash, A's typed document, and embedded project ID/revision against the selected SQL row. `list_projects` offers explicit pages of 1–32 with offset and performs the same checks. Portable reads never select/join private bindings.
- `open_private_bindings` is a separate explicit API, validating private shape, duplicate keys and embedded project identity. SQL/parse failures do not echo payloads, SQL trigger text or private paths.
- Transactional SQLite v6 migration adds only `creator_program_projects` and `creator_private_bindings`; revision bounds are 0..9007199254740991. Migration tests preserve populated repository/catalog/download data, check idempotence, and prove failed second-table creation rolls back the first table and user_version. Historical reset fixtures were updated for the real new tables rather than weakening production DDL.

The original `F:/MVP/Fusion-Creator-C` dirty worktree was preserved. Its scoped file contents still match the temporary export index used for integration. The additive module registration retains B's profiles module.

## Actual verification

- First typed-save attempt: INVALID compile because an old unavailability test still used the former non-null expected-revision signature; recorded separately, not counted as a behavioral failure.
- Genuine RED `parent-red-c-typed-behavior`: one executed test failed on `ActionUnavailable` before connecting A's carrier to the real write.
- `check.py parent-green-c-typed-save cargo test --manifest-path src-tauri/Cargo.toml --lib -- project_storage creator_migration_tests --nocapture`: 8 executed tests PASS.
- Final focused `parent-c-typed-boundary` with the same filters: **11 executed tests PASS**, including typed reopen/list, private CAS, rollback after a second-table failure, sanitized errors, digest-consistent embedded-ID/revision tampering and private/portable isolation.
- Independent static review `deleg_0fcf734e`: **PASS**, no security/logic blockers or suggestions. Reviewer did not execute builds/tests or claim IPC readiness.
- Final `check.py parent-qa-c-typed-final` / **`RUST_TEST_THREADS=4 npm run qa`: exit 0; 159 Node, 586 Rust library + 3 CLI PASS; 4 existing ignored; typecheck, static-check, build, fmt, clippy PASS**.
- `git diff --check`: PASS.

Final full-QA log SHA-256: `920f340f51026e045b4c631bdb4f37750836e1832d5642844b9ac92cc66c96a2` (local `.tmp/creator-agent/parent-qa-c-typed-final.log`). Prior migration-fixture and native failures remain in their original logs; no native fix is claimed here. The pre-existing sidecar qualification is detailed in the A handoff.

## Contracts / boundaries / NOT TESTED

This is the requested **storage-first C prerequisite for X**, not the completed C jobs/UI surface. APIs are crate-visible Rust storage operations exercised against real temporary SQLite files. Frozen portable wire contracts, dependencies, lockfiles and IPC registration are unchanged. Registered Creator/input IPC operations remain explicitly unavailable until their separate host/job integration; no successful IPC/UI action is claimed.

Jobs, checkpoint/recovery orchestration, release export, GUI, physical controllers, declarative execution and real SuperTuxKart lifecycle are NOT IMPLEMENTED / NOT TESTED in this handoff. No power-loss/crash durability experiment or hosted CI was performed. Descriptor cross-reference/build semantics still belong to X and are not implied by structural project persistence.

The three prerequisite handoffs can now be integrated from immutable `creator-wave1-descriptor`. This commit itself does not create `creator-wave2-base`; the required aggregate four-thread QA remains a separate integration gate.
