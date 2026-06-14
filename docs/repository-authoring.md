# Fusion Launcher Repository Authoring

Fusion Launcher repositories describe catalogs, setup requirements, and safe source
metadata. Official examples must not include commercial ROM, ISO, NSP, XCI,
BIOS, firmware, key, or magnet payload links.

## Schema Versions

`schemaVersion: 2` remains supported. `schemaVersion: 3` adds rich catalog
metadata for launcher-style browsing:

- `artwork.cover`, `artwork.hero`, `artwork.logo`, and `artwork.screenshots`
- `metadata.releaseYear`, `developer`, `publisher`, `genres`, `tags`,
  `players`, `series`, and `externalIds`
- `contentMode`: `downloadable`, `user_provided`, or `metadata_only`
- `setupProfileId`: optional built-in setup profile such as `nes-mesen`,
  `snes-mesen`, `n64-rmg`, `gba-mgba`, `ps2-pcsx2`, `psp-ppsspp`,
  `ps1-manual`, or `switch-manual`; unknown IDs stay visible but show an
  unsupported setup state

## Content Modes

Use `downloadable` only for content that the repository is legally allowed to
distribute and that includes required hashes. Use `user_provided` when the user
must import their own local game, BIOS, firmware, or keys. Use `metadata_only`
for catalog-only entries.

Commercial-console-style entries in official examples must use
`user_provided` or `metadata_only`; they must not include HTTP or magnet payload
links.

## Artwork

Artwork fields are URLs. The app renders them when available and falls back to
generated art when a URL is missing or fails to load. Do not commit copyrighted
artwork into this repository unless its license explicitly permits that use.

See `examples/repositories/showcase.metadata.json` for a safe fictional
metadata source.
