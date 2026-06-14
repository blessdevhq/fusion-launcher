# Fusion Launcher Source Library Template

This directory is a starter source library for Fusion Launcher. Copy it into a new
repository, edit `repository.json`, host the JSON file, and connect that URL or
local file in Fusion Launcher.

The canonical starter template is published by Fusion Launcher at:

```text
https://mrbeastie.github.io/fusion-launcher/source-library-template/repository.json
```

## Quick Start

1. Copy this directory into a new GitHub repository, including the
   `.github/workflows` folder.
2. Replace the metadata in `repository.json`.
3. Replace the sample catalog entries with your own entries.
4. Replace `YOUR_ORG_OR_USER/fusion-launcher` in the copied workflows with the
   Fusion Launcher validator repository, usually `MrBeastie/fusion-launcher`.
5. Run the validator from the Fusion Launcher project:

```powershell
npm run source:validate -- templates/source-library/repository.json
```

For a standalone source repository, keep the same command in CI after checking
out the Fusion Launcher validator, as shown in `.github/workflows/validate.yml`.

## Publish with GitHub Pages

1. In the new source repository, open Settings > Pages.
2. Set Build and deployment to GitHub Actions.
3. Push to `main` or run `Publish Fusion Launcher source library` manually.
4. Paste this URL into Fusion Launcher Settings > Sources:

```text
https://<owner>.github.io/<repo>/repository.json
```

## Source Modes

- `downloadable`: content you are allowed to distribute, using `http` or
  `magnet` sources.
- `user_provided`: content the user must import locally, such as ROMs, disc
  images, BIOS files, firmware, and keys.
- `metadata_only`: catalog metadata and setup hints without payload
  distribution.

Fusion Launcher supports community and private source libraries. The library author
is responsible for every URL, magnet URI, checksum, and legal claim they
publish. Fusion Launcher only labels this template as a community source and checks
that the JSON shape can be loaded.

## Hosting

You can host `repository.json` through GitHub Pages, a raw GitHub URL, a static
CDN, or a private HTTPS endpoint. Local JSON import is available in the
Fusion Launcher desktop build.
