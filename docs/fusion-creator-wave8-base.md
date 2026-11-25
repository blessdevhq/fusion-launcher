# Fusion Creator wave 8: immutable release evidence

Base: `creator-wave7-base` (`c13cac4`). This is a host-side C2/G integration, not a production verifier or exporter. Public contracts, SQLite schema, and IPC are unchanged.

- `project_list` follows the report's release ID and checks matching project/revision, manifest BLAKE3, executor and profile pair without rewriting the immutable release. See [C2 regression evidence](fusion-creator-c2-release-evidence.md).
- A build handler stages `manifest.json` and only the declared `artifacts/<id>` files under its owned workspace. `checkpoint_release` audits the CAS bundle and commits its receipt with the release row and `ReleaseBuilt` checkpoint. A loose CAS object or metadata-only row does not authorize a read.
- `read_release_content` binds audit to exact project/revision/release and returns verified manifest bytes, inventory and artifact file handles. Start/resume and checkpoint revalidation pin an explicitly selected release to its receipt. Artifact reads recheck the opened file. The regression covers discarded staging bytes, wrong revision and project, tampered bytes of unchanged size, and wrong digest at publication.

## Verification

- C2 association: 139 focused tests passed (one opt-in ignored); pre-fix recovery regression reproduced `IntegrityViolation`.
- Release-content branch (`f5bda14`), `.tmp/green-final.log`: 142 focused tests passed (one opt-in ignored).
- Integrated Windows `RUST_TEST_THREADS=4 npm run qa`, attempt 1: **FAIL**, exit 101 after 229 Node tests, 727 Rust tests (five ignored) and three Packer CLI tests passed. Clippy rejected the Windows-only tamper fixture's readonly-bit clearing. This is a real test-code lint failure, not a flaky test. A Windows-only lint allowance on that expression explains why it does not modify ACLs.
- Attempt 2, `.tmp/wave8-qa-02.log`: **PASS**, exit 0 — 229 Node tests, 727 Rust tests (five ignored), three Packer CLI tests, formatting and Clippy. Both attempts held `F:/MVP/.qa.lock` exclusively.

## Not tested / not shipped here

No real Creator Build/Verify/Export handlers are registered by this wave. A byte audit does not prove a manifest signature, clean install, observed launch, or export allowlist. HTTPS and the physical controller acceptance were not run. No UI files changed; visual acceptance was not run for this slice.
