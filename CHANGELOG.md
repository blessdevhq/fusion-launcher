# Changelog

## v0.4.0

Storage, sources, and artwork quality-of-life from tester feedback.

### Added

- Configurable **Library folder** for large content (emulators, ROMs, system files, temp). Defaults to `AppData\Local` instead of `Roaming`, with a folder picker in Settings. The database, logs, and config stay in `Roaming`; existing installs keep working with no migration.
- Unified **Add a source** flow: a single field accepts a community/personal repository or a game manifest (URL or pasted JSON) and auto-detects the format. Adding a source registers its catalog as browsable without installing anything.
- Automatic **cover-art backfill**: a background scrape fills in missing covers on launch and after a source is added (capped, skipping already-scraped games), shipping a default SteamGridDB key so it works out of the box.
- Downloads now **group an emulator/core bundle under the game** it was pulled in for, instead of showing two unrelated rows.
- The NSIS installer **bundles the Visual C++ Redistributable** and installs it on demand when missing, so the libtorrent download sidecar runs on a clean Windows.

### Changed

- First-run onboarding requires only a **connected source**; metadata and emulator setup moved to Settings (the emulator installs on demand when a game is installed), so nothing is downloaded just to get into the app.
- Release builds no longer open an extra **console window** alongside the app.

### Fixed

- Torrent sidecar failures now surface the real **stderr** cause (e.g. a missing runtime) instead of a bare "exited unexpectedly (code 1)".

### Notes

- The shipped SteamGridDB key is read-only and rate-limited; a user-supplied key (Settings) or a compile-time release key overrides it.

## v0.2.0

Automatic metadata and artwork scraping.

### Added

- ScreenScraper integration: hash-based (CRC32) and name-based matching to fill missing title, description, year, developer, publisher, genres, and cover art after imports. Uses the player's own ScreenScraper account, stored locally.
- One-pass ROM hashing (CRC32/MD5/SHA1/SHA256) with streaming progress, skipped for directory-based, `.chd`/`.cso`/`.cue`/`.gdi`/`.m3u`/`.pbp`/`.rvz`, and metadata-only titles.
- SteamGridDB artwork enrichment for hero and logo images, with an optional built-in app key and a per-user key fallback.
- Region and language preferences for scraped metadata and artwork selection.
- Ambiguous-match disambiguation and manual metadata override per game, with overrides that survive catalog refreshes.
- "Scrape entire library" batch job with a resumable, cancelable queue and progress reporting.
- Scraped metadata is cached locally with a 30-day TTL and merged under a strict priority: source catalog over scraped, manual override over both.
- Per-provider daily request budgets to respect ScreenScraper and SteamGridDB rate limits.

### Notes

- Scraped metadata and artwork are fetched directly from the provider to the user's machine and cached locally; Fusion Launcher does not proxy or redistribute them.
- ScreenScraper credentials and the optional SteamGridDB key are stored locally in app config; treat the machine profile as the trust boundary.
- Official builds may bake a SteamGridDB key and ScreenScraper developer credentials at compile time via release secrets; when absent, scraping falls back to per-user credentials.

## v0.1.0

Fusion Launcher MVP release candidate.

### Added

- Tauri-first Windows desktop launcher shell with local-first library storage.
- Bring-your-own-repository catalog preview, connection, refresh, and validation.
- Schema v3 rich metadata support for artwork, tags, genres, platform setup, and user-provided content.
- Automatic portable-emulator setup for NES/Mesen2, SNES/Mesen2, Nintendo 64/RMG, Game Boy Advance/mGBA, PlayStation 2/PCSX2, and PSP/PPSSPP.
- Manual executable and user-provided system-file setup for PlayStation 1 and Nintendo Switch.
- Game setup center with emulator, system file, game file, and launch readiness checks.
- Settings health reporting for repositories, downloads, game files, and platform setup.
- First-party NES smoke repository and package smoke harness without commercial ROM, BIOS, firmware, or key payloads.
- Windows RC gates for QA, preview smoke, visual smoke, Tauri build, NSIS artifact verification, and packaged binary smoke.
- GitHub Releases updater integration with updater-artifact signing for tagged Windows releases.

### Notes

- Only updater signing is required for this RC. Windows Authenticode signing is deferred production hardening.
- The packaged smoke harness verifies the built binary, NSIS artifact presence, clean app data setup, and profile readiness transitions. It does not automate the NSIS installer UI or uninstall flow.
- Users must provide their own legally obtained commercial games, BIOS files, firmware, and keys.
