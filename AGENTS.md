# Fusion Project Guide

**Fusion** is an open, Windows-first platform for distributing, installing, updating, and running layered software builds and packages on the open, domain-neutral **Fusion Source v1.0** standard, licensed under GPL-3.0-or-later.
The canonical repository is `https://github.com/blessdevhq/fusion-launcher`.

## Positioning & Product Modes

Fusion operates user and author workflows inside one desktop application over a single open Rust engine (`src-tauri/src/fusion_source/`, being extracted to `fusion-source-core`):

- **Fusion Launcher** (user mode): connects direct `fusion://` / HTTPS / local Sources, installs layered packages via an immutable BLAKE3 Content-Addressed Store (CAS), keeps user data/saves isolated outside replaceable workspace snapshots (with pre-activation and manual local save snapshots), enforces explicit per-binary hash trust before execution, supports atomic version rollback, and supervises launched process trees.
- **Fusion Creator v1** (built-in author workflow, currently implemented in `src-tauri/src/fusion_creator/` + `src/components/creator/`): guides authors through `Base -> Composition -> Profiles -> Verification -> Release/Export -> Verify HTTPS` over `ProgramDescriptor 1.0`, `GameManifest 1.1` Source-relative mirrors, isolated clean-install verification (`CreatorVerifyPanel`), allowlisted export + live HTTPS acquisition check (`CreatorReleasePanel`), and local SDL GameController calibration/binding (`CreatorDeviceDialog`) guarded by a pre-launch SDL release barrier.
- **Fusion Studio** (target Stage 6 evolution of author mode): aligns author workflows strictly as a GUI wrapper over the extracted `fusion-source-core` API and unified CLI (`fusion init / pack / test / publish`) — never hidden or proprietary protocol logic.

**Initial market wedge:** fan-made builds, PC ports, recompilations, translations, and modpacks layered non-destructively over users' own games (Steam/GOG/Epic installations or user-owned dumps). Architecturally, the protocol and core engine are domain-neutral from day one and equally support portable non-game tools and AI bundles.

## Non-Negotiable Architecture Principles

1. **Single Open Core (`fusion-source-core`):** Fusion Launcher, Fusion Studio, the CLI (`fusion-packer`, `fusion-local`, and the upcoming unified `fusion` CLI), and third-party clients share one Rust core crate.
2. **Domain-Neutral Fusion Source v1.0 Contract:** The current `"1.0"` schema in `src-tauri/src/fusion_source/schema.rs` is an unpublished internal draft being finalized to domain-neutral v1.0 (`SourceIndex.packages`, `SourcePackageEntry`, `PackageManifest`, `QualifiedPackageId` / `package_id`, `PersistenceSpec.data_paths`, optional `metadata` instead of mandatory `platform`), while keeping BLAKE3 CAS, Ed25519 signatures, `capabilities` + `fallback` (`capabilities.rs`), and `SchemaUpcaster` (`evolution.rs`).
3. **Legacy Deprecation Rule:** The legacy retro-catalog / emulator-profile pipeline (`src-tauri/src/schema.rs`, `repository*`, `setup_profiles.rs`, `emulator_profiles.rs`, `manifest.rs`, `orchestrator.rs`, `GameDetailsModal.tsx`, legacy `launcherStore.ts` flows) is **DEPRECATED and scheduled for full removal in Stage 2**. Do NOT add new features to Legacy or expand the legacy emulator registry.
4. **Zero Vendor Lock-in:** HTTPS, GitHub Releases, mirrors, local directories, and BitTorrent (libtorrent sidecar) are equal transports. There is no central monopoly registry; starter/demo Sources exist only for onboarding.
5. **100% Legal Model:** Fusion ships no commercial games, ROMs, BIOS files, firmware, keys, or third-party payloads. Users provide their own files. `Bios`, `Firmware`, and `Keys` layers must always be `user_provided`.
6. **Security & Data Invariants:**
   - Stream downloads to disk; never buffer full archives into memory. Incremental hashing and bounded streaming are required.
   - CAS objects are read-only and immutable. Mutable paths (`writable_copy_paths`), patch outputs, and generated configs must be materialized as separate physical copies — never modify permissions through a hardlink.
   - Executable trust is explicit per exact BLAKE3 digest (`critical_files`); source metadata never grants execution trust automatically.
   - Ed25519 signing keys require explicit UX explaining where the key lives and how to back it up.
   - SQLite storage writes must use transactions.

## Active 6-Stage Roadmap

- **Stage 1 (Next):** Extract `fusion-source-core` into a Cargo workspace crate + finalize domain-neutral `v1.0` wire/canonical contracts (while aligning `fusion_creator` on `fusion-source-core`).
- **Stage 2:** Audit technical dependencies on Legacy modules (transports, hashers, UI/stores) and completely remove the old retro-launcher engine and duplicate UI.
- **Stage 3:** Core completeness (`Save / Data Restore`, `Easy Setup`, reliable install/update/rollback/error recovery).
- **Stage 4:** Unified CLI (`fusion init`, `fusion pack`, `fusion test`, `fusion publish`) over `fusion-source-core`.
- **Stage 5:** 3 open reference Sources (portable non-game/AI tool → modpack over Steam/GOG/Epic → fan PC port/recompilation).
- **Stage 6:** Evolve built-in **Fusion Creator v1** into **Fusion Studio** on top of the extracted shared core and unified CLI.

## Stack

- **Frontend:** Next.js 15 static export to `out/`, React 19, TypeScript, Tailwind 3, Zustand (`src/stores/fusionSourceStore.ts`, `src/stores/creatorStore.ts` + transitional `src/stores/launcherStore.ts`), Zod, framer-motion, lucide-react.
- **Backend:** Tauri 2 + Rust (`src-tauri/src`), `fusion_source` engine (`src-tauri/src/fusion_source`), `fusion_creator` engine + bundled `sdl2` input service (`src-tauri/src/fusion_creator`), rusqlite with bundled SQLite (`src-tauri/src/storage/fusion_source.rs`), reqwest with rustls, tokio. P2P downloads run in a standalone Python/libtorrent sidecar (`sidecar/torrent_sidecar.py`) spawned over stdio and bundled via `tauri.sidecar.conf.json`.
- **Tests:** Node `--test` with `--experimental-strip-types` for TypeScript, Playwright for UI, and `cargo test` for Rust (with CMake 3.31.x for bundled `sdl2`, physical `FUSION_TEST_PYTHON`, and `--test-threads=4`).

## Commands

- `npm run dev`: Next dev server.
- `npm run tauri:dev`: full desktop app.
- `npm run check`: typecheck, node tests, static check, and frontend build.
- `npm run qa`: `check`, Rust tests, Rust formatting check, and clippy.
- `npm run tauri:build`: production app.
- `npm run mvp:smoke` / `npm run mvp:visual`: release smoke checks.
- `cargo build --manifest-path src-tauri/Cargo.toml --bin fusion-packer --bin fusion-local`: build standalone authoring/offline CLI binaries.

## Layout

- `src/app`: Next app entrypoints.
- `src/components`, `src/core`, `src/stores`, `src/lib`, `src/types`: frontend and shared TypeScript (`fusionSource.ts`, `fusionSourceStore.ts`, `creatorStore.ts`, `components/creator/`).
- `src-tauri/src/fusion_source/`: Fusion Source v1.0 core engine (schema, canonical, capabilities, evolution, cas, layers, persistence, plan, resolver, workspace, supervisor, telemetry, packer, service, offline).
- `src-tauri/src/fusion_creator/`: built-in Fusion Creator v1 engine (declarative `ProgramDescriptor 1.0`, `GameManifest 1.1` build/export, clean-install Verify, Verify HTTPS, SDL input service & launch barrier).
- `src-tauri/src/commands/`: Tauri command modules (`fusion_source.rs`, `fusion_creator.rs` + legacy commands).
- `src-tauri/src/storage/`: SQLite store and migrations (`fusion_source.rs`).
- `src-tauri/src/bin/`: standalone `fusion-packer` and `fusion-local` CLI binaries.
- `sidecar/`: Python/libtorrent sidecar for BitTorrent transport.
- `docs/`: Fusion Source v1.0 spec (`fusion-source-standard-v1.md`), Creator guide (`fusion-creator-guide.md`), CLI docs (`fusion-packer.md`, `fusion-local.md`), release and install docs.

## Visual OS

For UI work, read the relevant `design/` contract and `.agents/visual-os/` guidance before editing.

- `src/app/globals.css` and `tailwind.config.js` are the canonical visual-token layer; reuse `--rh-*` and `fusion-*` roles before adding raw visual literals.
- Search existing shell, dashboard, modal, focus and `data-testid` patterns before creating UI primitives or navigation behavior.
- Treat external screenshots/Figma as intent only: do not copy protected assets, CSS, copy, logos or distinctive trade dress.
- For covered launcher flows, run `npm run mvp:visual`; inspect supported desktop states and report intended visual diffs.
- Do not autonomously approve new shared visual roles, canonical primitives or future screenshot baselines.
