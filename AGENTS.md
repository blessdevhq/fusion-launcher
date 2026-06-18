# Fusion Launcher Project Guide

Windows-first desktop launcher and emulator-ready retro game library manager, licensed GPL-3.0-or-later.
The canonical product name is **Fusion Launcher** and the canonical repository is
`https://github.com/blessdevhq/fusion-launcher`.

## Stack

- **Frontend:** Next.js 15 static export to `out/`, React 19, TypeScript, Tailwind 3,
  Zustand (`src/stores/launcherStore.ts`), Zod, framer-motion, lucide-react.
- **Backend:** Tauri 2 + Rust (`src-tauri/src`), rusqlite with bundled SQLite,
  reqwest with rustls, tokio. P2P downloads run in a standalone libtorrent
  sidecar (`sidecar/`), spawned over stdio.
- **Tests:** Node `--test` with `--experimental-strip-types` for TypeScript,
  Playwright for UI, and `cargo test` for Rust.

## Commands

- `npm run dev`: Next dev server.
- `npm run tauri:dev`: full desktop app.
- `npm run check`: typecheck, node tests, static check, and frontend build.
- `npm run qa`: `check`, Rust tests, Rust formatting check, and clippy.
- `npm run tauri:build`: production app.
- `npm run mvp:smoke` / `npm run mvp:visual`: release smoke checks.

## Architecture Notes

- Emulator registry data is intentionally duplicated across platform types,
  setup profiles, bundled profile JSON, security allowlists, and install flows.
  Adding a platform must update all of them together.
- Downloads must stream to disk, never buffer full archives into memory.
  Incremental SHA-256 and torrent streaming paths are required for large files.
- Catalogs can be large. `get_catalog` uses batched enrichment, and the Dashboard
  uses progressive rendering because gamepad focus reads DOM rects by
  `data-focus-id`.
- Storage writes should prefer transactions; direct-download flows can otherwise
  leave partial records.

## Legal Model

Fusion Launcher ships no commercial ROMs, BIOS files, firmware, keys, or
commercial game payloads. Users provide their own files. Auto-download support is
gated by source policy and emulator licensing.

## Layout

- `src/app`: Next app entrypoints.
- `src/components`, `src/core`, `src/stores`, `src/lib`, `src/types`: frontend and shared TypeScript.
- `src-tauri/src/commands`: Tauri command modules.
- `src-tauri/src/storage`: SQLite store modules.
- `src-tauri/src`: Rust orchestration, downloads, scrapers, torrent, security, and setup profiles.
- `docs/`: release, authoring, source library, and install documentation.
