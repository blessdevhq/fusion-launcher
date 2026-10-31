# Creator contract JSON Schema

[`fusion-creator-contracts.schema.json`](./fusion-creator-contracts.schema.json) uses
JSON Schema **draft 2020-12**. It mirrors the portable Creator and IPC contracts in
`src/types/fusionCreator.ts`, `src-tauri/src/fusion_creator/`, and
`src-tauri/src/fusion_source/creator_schema.rs` for the legacy definitions.
The additive `creator-wave1-descriptor` definitions freeze a **new document format only**:
ProgramDescriptor 1.0, CreatorProgramProject 1.1, and ProgramCreatorManifest. They are not
yet mirrored by Rust/TS runtime DTOs or wired to the reader/executor. The normative semantics,
security checks and handoff gaps are in [`../fusion-creator-contracts-v1.md`](../fusion-creator-contracts-v1.md).

## Select the correct contract

The document root accepts exactly one of:

| Definition | Wire shape |
| --- | --- |
| `CreatorProject` | Portable project, **snake_case**, `schema_version: "1.0"` |
| `CreatorJobProgress` | Job progress event, **camelCase** |
| `InputState` | Input session state event, **camelCase** |
| `ProgramDescriptor` | Standalone program recipe, **snake_case**, document_type `program_descriptor`, schema_version `1.0` |
| `CreatorProgramProject` | New authoring carrier, **snake_case**, schema_version `1.1`, inline program_descriptor; no duplicate legacy adapter/profiles |

All definitions are available under `#/$defs/<Name>`. Additional definitions cover
adapter references, all component-origin and logical-binding variants, profiles,
projection rules, checkpoint/resume stamps, reports, releases, job records/receipts,
and input binding requests/receipts (including the separate raw-binding union).
`CreatorProfileSelection` is the explicit camelCase pair `buildProfileId` /
`controlsProfileId`; select it via `$defs`, not the document root.

**`CreatorManifest` validates only `manifest.creator`, the snake_case Creator
extension embedded in a GameManifest 1.1.** It does not validate the containing
GameManifest, its version, layers, runtime, or any other base-manifest field.
Neither a full GameManifest nor the bare extension is accepted by the document
root. Select `#/$defs/CreatorManifest` explicitly to validate the extension, and
use the existing Source validators for the containing manifest.
The original `4cf8971` DTO-only release did not change the legacy reader; that is
historical scope, not the unified source baseline. The unified source accepts
GameManifest 1.0/1.1 and retains the typed 1.1 extension in canonical/storage forms.
Optional `buildProfileId` / `controlsProfileId` are present in options/plans and
snapshot plans; absent IDs preserve legacy serialization. Manifest 1.1 execution
and non-null profile selections remain explicitly `action_unavailable` before
candidate mutation. Schema validation does not enable installation or profile
application; the combined base QA is recorded in `../fusion-creator-wave1-base.md`.
Descriptor-format verification is recorded separately in `../fusion-creator-descriptor-evidence.md`.

`ProgramCreatorManifest` is selected explicitly through `$defs`: it validates exactly
`{ program_descriptor: ProgramDescriptor }`, the newly specified alternative for GameManifest
1.1 `creator`. It is not a complete GameManifest, not a new SourceIndex version, and not the
legacy `CreatorManifest`. Inline descriptor content must be covered by canonical manifest
hash/signature. Current metadata consumers reject this new alternative until A adds typed
reader/canonical/storage support; do not silently discard it or enable execution prematurely.

The schema identifier is `urn:fusion:creator-contracts:1.0`. Register the whole
document with a draft-2020-12 validator before referencing, for example,
`urn:fusion:creator-contracts:1.0#/$defs/CreatorManifest`. Keep the full `$defs`
resolution context; copying a single definition without its dependencies does not
produce a standalone schema.

## Validation boundaries

- DTO objects reject unknown fields (`additionalProperties: false`). Dictionary
  objects intentionally allow arbitrary keys with a declared value schema.
  `FusionError.details` is an arbitrary JSON object.
- Nullable DTO fields must still be present. For example, `error: null` is valid;
  omitting `error` is not.
- Sequences are integers in `1..9007199254740991`. Revisions and progress counters
  are integers in `0..9007199254740991`. A single JSON document cannot establish
  sequence monotonicity, persistence across restarts, or valid state transitions.
- BLAKE3 strings contain exactly 64 lowercase hexadecimal characters. External
  origins require both transport and inventory hashes and a credential-free HTTPS
  URI. Enable `format: "uri"` assertion in your validator; draft 2020-12 does not
  require format assertions by default. Application URL parsing remains
  authoritative for URL normalization and transport policy.
- `redistribution_allowed` is a boolean, not a publication authorization. A value
  of `false` is representable; publication policy must be checked separately.
- Raw binding indices/masks follow Rust's `u32`/`u8` wire domains. The TypeScript
  raw-binding interfaces alone do not enforce these numeric limits.
- Input axis values have the shared wire range `-1..1`. Trigger-specific `0..1`
  rules, deadzone policy, device mapping, handle ownership, cross-reference
  integrity, relative-path safety, verified checkpoint provenance, and signature
  or hash verification remain application responsibilities.
- Portable project/manifest definitions do not contain local paths, signing-key
  references, or physical device identities. `InputBindingsSaveRequest` is a
  local IPC DTO, **not** a portable project or export format. Private-storage
  identity/binding DTOs and the complete command-request surface are not covered.
- Program paths have a lexical Windows prefilter, not filesystem authority. References,
  unique IDs, profile completeness, scalar-type/domain consistency, codec/seed compatibility,
  reparse/TOCTOU checks, environment policy, complete critical-file approval, private writable
  copies and export allowlisting remain mandatory host validation. Simple XML attributes and
  literal INI keys are not scripts, XPath or template expressions. Unknown capabilities fail
  explicitly; native application defaults must not be reported as Fusion remapping.

## Tests (no new dependencies)

From the repository root:

```sh
node --experimental-strip-types --test src/types/fusionCreatorJsonSchema.node-test.ts
```

The test is also picked up by the existing `npm test` glob. It reads
`fixtures/creator/contracts.json`, validates the project/job/input roots and
`manifest.creator`, compares shared definitions with the existing Zod validators,
and exercises union discriminators, missing/extra fields, nullability, numeric
bounds, hashes, credential rejection, and checkpoint/report DTOs.

Ajv is not a project dependency. The test therefore contains a deliberately
limited, test-only schema interpreter covering every assertion keyword used in
this document. A recursive audit fails on unsupported keywords or unresolved
references so schema growth cannot silently skip constraints. It is not a
production JSON Schema validator or a replacement for a conforming draft-2020-12
implementation. Its URI check uses Node's URL parser rather than a complete
RFC 3986 implementation.

The same test validates `fixtures/creator/programs/{eden-0.2.1,supertuxkart-1.5}.json`, new
carriers, unsafe paths, finite list budgets, operation/selector discriminators and forbidden
portable private fields. Read the fixture README: Eden is intentionally schema-only and
SuperTuxKart's byte inspection is not launch/lifecycle acceptance. New list assertions
(`minItems`, `maxItems`, `uniqueItems`) are audited and exercised; no dependency was added.
