# Fusion Launcher

[Русская версия](README.ru.md)

![Fusion Launcher key visual](public/fusion/hero-key-visual.png)

**Status:** public MVP preview for Windows; fresh Fusion-branded installer
release is being prepared.
**Repository:** [MrBeastie/fusion-launcher](https://github.com/MrBeastie/fusion-launcher)

Fusion Launcher is a source-driven Windows launcher for retro and console games
on PC. It combines source-library browsing and install tracking with guided
portable-emulator setup: connect a source, install content you are allowed to
use, satisfy emulator requirements, and launch everything from one desktop app.

Fusion Launcher does not host, curate, or distribute commercial games, BIOS
files, firmware, keys, or third-party payloads. Users are responsible for the
sources and content they connect.

The project is built with Next.js, Tauri 2, Rust, and local-first storage.

## What It Does

- Connects community, personal, or local source-library JSON catalogs from
  GitHub Pages, HTTPS URLs, or local files.
- Presents retro and console game catalogs in one Windows desktop launcher.
- Tracks direct, bundled, and torrent-aware install state for content described
  by connected sources.
- Validates schema v3 catalogs with rich metadata, artwork, tags, genres, setup
  profiles, and install requirements.
- Helps prepare supported portable emulators and stores user-selected emulator,
  BIOS, firmware, key, and game-file paths locally.
- Resolves metadata and artwork from source libraries, ScreenScraper, and
  SteamGridDB when configured.
- Runs launch preflight checks before starting emulator processes.
- Provides a first-party NES smoke-test repository, diagnostics, health checks,
  GitHub Releases update checks, and Windows package smoke tests.

## First Run

The fastest path on Windows is the built-in demo setup:

1. Install Fusion Launcher.
2. Open the app and choose **Set up demo**.
3. Fusion Launcher connects the built-in demo source, prepares a supported NES
   emulator, installs the first-party demo cartridge image, and enables
   **Play Demo**.

Automatic portable-emulator setup is available for NES, SNES, Nintendo 64, Game
Boy Advance, PlayStation 2, and PSP. PlayStation 1 and Nintendo Switch currently
use manual executable selection. Platform-owned BIOS, firmware, and keys always
remain user-provided.

## Content Model

Fusion Launcher is content-neutral launcher infrastructure. It connects to the
sources users choose, tracks files described by those sources, and launches
content through configured emulator profiles.

The Fusion Launcher project, repository, and official releases do not host or
ship commercial ROMs, BIOS files, firmware, keys, or third-party game payloads.
Users and source-library authors are responsible for the sources and content
they connect. Fusion Launcher is not affiliated with emulator projects, game
publishers, or console manufacturers.

The bundled `public/demo-content/fusion-launcher-smoke.nes` file is a
first-party demo cartridge image for launcher validation. Its terms are
documented in
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

New installs use Fusion Launcher names, identifiers, and files. Legacy preview
app identifiers, database names, and demo identifiers are still supported as
fallbacks for existing preview installs and CI setups.

## License

Fusion Launcher source code is licensed under the GNU General Public License
v3.0 or later. See [LICENSE](LICENSE).

Bundled demo smoke-test content is covered separately in
`public/demo-content/LICENSE.txt`.
