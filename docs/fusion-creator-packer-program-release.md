# Packer: signed managed ProgramDescriptor release

Base: `creator-wave8-base` (`45005ad`). `PackerGameRecipe.program_descriptor` is an optional local-recipe field. With it, the Packer emits GameManifest 1.1 with the descriptor inline and requires the existing Source signing key. The signed SourceIndex remains 1.0 and pins the exact manifest bytes. Existing recipes continue through the 1.0 path. Frozen portable and IPC contracts are unchanged.

This slice supports standalone managed ZIP components with explicit redistribution permission and an empty component `source_root`. The descriptor determines the mount; conflicting declarations are rejected. Packer hashes all loadable files, compiles the full checked inventory, and runs the planner for every declared profile pair. Other component origins, inheritance and partial archive roots are not implemented in this slice. It uses the existing publication URL behavior; hosting parity is not asserted.

Keep the signing key outside the publication tree and all input components. Packer rejects an exact copy of the supplied key in the checked input inventory before publication, including a renamed copy, and `verify` rejects unsigned inline-descriptor releases. This check is not a general secret scanner or a substitute for an export allowlist.

## Windows evidence

- `packer-program-red.log`: the signed fixture could not parse because the recipe rejected `program_descriptor` (exit 101).
- `packer-program-green.log`: 11 legacy tests passed, 2 new tests failed because the self-check used the legacy plan without profile IDs (exit 101). The self-check now plans every declared pair for inline programs; legacy plans are unchanged.
- `RUST_TEST_THREADS=4 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --lib fusion_source::packer_tests:: -- --nocapture`: **13 passed**, 0 failed (`packer-program-profiles.log`, exit 0). The test builds a signed folder Source, removes the author inputs, checks the signature and TOFU pin, installs from a fresh library through FusionService, and checks the mounted files, CAS and generated INI. Launch is deliberately refused as untrusted; the synthetic files are not real application binaries. Another test rejects unsupported/invalid recipes and checks that the previous publication is unaffected.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` and `git diff --check`: **PASS**.
- `packer-review-red.log`: the two regressions failed as expected — build accepted a component containing the fixture signing key, and verify accepted an unsigned ProgramDescriptor Source.
- `RUST_TEST_THREADS=4 npm run qa` under exclusive `F:/MVP/.qa.lock` after the fixes: **PASS** (`packer-review-qa-01.log`, exit 0), 229 Node tests, 731 Rust tests (5 ignored), 3 Packer CLI tests, formatting and Clippy. Both new regressions passed.
- Observed launch, Creator IPC Build/Verify/Export, hosted HTTPS and physical controller: **NOT TESTED** in this slice.
