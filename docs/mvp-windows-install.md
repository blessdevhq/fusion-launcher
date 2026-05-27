# RetroHydra Internal MVP Windows Install

1. Build the installer with `npm run tauri:build` on Windows.
2. Install the generated NSIS setup from `src-tauri/target/release/bundle/nsis`.
3. Windows may warn because the internal MVP is unsigned. Code signing is intentionally deferred until after MVP validation.
4. On first launch, connect the official demo repository, configure one MVP emulator, download a demo entry, then run Settings -> Health Check.
5. Logs are written to the app local data directory under `logs/retrohydra.log`; use Settings -> Copy diagnostics when reporting launch or download issues.
