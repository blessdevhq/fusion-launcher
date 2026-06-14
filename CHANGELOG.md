# Changelog

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
