# Fusion Launcher Internal MVP Windows Install

## Automated release smoke

Before building an installer, run:

```powershell
npm run mvp:smoke
npm run mvp:visual
npm run qa
```

For a single release gate, run:

```powershell
npm run mvp:release
```

For a Windows RC package gate, run:

```powershell
npm run mvp:release:windows
```

`npm run mvp:smoke` validates the preview runtime path from source preview to
catalog, built-in demo setup, user-provided game import, download, launch, and
health diagnostics. `npm run mvp:visual` starts a local Next preview/dev server,
checks the console-launcher Home, Library search, Game Setup, import flow,
Collections, Downloads, and Settings Sources screens, and writes screenshots to
`.tmp/mvp-visual`.

`npm run mvp:release:windows` builds an NSIS package and then runs
`npm run mvp:package-smoke`. Package smoke verifies the release binary, confirms
an NSIS installer artifact exists, and runs the packaged binary harness against
clean `.tmp/package-smoke` app data. It does not automate the NSIS installer UI,
silent install, or uninstall flow.

If Playwright reports a missing browser, install Chromium once:

```powershell
npx playwright install chromium
```

## Manual Windows smoke

1. Build the installer with `npm run mvp:release:windows` on Windows.
2. Install the generated NSIS setup from `src-tauri/target/release/bundle/nsis`.
3. Windows may warn because Authenticode signing is not part of the RC gate. Updater packages are signed separately for tagged releases.
4. On first launch, click Set up demo. Fusion Launcher should connect the built-in demo repository, resolve and install the latest supported Mesen2 release, download the first-party NES demo entry, and enable Play Demo.
5. Click Play Demo. The configured Mesen2 executable should receive the downloaded first-party NES demo path.
6. Open Settings -> Sources. Add a community repository URL, preview it, connect it, refresh it, and confirm the catalog updates on Home/Library.
7. Open a user-provided showcase game. Confirm Game Details shows Emulator, System files, Game file, and Launch setup rows, then import a local fake fixture with the expected extension.
8. Start a catalog download. Confirm progress is visible in Game Details and Downloads.
9. Pause a download, restart the app, and confirm the paused/interrupted record is still visible and resumable.
10. Run Settings -> Health Check after setup. The built-in repository, Mesen2 emulator path, downloader session, imported game file, and NES demo file should be visible.
11. Logs are written to the app local data directory under `logs/fusion-launcher.log`; use Settings -> Copy diagnostics when reporting launch or download issues.
12. Updater packages are signed by Tauri's updater key, not the Windows code-signing certificate. Store the private key in GitHub Actions as `TAURI_SIGNING_PRIVATE_KEY` and store its password, if one is used, as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit the private key. Authenticode signing is deferred production hardening.
13. Test one real automatic install from each distinct archive path: RMG or Mesen2 from ZIP, and mGBA or PCSX2 from 7z. Confirm the installed executable launches a legally distributable or user-provided test game.
