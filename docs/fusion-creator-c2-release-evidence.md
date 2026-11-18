# C2: report association after an immutable release

Base: `creator-wave7-base` (`c13cac4`). `project_list` now verifies a stored
report through its release ID, matching project/revision, manifest BLAKE3,
executor and declared profile pair. Release JSON is stored before verification
and immutable; a later report does not require a backlink in that JSON.
No DTO, manifest, migration, or IPC contract changed.

- RED, `.tmp/c2-report-red.log`: Windows `cargo test --manifest-path
  src-tauri/Cargo.toml --locked --offline --lib
  fusion_creator::recovery_tests::killed_child_recovers_and_resumes_each_of_the_five_durable_boundaries
  -- --exact --nocapture` — ran one test, failed with `IntegrityViolation`
  while reading the verified revision via `project_list`.
- GREEN, `.tmp/c2-report-green.log`: Windows `RUST_TEST_THREADS=4 cargo test
  --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_creator::
  -- --nocapture` — **139 passed, 0 failed, 1 opt-in ignored**, exit 0. Across
  all five recovery boundaries it checks the real runner's committed report
  through `project_list` and compares the release JSON with its original bytes.
  A report with a different manifest digest is still rejected.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` and `git diff --check`
  passed. Independent read-only review: no blocker; not a second test run.
- Production release-content binding and build/verify/export are separate work;
  the recovery test's file handler is a test fixture, not a production verifier.
