# Creator wave 6 — early UI integration

From `creator-wave5-base` (`7891e09`), merged `creator/ui-early`
(`74b38c9c45e72f50fa7240d6b53851aa744fe8a0`) with no conflicts at `f7aa959`.
The shell entry, modal focus handling and job/status display are an early UI,
not the completed authoring flow. Porting Studio remains separate.

Final checked source: `ad2039aa4a7232a90229073ba8d5f0afaf181f57`.
Only the visual test harness changed after `f7aa959`; Zustand state,
IPC and production UI were not changed by the integrator. No contract changes.

## Local Windows evidence

- `RUST_TEST_THREADS=4 npm run qa` — **PASS**, exit 0: typecheck, **229 Node**,
  static-check, build, **706 Rust lib** (5 ignored), **3 Packer CLI**, fmt, Clippy.
  Raw log `.tmp/wave6-qa-final.log`, SHA-256
  `c7974af67d6c37b5c37e528b2afce8f7f80b332715e86114709be015d29c64f8`.
- `npm run mvp:visual` — **PASS**, exit 0, **57 screenshots**. Creator layout and
  nested focus/restoration in EN/RU at 1280×720, 1365×768, 1440×900, 1920×1080;
  legacy and Fusion game-details context entry. Checked samples have visible
  focus, reachable close action and no visible overlap. Existing tokens retained;
  no new screenshot baseline approved. Raw log `.tmp/wave6-visual-02.log`, SHA-256
  `53d3864e01ea9b73b765ab29071de1f4702f589e7bfba1b8642f9791938f4115`.
- `wave6-visual-01.log` and `wave6-visual-diagnostic.log` — **FAIL**, exit 1.
  Exact-title assertion expected `Orbit Runtime`, but the fixture renders
  `Orbit Runtime Demo`. Cleanup also tried to disconnect a preview Source with
  an installed game. The Creator test now uses its own disposable browser
  context; the legacy MVP tests retain their independent state and assertions.
  The successful run used amended source, not an allowed flaky retry.
- The initial lock gate and a `node` alias TTY error stopped before starting
  their respective commands; neither counts as a test run. Actual QA and visual
  commands had exclusive `F:/MVP/.qa.lock` ownership. QA used VS2022/CMake.

## Not tested by this wave

Playwright preview verifies early browser UI states; it is not evidence of real
Tauri authoring, controller capture, production build/verify/export, or Eden.
The release-verification pipeline remains blocked on the C2 release/report
association and a verified immutable release-content reader.
