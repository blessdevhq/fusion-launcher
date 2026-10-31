# Fusion Launcher Release Checklist

Use this checklist for a Windows public MVP preview release tagged `vX.Y.Z`.
Before tagging, confirm the tag version matches `package.json`,
`src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. These files currently
use `0.4.0`; update them together when preparing a different version.

## Local RC Gate

Run these before tagging:

```powershell
npm ci
pip install -r sidecar/requirements.txt
npx playwright install chromium
npm run qa
npm run mvp:smoke
npm run mvp:visual
npm run mvp:release:windows
git diff --check
```

Use Python 3.12 for the sidecar dependencies, as in the release workflow.
`npm run mvp:release:windows` builds the torrent sidecar, fetches the VC++
Redistributable, and builds a local NSIS smoke package with the sidecar
configuration. If
`TAURI_SIGNING_PRIVATE_KEY` is not set, it disables updater artifacts for the
local smoke build only, then runs packaged smoke against clean `.tmp/package-smoke`
data.

## Tagged Windows Release

Before pushing a `vX.Y.Z` tag, configure these GitHub Actions secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, only if the private key has a password

The tagged Windows workflow must complete these gates before upload:

- `npm ci`
- Python 3.12 setup and `pip install -r sidecar/requirements.txt`
- `npm run sidecar:build`
- `npm run redist:fetch`
- `npm run qa`
- `npm run mvp:smoke`
- `npx playwright install chromium`
- `npm run mvp:visual`
- updater signing secret validation
- `npm run tauri:build -- --target x86_64-pc-windows-msvc --config src-tauri/tauri.sidecar.conf.json` with updater signing enabled
- `npm run mvp:package-smoke` with `FUSION_LAUNCHER_RELEASE_DIR=src-tauri/target/x86_64-pc-windows-msvc/release`

The workflow creates a draft GitHub Release if one does not already exist for
the tag, then uploads or replaces these assets on that release:

- NSIS installer
- updater NSIS zip
- updater signature
- `latest.json`

## Manual Installer Smoke

After the assets are uploaded, download the NSIS installer from the release
and verify on a clean Windows profile before publishing a draft:

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
