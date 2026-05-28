# RetroHydra Internal MVP Windows Install

1. Build the installer with `npm run tauri:build` on Windows.
2. Install the generated NSIS setup from `src-tauri/target/release/bundle/nsis`.
3. Windows may warn because the internal MVP is unsigned. Code signing is intentionally deferred until after MVP validation.
4. On first launch, click Set up demo. RetroHydra should connect the built-in demo repository, install pinned Mesen2, download the first-party NES demo entry, and enable Play Demo.
5. Logs are written to the app local data directory under `logs/retrohydra.log`; use Settings -> Copy diagnostics when reporting launch or download issues.
6. Run Settings -> Health Check after setup. The built-in repository, Mesen2 emulator path, and NES demo file should be ready.
7. Updater packages are signed by Tauri's updater key, not the Windows code-signing certificate. Store the private key in GitHub Actions as `TAURI_SIGNING_PRIVATE_KEY` and store its password, if one is used, as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never commit the private key.
