# Fusion Launcher

[Русская версия](README.ru.md)

Fusion Launcher is a Windows-first desktop launcher for emulator-ready game
libraries. It connects bring-your-own repositories, validates catalog metadata,
tracks installs, and launches games through user-configured or automatically
prepared emulators.

The project is built with Next.js, Tauri 2, Rust, and local-first storage.

## What It Does

- Connects source-library JSON catalogs from GitHub Pages, HTTPS URLs, or local
  files.
- Validates schema v3 catalogs with rich metadata, artwork, tags, genres, setup
  profiles, and install requirements.
- Supports a first-party built-in NES smoke-test repository for safe setup and
  release validation.
- Tracks direct, bundled, and torrent-aware download state.
- Imports user-provided games, BIOS, firmware, and keys without bundling
  commercial payloads.
- Resolves metadata and artwork from source libraries, ScreenScraper, and
  SteamGridDB when configured.
- Runs launch preflight checks before starting emulator processes.
- Provides diagnostics, health checks, GitHub Releases update checks, and
  Windows package smoke tests.

## First Run

The fastest path on Windows is the built-in demo setup:

1. Install Fusion Launcher.
2. Open the app and choose **Set up demo**.
3. Fusion Launcher connects the built-in demo source, prepares a supported NES
   emulator, installs the first-party smoke ROM, and enables **Play Demo**.

Automatic portable-emulator setup is available for NES, SNES, Nintendo 64, Game
Boy Advance, PlayStation 2, and PSP. PlayStation 1 and Nintendo Switch currently
use manual executable selection. Platform-owned BIOS, firmware, and keys always
remain user-provided.

## Content Model

Fusion Launcher does not ship commercial ROMs, BIOS files, firmware, keys, or
third-party game payloads. Users and source-library authors are responsible for
using only content they are legally allowed to use.

The bundled `public/demo-content/fusion-launcher-smoke.nes` file is first-party
smoke-test content for launcher validation. Its terms are documented in
`public/demo-content/LICENSE.txt`.

## Source Libraries

Source libraries are JSON catalogs that describe games, platforms, metadata,
artwork, setup profiles, and install requirements.

Useful entry points:

- [Source library template](docs/source-library-template.md)
- [Repository authoring guide](docs/repository-authoring.md)
- [Rich metadata example](examples/repositories/showcase.metadata.json)

The starter template is published at:

```text
https://mrbeastie.github.io/fusion-launcher/source-library-template/repository.json
```

## Development

Prerequisites:

- Node.js 22
- Rust stable
- Windows build tools for Tauri desktop builds

Install dependencies:

```powershell
npm ci
```

Run the standard checks:

```powershell
npm test
npm run typecheck
npm run static-check
npm run source:template:check
npm run rust:test
```

Run the full local QA gate:

```powershell
npm run qa
```

Run the web shell:

```powershell
npm run dev
```

Build the Windows desktop app:

```powershell
npm run tauri:build
```

Validate a source library:

```powershell
npm run source:validate -- templates/source-library/repository.json
```

## Release

The Windows release workflow runs on tags matching `vX.Y.Z` and uploads:

- NSIS installer
- updater zip
- updater signature
- `latest.json` for the Tauri updater

Release builds require `TAURI_SIGNING_PRIVATE_KEY`; set
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when the private key is encrypted.

Run the Windows release smoke gate locally:

```powershell
npm run mvp:release:windows
```

Related docs:

- [MVP Windows install checklist](docs/mvp-windows-install.md)
- [Release checklist](docs/release-checklist.md)
- [Metadata and artwork notes](docs/metadata-artwork.md)

## Compatibility Notes

The project was previously named RetroHydra. New installs use Fusion Launcher
names, identifiers, and files. Legacy `RETROHYDRA_*` environment variables,
database names, and built-in demo identifiers are still supported as fallbacks
for existing installs and CI setups.

## License

No source license has been granted yet. The repository is public for MVP review
and validation unless a separate license file is added. Demo smoke-test content
is covered separately in `public/demo-content/LICENSE.txt`.
