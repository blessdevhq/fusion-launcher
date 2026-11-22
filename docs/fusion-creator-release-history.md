# Creator release history — native read path

Base: `creator-wave8-base` (`45005ad`). The existing `fusion_creator_release_list` and `fusion_creator_report_get` wire contracts are unchanged. The additive dispatch in `commands/fusion_creator.rs` runs against the application-owned SQLite store. No new registration, DTO, migration or dependency.

The release list is ordered by descending project revision, then by release ID. Its existing `reportIds` field is populated from validated report rows in the same read transaction, never from an update to stored release JSON. Report lookup checks the release ID, project, revision and pinned manifest BLAKE3. Current-revision reports also match the current descriptor's executor and profiles. Historical reports are not compared to the current descriptor. This is display metadata, not release-byte audit, verification, or export authorization.

## Authoring gap (not changed here)

`docs/fusion-creator-contracts-v1.md` defines read-only access to private bindings on project open. Project save and job start intentionally cannot write those bindings. `save_project_with_bindings` exists in typed storage, but no production command sets `componentPaths` or `signingKeyRef`. UI Build requires an explicit local authoring path to persist this data separately. Do not encode it in the descriptor, job request, diagnostics or published Source.

## Verification

- `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_creator::project_service_tests::history_reopens_old_release_and_later_report -- --exact`: **RED**, failed with `ActionUnavailable` before the read path was implemented.
- `RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_creator::`: **PASS**, 144 passed, one ignored. The test reopens SQLite after a project edit, reads a later report, checks immutable release JSON, and rejects an unrelated manifest digest. The tests used the exclusive Windows QA lock.
- Review follow-up: `history-review-red.log` reproduced both the unchecked historical profile IDs and the incorrect `Io` for a non-BLOB report. `history-review-green.log`: **PASS**, 146 Creator tests, one ignored, under the same exclusive lock. History now checks the wire bounds for every report's profile IDs and classifies malformed SQL column values as `IntegrityViolation`. It still does not re-verify old profile pair membership without the original descriptor; stored history does not authorize export.
- `node --experimental-strip-types --test src/types/fusionCreator.node-test.ts`: **PASS**, 9 tests. Initial additive macro dispatch double-counted declarations in the namespace test; it was fixed without changing the test or registered names.
- Full `npm run qa`, visual acceptance and desktop runtime: **NOT TESTED** on this branch.
