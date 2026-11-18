# Creator wave 7 — native SDL service integration

From `creator-wave6-base` (`173e5d2`), merged `creator/input-service-e`
(`e1b61b2a9a7e7511f484ff52ba73a33c56eaff06`) at
`e690088437a58f077f19cf7de06c64ea14d5d475`. Git merged the shared command and
startup files without conflicts. No package, lockfile or wire-contract changes.

The host SDL service and its existing IPC bindings are included, with the
fail-closed synchronous callback barrier. See [E handoff](fusion-creator-input-service-e.md).
The production executor has not been connected to that barrier yet.

## Verification

- Windows `RUST_TEST_THREADS=4 npm run qa` — **PASS, exit 0** on `e690088`:
  typecheck, **229 Node tests**, static-check, Next build/export,
  **722 Rust library tests** (5 ignored), **3 Packer CLI tests**, rustfmt and
  Clippy with `-D warnings`. No permitted retry or FLAKY result.
- The run included native virtual-controller and checked teardown scenarios.
  The four entries in the known flaky allowlist passed on the initial run.
- Ran with exclusive `F:/MVP/.qa.lock`, VS2022 Native Tools and CMake.
  Raw log `.tmp/wave7-qa-01.log`, SHA-256
  `2b15905cea2da45659baab1c4be5ae623e5d65396a89ec875d737c02ae364e2f`.
- [Hosted Windows CI](https://github.com/blessdevhq/fusion-launcher/actions/runs/44413147301)
  **PASS** for the input branch SHA `e1b61b2`; not hosted evidence for the merge.

## Not tested by this wave

No UI source changes; wave 6 visual evidence remains separate. Physical SDL
capture/release, real Eden operation, input profile application, native Creator
window acceptance, and production release verification/export remain **NOT TESTED**.
The successful checks above do not qualify Creator v1 as ready.
