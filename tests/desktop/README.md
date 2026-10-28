# Native Tauri smoke test (Windows)

This is a small real-window WebDriver smoke test, not a visual-diff suite and not a replacement for the existing Playwright `mvp:visual` checks.

From the project root in Git Bash:

```bash
npm run build
npm --prefix tests/desktop ci
node tests/desktop/run.mjs
```

`npm run build` is required before every run when frontend files have changed: the runner tests the existing static `out/` export. The first native build downloads Rust dependencies and may take several minutes and significant disk space. Later builds reuse `.tmp/desktop-e2e-target`. The test opens a real app window; don't interact with it while the test is running.

The runner copies the current `src-tauri`, `public`, and `out` into a unique directory under `.tmp/desktop-e2e-*`, with a unique Tauri identifier. Only in that copy it enables the embedded WDIO WebDriver plugin and the global Tauri API, disables Windows `fusion://` registration, legacy-database import, and background artwork scraping. The real source files, normal app identifier, database, and normal Cargo target are not intentionally modified. Test app data is created under the OS app data folders for that unique identifier; stage files and test app data are not automatically deleted. Do not ship the instrumented test build: its local WebDriver endpoint is for testing only.

The test checks that first-run onboarding and the language control appear, that Next opens the source step, and that proceeding without a source is disabled. It saves `artifacts/onboarding-source.png` in the stage directory printed by the runner. The screenshot is an inspection artifact, not an automated pixel comparison.

The copied-source instrumentation is deliberately fail-closed: if an expected source string changes, the runner stops rather than running with unsafe assumptions. Adapt the staged modifications when app startup changes. `npm test` alone requires `FUSION_E2E_BINARY` and is only for rerunning an already instrumented binary; use the commands above for a fresh isolated build.
