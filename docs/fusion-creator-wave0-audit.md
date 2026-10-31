# Creator wave-zero: baseline audit and discrepancies

Baseline: `creator-base` / `287dc370b891577c5c5ca27ac9126766daf89920`.
This audit distinguishes existing code from planned v1 work. File references describe the baseline; subsequent contract additions move line numbers.

## Blocking discrepancy resolved before implementation

The requested `creator-base` did not exist in local branches, remote-tracking refs or `git ls-remote --heads origin creator-base creator/*`. The original `wip/lifecycle-network` worktree contained required uncommitted source changes. Creating a worktree at its old HEAD would omit them.

After the user's approval to act autonomously, an isolated worktree was created and a snapshot committed as `287dc37`. It includes tracked changes plus their untracked source/test dependencies, the desktop harness sources and the revised plan. It excludes `live-source/` (including private keys and payload), `docs/totk-codex/` local evidence and `tools/cloudflared/`. The original worktree/index were not staged, reset, switched or committed. `docs/fusion-creator-v1-plan.md` is the preserved revised plan; the old original plan is not authoritative.

## Fact check

| Claim | Baseline evidence | Finding |
| --- | --- | --- |
| SQLite version 5 | `src-tauri/src/storage/migrations.rs:280-287`, `storage/tests.rs:854-863` | CONFIRMED. The local-directory-source migration already occupies 5. C owns the single next migration (6 if still free at integration), including projects/private bindings/jobs/releases/reports. G must not create a competing migration. No migration is added in wave zero. |
| Packer reusable as library | `fusion_source/packer.rs:299-358`, `:849`; CLI `src/bin/fusion_packer.rs:201-224` | CONFIRMED. PackerEngine has build/inspect/verify entry points. It is synchronous; build has no progress/cancel/checkpoint API. C/G must add a blocking-worker integration, not pretend the current build is resumable. |
| FusionService | `fusion_source/service.rs:185`, `:254-306`, `:1420-1434` | CONFIRMED. Existing service owns install/update/launch operations and injected storage/transport/progress. Do not introduce a second installer. |
| CAS | `fusion_source/cas.rs:43-115`; `fusion_source/mod.rs:8-9` | CONFIRMED. Immutable, inventory-addressed CAS with private staging and streaming copies/hashing. Writable files must be detached copies; hard links are not COW. |
| Snapshots | `fusion_source/workspace.rs:31-51`; `service_tests.rs:465-545` | CONFIRMED. Existing immutable generation snapshots and persisted active ownership are reusable. Creator project/release/checkpoint registries do not exist yet. |
| Native supervisor | `fusion_source/supervisor.rs:1-4,29-31`; `local_sources_tests.rs:60-79,83-156` | CONFIRMED. Windows Job Object process ownership and real supervised child tests already exist. |
| Ed25519 + TOFU | `fusion_source/signatures.rs:21-37,128-170`; `service_tests.rs:31-104` | CONFIRMED. First-use pin is retained after source removal, replacement/stripping is rejected. Source trust is distinct from per-path/per-hash executable approval. |
| Protected signing-key file | `fusion_source/signatures.rs:308-343,383-436`; CLI `fusion_packer.rs:216-221` | CONFIRMED, with qualification: existing API is a caller-supplied PKCS#8 PEM file, not a Creator key registry or credential vault. Windows keygen uses a protected DACL from file creation; Unix uses mode 0600. Reuse this protection, do not claim DPAPI/key rotation exists. |
| Existing helpers | `fusion_source/service_tests.rs:169-313`, `service_host_tests.rs:21-40,102-159`, `local_sources_tests.rs:20-156`, `tests/desktop/` | CONFIRMED, but different evidence scopes. Memory/injected transport is not external HTTPS acquisition; offline source tests are not ordinary desktop-transport tests. The existing desktop-local lifecycle really launches a child. |

## Additional gaps and non-equivalences

1. **The named service lifecycle does not launch.** `install_exact_trust_runtime_update_and_rollback_keep_latest_saves` checks trust/readiness, install/update/rollback/saves, but contains no `game_launch` call. It cannot alone establish the requested full lifecycle. Run the separate native desktop lifecycle and the foundation H scenario.
2. **The alleged hard-link Access denied has no proven root cause in the prompt.** The service lifecycle passed on this local Windows host. No fallback/cross-volume copy is justified from that allegation. A native launch failure, if found, must be diagnosed at its actual failing operation.
3. **Baseline parsing is 1.0 only.** `schema.rs:9`, `evolution.rs:220-238,270-275` reject 1.1. Wave zero adds typed reading of 1.0/1.1, keeps SourceIndex at 1.0 and explicitly disables unimplemented 1.1 execution; A/B own the later behavioral implementation.
4. **No shared pure winning-provider resolver exists yet.** `workspace.rs:434-542` projects ordered inventories and replaces earlier projected files. Dependency order is not an explicit author conflict decision. Wave zero freezes the pure function signature, not an alternate production resolver.
5. **Installer interruption is not Creator resume.** `storage/fusion_source.rs:578-589` marks operations interrupted and prevents terminal revival, but does not persist/revalidate Creator revision, input hashes, adapter versions or artifact/release/report checkpoints. `packer.rs:2097-2113` scratch cleanup is not recovery.
6. **Existing Eden local preparation is not the full portable adapter D.** `local_sources_tests.rs:36,50-52,124-126` covers a host preparation hook and whole-user persistence. It does not prove the v1 allowlist, no-private-export guarantee, game-specific import, profile application or SDL ParamPackage mapping.
7. **Documentation identity differs.** AGENTS.md names `blessdevhq/fusion-launcher`; CLAUDE.md still names `MrBeastie/fusion-launcher` and an older legal model. AGENTS.md is used as the current contract; no unrelated legal/documentation rewrite is included.

## Environment observations

- Native local Windows terminal is used; no sandbox permission failure was reproduced by the service lifecycle.
- `cmake` is absent from ordinary PATH, but exists under Visual Studio BuildTools' CMake directory; the isolated SDL spike must use that executable, not claim that SDL cannot build.
- `npm ci --no-audit --no-fund` failed with `EIDLETIMEOUT` contacting registry.npmjs.org. The integrator uses a directory junction to the already installed dependency tree for checks; package/lock files are unchanged. This is not evidence of a fresh reproducible dependency install.

Actual acceptance results and NOT TESTED items are recorded in the wave-zero handoff, not inferred from this static audit.
