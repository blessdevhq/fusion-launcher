# Fusion Launcher Release Checklist

Use this checklist for the `v0.1.0` Windows release candidate.

## Local RC Gate

Run these before tagging:

```powershell
npm run qa
npm run mvp:smoke
npm run mvp:visual
npm run mvp:release:windows
git diff --check
```

`npm run mvp:release:windows` builds a local NSIS smoke package. If
`TAURI_SIGNING_PRIVATE_KEY` is not set, it disables updater artifacts for the
local smoke build only, then runs packaged smoke against clean `.tmp/package-smoke`
data.

## Tagged Windows Release

Before pushing a `vX.Y.Z` tag, configure these GitHub Actions secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, only if the private key has a password

The tagged Windows workflow must complete these gates before upload:

- `npm ci`
- `npm run qa`
- `npm run mvp:smoke`
- `npx playwright install chromium`
- `npm run mvp:visual`
- updater signing secret validation
- signed `tauri build --target x86_64-pc-windows-msvc`
- `npm run mvp:package-smoke` with `FUSION_LAUNCHER_RELEASE_DIR=src-tauri/target/x86_64-pc-windows-msvc/release`

The workflow publishes a draft GitHub Release with:

- NSIS installer
- updater NSIS zip
- updater signature
- `latest.json`

## Manual Installer Smoke

After the draft release is created, download the NSIS installer from the draft
release and verify on a clean Windows profile:

- Install the NSIS package.
- Launch Fusion Launcher from the installed shortcut/start menu entry.
- Run the built-in demo setup.
- Confirm the NES demo reaches Play readiness.
- Open Settings and verify app data, downloads, logs, health, and platform setup state.
- Confirm no commercial ROM, BIOS, firmware, or key payloads are bundled or downloaded.
- Uninstall Fusion Launcher.

## Release Boundaries

- Updater signing is mandatory for tagged releases.
- Windows Authenticode signing is not part of this RC gate.
- Silent installer automation is not part of this RC gate.
- Source-defined setup profiles are not part of this RC gate.
