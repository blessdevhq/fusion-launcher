# RetroHydra

RetroHydra is a Windows-first desktop MVP for managing emulator-ready game
libraries from bring-your-own-repository metadata. It is built with Next.js,
Tauri 2, Rust, and local-first storage.

The MVP focuses on:

- repository preview, connection, refresh, and catalog validation;
- emulator path setup and launch preflight checks;
- direct, bundled, and torrent-aware download state tracking;
- a built-in first-party NES smoke-test repository for validation without
  commercial content;
- diagnostics, health checks, and GitHub Releases updater integration.

## First User Path

The public MVP supports a one-pass playable demo setup on Windows:

1. Install RetroHydra.
2. Click **Set up demo** on first launch.
3. RetroHydra connects the built-in demo repository, installs pinned Mesen2
   `2.1.1`, downloads the first-party NES smoke ROM, and enables **Play Demo**.

Only the NES demo path is automatic in this MVP. Other emulator platforms still
use manual executable selection.

## Legal content model

RetroHydra does not ship commercial ROMs, BIOS files, firmware, keys, or
third-party game payloads. Users are expected to provide only content they are
legally allowed to use.

The bundled `public/demo-content/retrohydra-smoke.nes` asset is first-party
RetroHydra smoke-test content and is documented in
`public/demo-content/LICENSE.txt`.

## Development

Prerequisites:

- Node.js 22
- Rust stable
- Windows build tools for Tauri desktop builds

Install and check the project:

```powershell
npm ci
npm run check
cargo test --manifest-path src-tauri/Cargo.toml
```

Run the web shell:

```powershell
npm run dev
```

Build the Windows desktop app:

```powershell
npm run tauri:build
```

When updater artifacts are enabled, Tauri requires `TAURI_SIGNING_PRIVATE_KEY`
and optionally `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to sign updater packages.
The GitHub Actions release workflow expects these secrets to be configured in
the repository before publishing a tagged release. The private key must never be
committed.

## Release

The Windows release workflow runs on tags matching `vX.Y.Z` and uploads:

- NSIS installer;
- updater zip;
- updater signature;
- `latest.json` for the Tauri updater.

See `docs/mvp-windows-install.md` for the internal MVP install checklist.

## License

No source license has been granted yet. The repository is public for MVP review
and validation unless a separate license file is added. Demo smoke-test content
is covered separately in `public/demo-content/LICENSE.txt`.
