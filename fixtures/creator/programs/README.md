# Program descriptor fixtures

These are **contract/schema examples**, not successful Creator releases or runtime acceptance.
See `docs/fusion-creator-contracts-v1.md` and `docs/fusion-creator-descriptor-evidence.md`.

- `supertuxkart-1.5.json`: a non-emulator, Windows x64 descriptor based on the actual official
  `SuperTuxKart-1.5-win.zip`. Transport and normalized CAS-tree BLAKE3 were computed by streaming
  the whole archive and every file (CRC checked by ZipFile). `source_root` removes exactly the
  observed top-level directory; `mount_path` does not affect the normalized inventory hash.
  Recorded inspection: `docs/evidence/supertuxkart-1.5-inspection.json`. No executable was run.
  `required_files` is a small required-role declaration, NOT a complete critical-file inventory.
  All other PE/DLL files remain subject to the common approval policy.
- `eden-0.2.1.json`: **schema-only**, a normalized directory containing `eden.exe`, not an assertion
  about the root of every upstream Windows package. `schema-fixture-only-unresolved-eden-runtime`
  intentionally resolves to no artifact in this repository. `redistribution_allowed: false`
  and the incomplete keyboard example prohibit treating this as a verified/exportable release.
  Real simple keys and the `backend\default=false` companion come from pinned v0.2.1 source.
  No ROMs, BIOS, firmware, keys, actual user configuration, physical device IDs or fabricated
  payload hashes are included.

The new document carriers are `CreatorProgramProject` 1.1 and `ProgramCreatorManifest`.
Legacy `CreatorProject` 1.0 and `CreatorManifest` are unchanged and reject the new shapes.
The Node test verifies this boundary explicitly. No ProgramDescriptor Rust/Zod execution
consumer is introduced by these fixtures.
