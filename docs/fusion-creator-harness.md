# Creator foundation H

Test-only module: `src-tauri/src/fusion_source/creator_harness_tests.rs`.
Integrator adds its `#[cfg(test)] mod creator_harness_tests;` registration to `fusion_source/mod.rs`.

## What is exercised

- Existing `PackerEngine` builds two synthetic owned publications; no commercial game or emulator is distributed.
- Existing `LocalSourceCandidate` explicitly grants desktop-owned local-directory authority. Existing `DesktopSourceTransport` and the real `FusionService` perform acquisition and installation; this harness implements no transport trait and no alternate installer.
- Each fixture has its own temporary SQLite database, library/CAS, appdata, local-appdata, player input and external workspace root.
- Full `InstallRequest` supplies `user_files`, `raw_paths` and `external_workspace_root` explicitly. Local-locator reference and player-supplied raw input exercise distinct provenance paths.
- Author directory is removed before installation/launch. Published content and player-provided requirements are separate. Publication is signed with a fixed explicitly public TEST key; no real private key file is opened.
- Native Windows fixture reuses `supervisor::tests::native::native_process_fixture` by copying the current test executable and, when present, its external activation manifest. `game_launch` supervises a real child process and waits for exit/save reconciliation.
- First launch without executable approval must fail. Explicit per-file/per-hash approval is separate from the Source signature/TOFU pin.
- Release two changes the runtime's exact hash through a test-owned PE overlay. The scenario installs, launches, updates, requires fresh approval, launches again, rolls back, retains the newest save and launches the restored runtime. No game payload is used.
- `pub(super) CreatorHarness` is available to sibling test modules so later streams can extend the common fixture rather than build another installer. It is excluded from production builds by module registration.

## Baseline adaptation, not a production feature

The baseline Packer cannot author user_provided recipe components. After Packer verifies its original artifact set, this TEST helper augments the existing 1.0 wire manifest with a player-supplied layer, writes new digest-named bytes, updates the immutable index URL/digest and signs that exact index. It does not claim Packer verified the augmented publication or implement the future Creator recipe compiler.

Saves use an explicit directory declaration (`workspace/saves`), matching the existing supervisor fixture's parent-directory atomic save behavior. Raw archive destinations, including the optional activation manifest, are supplied explicitly; filenames are never inferred from an untrusted URL. These are fixture configuration requirements, not hard-link fallback changes.

## Reproduce

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib fusion_source::creator_harness_tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib local_sources::tests::desktop_local_import_restart_launch_update_and_rollback -- --exact --nocapture
npm run qa
```

On non-Windows, native launch scenario is compiled out; this is not a Windows PASS. Use default native toolchain environment consistently when sharing a Cargo cache across worktrees; changing MSVC environment between builds can rebuild all native dependencies.

## Evidence boundary / NOT TESTED

This foundation is a native **local-directory** Source acceptance path through the ordinary desktop transport, not an external HTTPS dependency acquisition test. An `.invalid` reserved host is intentionally bound by explicit local import and never treated as evidence of a working remote endpoint.

External HTTPS with an empty cache, real Eden/gameplay, physical SDL device release, v1.1 installation, Creator crash recovery/jobs, native UI and cross-volume hardware acceptance are **NOT TESTED by this harness**. The v1.0 fixture is a foundation for H2, not complete Creator v1 acceptance. See the integrator wave-zero handoff for actual commands and results.
