# Source Library Template

Fusion Launcher source libraries are JSON catalogs that users can connect by URL or
local file. They are intended for bring-your-own-repository workflows: a person
or community can maintain a source library, while Fusion Launcher validates and
loads the format.

## Public Starter URL

The Fusion Launcher project publishes the canonical starter template through GitHub
Pages:

```text
https://blessdevhq.github.io/fusion-launcher/source-library-template/repository.json
```

Use this URL to preview the template in Fusion Launcher, or copy
`templates/source-library` to create a source library with your own metadata and
entries.

## Create a Public Library

1. Copy the contents of `templates/source-library` into a new GitHub repository,
   including the `.github/workflows` folder.
2. Edit `repository.json` metadata, catalog entries, and system file entries.
3. Keep `blessdevhq/fusion-launcher` in the copied workflow files as the
   Fusion Launcher validator repository, unless you maintain a validator fork.
4. In GitHub, open the new repository settings and set Pages build and
   deployment to GitHub Actions.
5. Push to `main`, then let `Publish Fusion Launcher source library` deploy Pages.
6. Paste `https://<owner>.github.io/<repo>/repository.json` into Fusion Launcher
   Settings > Sources.

If you copy the template workflow into a standalone repository, keep
`blessdevhq/fusion-launcher` as the validator repository unless you maintain a
validator fork.

## Create a Private or Local Library

For personal libraries, keep `repository.json` on disk or behind a private HTTPS
endpoint. In the desktop app, use the JSON import button to connect a local
file. Private libraries are still validated before Fusion Launcher stores them.

## Content Modes

- `downloadable`: use for content you can legally distribute. `http` sources
  require SHA-256 hashes. `magnet` sources are allowed as a transport, but the
  library author is responsible for what the torrent distributes.
- `user_provided`: use for ROMs, disc images, BIOS files, firmware, keys, and
  other files that users must supply themselves. Fusion Launcher shows Import
  instead of Download for these entries.
- `metadata_only`: use for catalog records, compatibility notes, and setup
  hints without a payload source.

## Magnet Entries

Magnet sources should describe a specific release that the source library is
allowed to share. Include `infoHash` and `sizeBytes` when known. Do not use the
official template or docs to publish links to content that the source library
cannot redistribute.

Example shape:

```json
{
  "kind": "magnet",
  "uri": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=template-homebrew.gba",
  "infoHash": "0123456789abcdef0123456789abcdef01234567",
  "sizeBytes": 131072
}
```

## Validation

Run:

```powershell
npm run source:validate -- templates/source-library/repository.json
```

The validator checks schema shape, duplicate ids, empty strings, bundled source
usage, and risky system file download sources. It does not download payloads,
inspect torrent contents, or make legal claims on behalf of the source author.

Fusion Launcher uses `trustLevel` for labeling. A copied template defaults to
`community`, so the app shows it as a community source after preview and
connection. Fusion Launcher does not curate community payloads or decide which
community sources users should trust.
