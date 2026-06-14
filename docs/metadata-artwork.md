# Metadata Artwork Enrichment

Fusion Launcher can enrich installed games with metadata and artwork in two layers:

1. ScreenScraper remains the primary metadata source for game descriptions, covers, release data, and match state.
2. SteamGridDB is an optional artwork-only enrichment layer for hero images, logos, and grid artwork.

SteamGridDB artwork is cached in the existing `metadata_cache` table with `provider = 'steamgriddb'` and a prefixed `rom_key` (`sgdb:{game_id}`). No extra provider artwork table is required for this phase.

## SteamGridDB API keys

The app resolves the SteamGridDB key in this order:

1. User-saved key from `steamgriddb.api_key`.
2. Built-in compile-time key from `FUSION_LAUNCHER_STEAMGRIDDB_KEY`.
3. No key, in which case SteamGridDB enrichment is skipped.

Leaving the settings field blank clears the user key and lets official builds use the built-in key when present.

## Batch scraping

The "Scrape entire library" action reuses `scrape_state` as a persistent queue. Installed games are marked `pending`, then a single background worker drains the queue sequentially. The worker emits:

```json
{
  "done": 0,
  "total": 0,
  "currentGameId": null
}
```

The app does not auto-resume pending metadata work at startup. This avoids spending daily provider limits without the user's explicit action.

## Legal note

Fusion Launcher does not proxy SteamGridDB artwork. Requests go from the user's local client to SteamGridDB, then artwork URLs are cached locally on the user's machine. This preserves the zero-responsibility model for both user-provided and built-in API keys.
