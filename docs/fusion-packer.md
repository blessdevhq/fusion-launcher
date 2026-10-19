# fusion-packer: authoring Fusion Source 1.0 repositories

`fusion-packer` shares the launcher's Rust `fusion_source` library. It computes
container BLAKE3 digests, byte sizes, extracted critical-file hashes and CAS
inventories, then validates the manifests before publishing the source index.
Input archives use the same safe extraction contract as launcher imports.

From the repository root:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin fusion-packer -- build --recipe ./recipe.json --out ./dist
cargo run --manifest-path src-tauri/Cargo.toml --bin fusion-packer -- verify --source-dir ./dist
cargo run --manifest-path src-tauri/Cargo.toml --bin fusion-packer -- inspect-artifact --file ./runtime.zip --critical emulator.exe:main_executable --critical core.dll:core_library
```

Build a standalone CLI with:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin fusion-packer
./src-tauri/target/debug/fusion-packer.exe --help
```

The Tauri application's default binary remains `fusion-launcher`. The packer does
not start the application, download releases or invoke recipe-supplied commands.
Success reports are JSON on stdout; failures are typed `FusionError` JSON on
stderr. Exit codes are 0 for success, 1 for build/verification failure and 2 for
invalid command arguments.

## Signing a source index

Generate an Ed25519 PKCS#8 PEM key and use it when publishing:

```powershell
./src-tauri/target/debug/fusion-packer.exe keygen --out ./private.pem
./src-tauri/target/debug/fusion-packer.exe build --recipe ./recipe.json --out ./dist --signing-key ./private.pem
./src-tauri/target/debug/fusion-packer.exe verify --source-dir ./dist
```

`keygen` uses the system random generator, creates a new file and refuses to
overwrite an existing path. Its JSON stdout contains `private_key_path` and the
Base64 `public_key`; it never prints private key material. Keep the PEM outside
the published repository. On Unix the newly created key has mode `0600`; on
Windows it is created with a protected DACL granting access to its owner and
SYSTEM only, before any private key bytes are written.

`build --signing-key` takes priority over `FUSION_PACKER_SIGNING_KEY`. The packer
derives `source_info.public_key` from the private key. An explicitly declared
recipe key must match, and a recipe containing a public key requires a signing
key. Existing repositories retain their identity and key: changing or removing
the key is rejected. Each incremental build verifies the previous index before
reading its manifests, then signs the entire combined index. Unsigned sources
without a key remain supported and omit the `signature` field.

The wire schema remains `"schema_version": "1.0"`. `signature` is standard
Base64 encoding of the 64 Ed25519 signature bytes. The signed message is the
UTF-8/ASCII prefix `FusionSourceIndex.v1` followed by a NUL byte, followed by the
compact JSON encoding of the original wire object with its top-level
`signature` removed. Every object's keys are recursively sorted; arrays retain
their order and all values retain their case and meaning. Relative URLs and
uppercase hashes are signed before any launcher normalization. Duplicate JSON
keys are rejected at every depth. Verification uses Ed25519 strict validation.

The launcher verifies the first keyed index using its declared key, then pins
that key in SQLite. Preview, reconnect, refresh and strict launch synchronization
check the pin, including disconnected sources. Missing/invalid signatures,
key replacement and key removal fail with `invalid_source_signature` before
changing the catalog. TOFU verifies continuity after first connection; it does
not independently identify the publisher. Public-key rotation is not supported.
`verify` checks the signature before opening manifests or artifacts and reports
`signature_verified` in its JSON result.

## Recipe

Both JSON and TOML recipes use `snake_case`. A recipe may contain one game for
an incremental update or several games for a new repository. Local input paths
resolve against the recipe's directory. Authors supply paths and roles rather
than hashes or file sizes:

```json
{
  "source_info": {
    "id": "my-source",
    "name": "My owned test source",
    "maintainer": "Source author"
  },
  "base_url": "https://example.org/my-source/",
  "games": [
    {
      "id": "my-game",
      "title": "My test game",
      "version": "1.0.0",
      "platform": "pc",
      "layers": [
        {
          "id": "base",
          "kind": "base",
          "local_path": "./owned-game",
          "archive": "zip",
          "mirrors": ["https://example.org/my-source/artifacts/{{ARTIFACT_FILENAME}}"],
          "writable_copy_paths": ["config/**"]
        },
        {
          "id": "runtime",
          "kind": "runtime",
          "local_path": "./runtime.zip",
          "mount_path": "runtime",
          "depends_on": ["base"],
          "critical_files": [
            {"path": "emulator.exe", "role": "main_executable"},
            {"path": "core.dll", "role": "core_library"}
          ]
        }
      ],
      "runtime": {
        "layer_id": "runtime",
        "executable": "runtime/emulator.exe",
        "args": ["--game", "{{WORKSPACE_DIR}}/game.bin"]
      },
      "config_templates": [
        {
          "id": "display",
          "path": "runtime/config/display.toml",
          "format": "toml",
          "template": "width = {{DISPLAY_WIDTH}}\nheight = {{DISPLAY_HEIGHT}}\n"
        }
      ]
    }
  ]
}
```

Directories become deterministic ZIPs: entries are sorted and metadata is
normalized. Existing ZIP/7z/raw inputs retain their container bytes. Generated
artifact names include their transport digest; mirror placeholders expand to
those names. A root or layer `base_url` provides an artifacts URL when explicit
mirrors are omitted. `archive: "raw"` represents a single file, with `raw_path`
specifying its path inside the layer.

Layer metadata also accepts `content`, `optional`, `default_enabled`, `patch`,
`dynamic_provider` and `attestation`; game execution, capability, persistence,
inheritance and template fields reuse the Fusion Source 1.0 types.

Patch recipes refer to the original and patched reference files instead of
supplying digests. For example, a raw xdelta layer can declare:

```json
{
  "id": "patch",
  "kind": "patch",
  "local_path": "./update.xdelta",
  "archive": "raw",
  "mount_path": "data",
  "critical_files": [{"path": "core.dll", "role": "core_library"}],
  "patch": {
    "format": "xdelta3",
    "base_layer": "base",
    "input_path": "data/core.dll",
    "output_path": "data/core.dll",
    "input_reference": "./owned-game/data/core.dll",
    "output_reference": "./patched-reference/core.dll"
  }
}
```

Set root `target_capabilities: ["patch.xdelta3.v1"]` (or `patch.bsdiff.v1`) when
authoring for a host configured with that patch adapter. This target registry is
recorded for offline verification; it does not install a patch tool or change
the launcher's advertised capabilities. The packer checks the original reference
against the base inventory and hashes the output reference. It does not execute
the patch. Launchers still require an explicitly configured, pinned and trusted
patch tool when assembling that layer. Target capabilities are stored as a sorted
set. Change the manifest version when changing that target set, since it is part
of the immutable authoring inventory.

For a local installation, replace `local_path`/archive/mirror fields with a
locator section. The reference directory is inspected locally and is never
published as an artifact:

```json
{
  "id": "base",
  "kind": "base",
  "local_locator": {
    "reference_dir": "./owned-game",
    "store": "manual",
    "path_hints": ["D:/OwnedGames/MyGame"],
    "key_files": [{"path": "game.exe", "role": "main_executable"}]
  }
}
```

Schema 1.0 stores locator key paths, roles and BLAKE3 hashes. Computed byte sizes
are returned in the build report's `locator_files` inventory without extending
the wire schema. Raw layers without a single critical file require the consuming
host to supply the reported `raw_path` in its install request.

## Repository updates

The output contains `artifacts/`, `manifests/`, `attestations/` when needed, and
`source-index.json`. `manifests/<game_id>.json` is a convenient current alias;
the index points to immutable `manifests/<game_id>-<blake3>.json` copies. Index
digests are calculated from the exact serialized bytes published at those URLs.
This keeps an old index usable while a newer build is being committed.

Adjacent `.artifacts.json` files bind local artifacts and CAS inventories for
offline verification. Keep them with the repository. Builds serialize writers
with `.fusion-packer.lock`, publish immutable files first and replace the index
atomically at the end. Previous immutable publications remain available.

To update one game, keep the same source ID and build a recipe containing that
game at its new version. Other index entries are retained. `extends` resolves
against games in the same recipe and the existing repository; declaration order
does not determine inheritance order. All affected manifests are validated
before publication. When changing a parent used by existing children, include
those children with updated manifest versions in the same build. This preserves
their explicit parent bindings. Explicit external parents can be supplied as
`parents: [{"source_id": "other-source", "path": "parent.json"}]`; inherited
remote layers must already have local artifact bindings in the repository.

## Inspecting a release

Archive type is inferred from `.zip` and `.7z`; other files are raw. Override it
with `--archive raw|zip|seven_zip`. Use `--raw-path bin/emulator.exe` to specify a
raw file's unpacked name. `--critical` is repeatable; roles are `main_executable`,
`injector_dll` and `core_library` (the corresponding PascalCase names also work).
`--auto-main-executable` requires an unambiguous executable candidate.

To generate release-specific metadata for a dynamic provider, supply all release
and asset identifiers:

```powershell
./src-tauri/target/debug/fusion-packer.exe inspect-artifact --file ./runtime.zip --critical emulator.exe:main_executable --release-id 123 --tag v1.2.0 --asset-id 456 --asset-url https://example.org/releases/runtime.zip
```

The resulting attestation binds the exact release, asset URL, archive digest and
unpacked executable hashes. The author must supply it through the consuming
launcher's trusted source-policy adapter. A signed source index binds the
manifest hashes and URLs to its pinned publisher key. Signing or hashing a file
does not grant executable trust.

The equivalent runtime layer recipe fields are:

```json
{
  "dynamic_provider": {
    "repo": "author/emulator",
    "asset_mask": "emulator-windows-*.zip",
    "channel": "nightly",
    "tag": "v1.2.0"
  },
  "attestation": {
    "release_id": "123",
    "release_tag": "v1.2.0",
    "asset_id": "456",
    "asset_url": "https://example.org/releases/runtime.zip",
    "tool_versions": {"runtime": "1.2.0"}
  }
}
```

## Validation

Build validation includes schema upcasting with inheritance, capability
negotiation, execution planning and actual config-template rendering. Missing
critical files, unsafe paths, archive collisions, links/reparse points, invalid
template syntax and unsupported required capabilities fail with typed errors.

`verify` is offline. It checks the exact manifest bytes named by the index and
the local artifacts recorded by the packer, including their extracted critical
files. Upload the complete generated repository to the HTTPS locations declared
in the recipe; offline verification does not verify that those mirrors have been
published.

Run the round-trip and complete Rust suites with:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml fusion_source::packer
cargo test --manifest-path src-tauri/Cargo.toml
```
