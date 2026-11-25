# Creator foundation H

**Integrated wave-one status:** the shared H helper and its consumer are now in `creator/contracts`;
the combined four-thread QA passed, including all six H scenarios and real native lifecycle.
Use the immutable `creator-wave1-base` tag after remote verification; see
[wave-one report](fusion-creator-wave1-base.md) for current results and exact-repeat evidence.
Actual A–G adoption remains work for those owners; the H-owned consumer proves the import contract,
not their completed implementation. Historical failures/readiness boundaries below are retained.

Historical H handoff status (before the current wave-one integration): **H implementation verified; integration acceptance OPEN**. Worktree `F:/MVP/Fusion-Creator-Harness`, branch `creator/harness`, baseline `92127e6` (foundation on `creator-base`). The shared API, reproducibility and H scenarios pass; downstream A–G adoption is not asserted and full frozen+E replay QA still has failures outside H. No new commit/push is included in this H task.

## One shared test-only API

- Implementation: `src-tauri/src/fusion_source/test_support/creator.rs`.
- Import: `crate::fusion_source::test_support::CreatorHarness`.
- `CreatorHarness::synthetic().await`: tiny authored Source; no executable/game/emulator payload. `CreatorHarness::native().await` is Windows-only and copies the launcher's own test image to reuse the existing supervisor child fixture.
- Use `fixture.service` directly with `fixture.game` and the complete `fixture.request`; methods `connect`, `snapshot`, `approve_and_launch` remain available in test configuration. Keep the fixture alive until all operations finish; do not retain cloned service/DB handles beyond it.
- `fixture.reproducibility.first/second` contain streamed BLAKE3 maps of actual relative publication files plus artifact transport digests, CAS digests and sizes, not a hash of output path strings.
- `publication_hashes` hashes test-owned plain files and rejects links/reparse points. Its negative-control test modifies the same filename's bytes and requires a different digest.
- The common module has both parent `#[cfg(test)]` registration and inner `#![cfg(test)]`; it is not a production IPC/feature or a separately exported test crate.

Consumers must be unit-test modules in the same Rust crate (registered under cfg(test)), not Cargo `tests/*.rs` integration crates that compile the library without cfg(test).

```rust
use crate::fusion_source::test_support::CreatorHarness;

#[tokio::test]
async fn my_scenario() {
    let fixture = CreatorHarness::synthetic().await;
    fixture.service.game_install(&fixture.game, fixture.request.clone()).await.unwrap();
    let installed = fixture.snapshot();
    assert!(installed.workspace_root.starts_with(&fixture.external_workspace));
}
```

Actual non-copying consumer: H-owned `src-tauri/src/creator_harness_consumer_tests.rs`, outside `fusion_source`. It imports the same API, uses two isolated fixtures, installs through their real services and checks that one consumer's SQLite/paths/state do not become the other's. Existing H lifecycle tests also import the common helper; no second CreatorHarness definition remains. This proves the import/extension contract; it does **not** claim actual A–G adoption or migration of their module tests.

## Additive shared registrations for the integrator

- `fusion_source/mod.rs`: `#[cfg(test)] pub(crate) mod test_support;` and registration of `creator_harness_tests` if not already present. The frozen integrator/E base already registers the latter: do not add it twice or overwrite that file.
- `src-tauri/src/lib.rs`: `#[cfg(test)] mod creator_harness_consumer_tests;` only. No runtime/IPC change.
- Cargo.toml/Cargo.lock, package manifests/locks, schemas, migration numbers and i18n are unchanged by H.

## What is exercised

- Existing `PackerEngine` builds a synthetic owned publication, an identical rebuild in another empty output directory, then a changed update. Payloads are authored raw bytes plus a tiny authored managed ZIP. No commercial game or emulator is distributed.
- Existing `LocalSourceCandidate` explicitly grants desktop-owned local-directory authority. Existing `DesktopSourceTransport` and the real `FusionService` perform acquisition and installation; this harness implements no transport trait and no alternate installer.
- Each fixture has its own temporary SQLite database, library/CAS, appdata, local-appdata, player input and external workspace root.
- Full `InstallRequest` supplies `user_files`, `raw_paths` and `external_workspace_root` explicitly. Local-locator reference and player-supplied raw input exercise distinct provenance paths.
- Author directory is removed before installation/launch. Published content and player-provided requirements are separate. Publication is signed with a fixed explicitly public TEST key; no real private key file is opened.
- Native Windows fixture reuses `supervisor::tests::native::native_process_fixture` by copying the current test executable and, when present, its external activation manifest. `game_launch` supervises a real child process and waits for exit/save reconciliation.
- First launch without executable approval must fail. Explicit per-file/per-hash approval is separate from the Source signature/TOFU pin.
- Release two changes the runtime's exact hash through a test-owned PE overlay. The scenario installs, launches, updates, requires fresh approval, launches again, rolls back, retains the newest save and launches the restored runtime. No game payload is used.
- `pub(crate) CreatorHarness` is available to other cfg(test) modules through the common test_support path; tests extend it by ordinary service calls rather than copying constructor/transport/installer implementations.

## Reproducibility scope

The same recipe, same input bytes/paths and same explicitly public TEST signing seed are built twice before modifying input for release two. Both output directories begin empty. Compare the complete relative file-to-BLAKE3 maps (signed index, manifests, raw/ZIP files and Packer metadata), the artifact transport/CAS digests and sizes. Player path hints are intentionally unchanged between these two builds. This does not claim identical outputs across different recipes, different absolute path hints, different native test executables/toolchains or a real Creator adapter/recipe compiler.

## Baseline adaptation, not a production feature

The baseline Packer cannot author user_provided recipe components. After Packer verifies its original artifact set, this TEST helper augments the existing 1.0 wire manifest with a player-supplied layer, writes new digest-named bytes, updates the immutable index URL/digest and signs that exact index. It does not claim Packer verified the augmented publication or implement the future Creator recipe compiler.

Saves use an explicit directory declaration (`workspace/saves`), matching the existing supervisor fixture's parent-directory atomic save behavior. Raw archive destinations, including the optional activation manifest, are supplied explicitly; filenames are never inferred from an untrusted URL. These are fixture configuration requirements, not hard-link fallback changes.

## Reproduce

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib fusion_source::creator_harness_tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib creator_harness_consumer_tests -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml --lib
npm run qa
```

On non-Windows, native launch scenario is compiled out; this is not a Windows PASS. Use default native toolchain environment consistently when sharing a Cargo cache across worktrees; changing MSVC environment between builds can rebuild all native dependencies.

## Historical verified execution and open gates

The results and unresolved gates below describe the earlier H/frozen+E snapshots, not the current merged tree. Current wave-one commands and acceptance results belong in the integrator report; no current aggregate PASS or FAIL is asserted here.

Local Windows, Rust 1.95.0. Commands used `CARGO_TARGET_DIR='F:/MVP/Fusion Launcher/src-tauri/target'` (shared build cache, not a shared fixture database). Node dependencies are a trusted local junction, not a sandbox or clean npm-install claim.

| Command / tree | Observed result |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib creator_harness -- --nocapture`, H tree | **6 passed / 0 failed / 0 ignored**, including real native lifecycle; 39.74 s |
| Two identical fixture builds | **8 relative file digests equal**, plus transport/CAS/size tuples of **2 artifacts**; managed ZIP 307 bytes, synthetic raw runtime 58 bytes |
| `RUST_TEST_THREADS=4 npm run qa`, H tree | **PASS**, 129 Node tests, 541 Rust library tests + 3 CLI tests, typecheck/static-check/build/fmt/clippy; 4 preexisting ignored |
| Same H-owned Rust files replayed onto frozen+E `0165f71`, `creator/harness-compat` | File SHA-256 comparison confirms identical H code; only additive cfg(test) wiring differs for existing registrations |
| `cargo test ... --lib creator_harness -- --nocapture`, frozen+E replay | **6 passed / 0 failed / 0 ignored**, 27.73 s; root consumer uses the common helper |
| `cargo fmt ... -- --check`, `cargo clippy ... --locked --offline --all-targets -- -D warnings`, replay | **PASS** |
| `cargo check ... --locked --offline --lib`, replay | **PASS**, ordinary non-test library build; no production helper registration |

RED was observed first: new consumer/repro tests failed to compile against the empty common module (`CreatorHarness` / `publication_hashes` absent). Log `.tmp/harness-red.log`. GREEN `.tmp/harness-green.log` completed with 6 passing tests; a read-only reviewer inspected it before completion, so that earlier partial log alone was not counted as acceptance. Review verdict: **PASS, no blockers**. Its consumer-name suggestion was applied (`common_harness_is_reused_from_a_root_test_module`), and that renamed test passes in the full H QA and final targeted replay.

Failures retained, not turned into PASS:

- First ordinary-parallel H `npm run qa`: **540 passed / 1 failed / 4 ignored** at existing supervisor `graph_primary_crash_stops_support_process_and_returns_sanitized_report`, Windows error 32 / executable lock still held. Exact isolated test then passed (3.19 s); four-thread H QA passed. Cause **UNCONFIRMED**; supervisor source/timeouts/assertions unchanged.
- Frozen+E replay `RUST_TEST_THREADS=4 npm run qa`: **142 Node passed; 549 Rust passed / 1 failed / 4 ignored** at existing sidecar `graceful_exit_preserves_final_output_and_releases_job` (`job.is_empty`). Isolated sidecar test then passed (0.28 s).
- Replay `RUST_TEST_THREADS=1 npm run qa`: **142 Node passed; 549 Rust passed / 1 failed / 4 ignored** at existing patch timeout cleanup (`.fusion-patch-*`, transforms.rs:1173). Sidecar and all H tests passed. No foreign source or tests were altered; combined aggregate QA remains **FAIL**, not a claimed integrated PASS.

H logs/evidence are under its `.tmp/`: `harness-red.log`, `harness-green.log`, `harness-qa.log`, `harness-supervisor-repro.log`, `harness-qa-four-threads.log`, `harness-reproducibility.json` (parsed real output, not generated expected hashes). Replay logs are under `F:/MVP/Fusion-Creator-Harness-Compat/.tmp/`: `harness-compat-qa.log`, `harness-compat-qa-serial.log`, `harness-compat-targeted.log`, `harness-compat-sidecar-repro.log`, `harness-compat-clippy.log`, `harness-compat-production-check-final.log`.

### Historical delivery / readiness boundary

At that earlier handoff, two alternative patches were supplied: `harness-base.patch` targets HEAD `92127e6`; `harness-frozen-E.patch` targets HEAD `0165f71`. Apply only the matching variant after `git apply --check`; the latter preserves the already registered H test module and existing E/contracts. Those patches described then-uncommitted worktree changes: branching from the old HEAD alone would **not** inherit them. At that handoff the request to commit/push received no response, so neither was done. The reviewed H foundation has since been captured in incoming snapshot `31008d027297274723820a15d10097726707f12e`; the earlier no-commit statement is not the delivery status of this integration.

The literal ready-when criterion of other workstreams adopting their own tests is **not confirmed** by an H-owned consumer contract. Owners can use the demonstrated import without copying helpers; H intentionally does not rewrite their tests. Integration also still needs owner triage of the recorded aggregate-QA failures. No full downstream adoption/integration completion is announced.

## Evidence boundary / NOT TESTED

This foundation is a native **local-directory** Source acceptance path through the ordinary desktop transport, not an external HTTPS dependency acquisition test. An `.invalid` reserved host is intentionally bound by explicit local import and never treated as evidence of a working remote endpoint.

External HTTPS with an empty cache, real Eden/gameplay, physical SDL device release, v1.1 installation, Creator crash recovery/jobs, native UI and cross-volume hardware acceptance are **NOT TESTED by this harness**. The v1.0 fixture is a foundation for H2, not complete Creator v1 acceptance. See the integrator wave-zero handoff for actual commands and results.

All installation roots derive from one TempDir. No new cross-volume test or hard-link fallback is added: the integrator did not confirm a different-volume cause. Other owners' unit tests and production modules are not rewritten to force adoption; their future tests can use the demonstrated import above.
