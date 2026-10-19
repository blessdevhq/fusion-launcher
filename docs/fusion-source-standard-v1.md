# Fusion Source Standard v1.0: core contracts and implementation

`src-tauri/src/fusion_source` contains pure parsing, normalization, validation,
capability negotiation and declarative planning. Stage 1 performs no downloads,
filesystem projection, process execution, database writes or IPC calls. Existing
source formats continue through their existing pipelines.

## Wire format and normalization

External source and game JSON uses `snake_case` and schema version `1.0`.
`SourceIndex` contains compact game metadata, manifest URLs and manifest BLAKE3
digests. Parsing an index does not load its game manifests. The source's optional
Ed25519 public key requires a valid source-index signature in Stage 5B. Signature
verification belongs to the fetcher and precedes normalization; the upcaster
alone validates structural metadata.
The consuming transport must verify a manifest's raw-byte BLAKE3 before passing
it into the upcaster; this stage validates digest syntax, not file contents.
`user_provided` sources may declare `archive: "zip"` or `"seven_zip"` for imported
containers; omission defaults to `"raw"`. Imports and downloads use the same
read-only extraction contract.

The canonical manifest is the only input to negotiation and planning. The v1.0
upcaster normalizes values and resolves `extends` against caller-provided
manifests. Unqualified references resolve within the current source;
`source_id::game_id` references resolve against an explicitly supplied source.
Missing parents and cycles fail with structured errors. A child adds layers and
cannot reuse an inherited layer ID. Supplied child execution settings replace
the parent's execution group; otherwise that group is inherited. Config
templates may replace the same inherited template ID. Capability requirements
are combined, and a required capability takes precedence over optional.

`FusionError` has stable typed codes and typed details, with `camelCase`
serialization suitable for future IPC. Invalid schema versions, malformed
digests, unsafe paths, unresolved references and graph cycles are separate
failures. Validation is lexical and platform independent, including Windows
drive, UNC, alternate-stream and backslash traversal paths. It does not replace
Stage 2 checks for reparse points or filesystem ownership.

## Storage and execution invariants

CAS objects are immutable and read-only. `writable_copy_paths` identifies
layer-relative paths or masks that must become independent writable files in the
workspace; the planner prefixes the layer's `mount_path`.
Patches and generated config outputs must also be private writable files.
Changing permissions through a hardlink would change the CAS object as well and
is forbidden by the contract.

`transport_digest` verifies integrity of the downloaded container; it does not
establish publisher trust. `critical_files`
describes unpacked executable/DLL paths, expected BLAKE3 and their roles. The
two checks are separate; no archive digest substitutes for a launch integrity
check.

Ordinary CAS and workspace storage share `library_root` and a volume. Local
locators can explicitly request placement on the located game's volume or an
explicit cross-volume projection fallback. The planner records strategy without
touching the local installation. A known volume mismatch with no explicit
alternative is an error. Stage 2 must preserve these choices and never silently
copy an entire base installation.
Version 1 permits only one local-locator base per installation: its single volume
fact cannot safely describe multiple discovered installations on different disks.

The plan orders `Fetch -> Transform -> Env -> Exec`. User addon selection
distinguishes omitted selection (manifest defaults) from an explicitly empty
selection. Runtime selection distinguishes pinned artifacts from dynamic
provider rules; a later resolver must lock the concrete provider result in the
workspace snapshot. Strict synchronization, critical-file checks, save backup,
process graph ordering and the no-shell execution contract remain declarative
until their implementation stages.

## Capabilities and fixtures

Negotiation is per game. An unsupported required capability blocks that game;
unsupported optional capabilities produce warnings and use a declared,
supported fallback or omit the optional behavior. Plans reflect the negotiated
choice. The current production registry advertises parsing, planning and the
implemented Stage 2 local adapters; tests use an explicit hypothetical registry
to exercise future adapters without claiming that they are implemented.

Fixtures under `src-tauri/src/fusion_source/fixtures` cover Switch/Zelda shared
user files and shader cache, a fictional PS5/Azimuth/shadPS4 layered setup with
xdelta and a Nightly provider, and a Cyberpunk/Minecraft coop graph extending a
locally located base. Product names are scenario metadata, not compatibility
claims. All commercial bases, keys and firmware are user-provided. Remote URLs
use `example.org` for fictional runtime/mod artifacts; digest values are test
placeholders and are not usable release manifests.

Run the module tests with `cargo test fusion_source --lib` from `src-tauri`.

## Stage 2: local assembly and save persistence

Stage 2 consumes files already acquired by the caller. `LayerCache` imports raw,
ZIP and 7z artifacts into `CasStore`; network mirrors, storefront discovery,
GitHub release lookup, process supervision and telemetry are supplied by Stage 3;
SQLite and UI remain later integrations. An archive's digest is verified separately from
the BLAKE3 inventory of its extracted CAS tree and the critical binary hashes.
Extraction streams to private staging and rejects traversal, reparse entries,
duplicate paths and declared resource-limit violations.

Published CAS files are independent read-only copies. The workspace projects
unchanged files through hardlinks. Paths matched by `writable_copy_paths`, save
paths, generated configs and patch outputs use private writable files. Updating
one runtime layer reuses the base CAS identity and records added, changed,
removed and reused inputs in the snapshot. Local storefront originals are never
chmod'd: immutable projections instead retain deny-write file descriptors for
the lifetime of the returned `AssembledWorkspace` lease. This lease must remain
alive until every process using the workspace has exited. It also prevents
concurrent activation, rollback and cleanup operations.

CAS publication performs a full streaming digest audit after writers close.
Ordinary `CasStore::get` checks the digest-addressed inventory, paths, file set,
sizes and read-only protection without rereading every payload byte. This keeps
runtime-only updates independent of the base layer's byte size. Explicit
`CasStore::verify` performs a full byte audit and detects same-size corruption of
noncritical content; critical binary digests are checked on every workspace lease.

`WorkspaceAssembler` writes a versioned candidate and verifies its declared
critical binaries before replacing the active state file atomically. Failed
assembly leaves the previous active snapshot selected. A snapshot retains its
canonical manifest, selected plan, CAS identities, concrete dynamic-runtime lock,
patch-tool receipts, persistence bindings and save-backup reference. Stage 2
requires callers to supply a resolved dynamic lock; it does not query `latest`.
Binary rollback reopens a retained snapshot and reuses its recorded inputs.
Cleanup rejects active/previous snapshots and refuses unknown files or changed
ownership, preserving their bytes instead of recursively deleting them.
Unactivated candidate directories from interrupted or failed builds are retained
conservatively; this stage does not recursively collect unrecorded candidates.

Local-locator roots may share the library volume, use an explicitly approved
sibling workspace root on their own volume, or opt into cross-volume file
symlinks. No local base is silently copied wholesale. Cross-volume symlinks
require Windows file-symlink privileges and fail with a structured error when
unavailable. Managed auxiliary layers placed on the local base's volume may be
copied under that explicitly selected placement policy. The original installed
game remains outside workspace write authority and is checked against its key
file hashes again when a lease is opened.

Patch tools are caller-configured absolute executables with pinned SHA-256,
version and format; xdelta3/bspatch binaries are not shipped or downloaded by
this module. Invocation requires explicit trust of that pinned executable,
uses argument vectors rather than a shell, receives private copies of its inputs,
and verifies output digests before publication. Patch tests use a deterministic
subprocess protocol fixture; they do not claim validation against a bundled real
xdelta3/bspatch distribution.

`PersistenceManager` keeps per-game save data outside binary snapshots. Initial
imports preserve original APPDATA/LOCALAPPDATA files and supply process-local
environment overrides. Workspace directory saves use owned junctions; existing
single-file saves use mutable hardlinks on the same volume or file symlinks across
volumes. Missing declared save paths default to directories. Before update or
rollback, writers must be stopped and the current workspace reconciled: games
which save through temp-file + rename may have replaced a single-file hardlink.
Reconciliation streams the latest file into the persistent slot atomically.
Save backups complete before new binary activation. Binary rollback retains
current save data and never restores an old save backup implicitly.

Built-in JSON and TOML templates place path/string variables inside double-quoted
strings, with escaping applied for the target syntax. Numeric display values and
`ADDON_ENABLED:<id>` flags may be bare typed values or quoted strings. TOML uses
single-line strings; variables in comments or literal strings are rejected. INI
supports literal values or JSON-style double-quoted strings. Unknown variables,
malformed delimiters, invalid rendered documents and unsafe output paths fail
before activation. Config publication always creates a private file and never
changes a shared CAS inode's permissions.

## Stage 3: delivery and process ownership

`SecureHttpsTransport` reuses existing DNS, URL and redirect protections.
Network reads pass bounded chunks through a four-slot 64 KiB writer queue.
Writer completion, digest checks and publication are separate boundaries;
cancelled calls stop and join their writers before allowing cleanup or reuse.
Large transport and local-key hashes run in blocking workers with cancellation
checks between chunks. Cancellation can wait for an in-flight OS write/flush;
it never detaches a writer which can still mutate an artifact.

`MirrorCascadeResolver` tries mirrors in declared order. Network/status failures
and digest mismatches permit the next mirror; cancellation is terminal. The
expected transport digest is checked before exclusive publication. Configured
torrent adapters receive BEP 19 seeds only after metadata matches the magnet and
a single-file container's filename layout. Incompatible direct URLs remain
independent mirrors. Multi-file torrents require an explicit container adapter.
Native magnet seeds pass the same URL/layout checks before forwarding.

`TorrentAdapter`, `CloudUriResolver` and `StorefrontInventory` are injectable host
interfaces. The desktop host configures Windows storefront discovery and
Pixeldrain, and enables libtorrent after a successful artifact-protocol probe.
GoFile remains unavailable until its own provider is configured.
`configured_capabilities` reflects configured adapters. Existing legacy torrent
installation commands and their SQLite state machine are not invoked for layers.

`DynamicProviderResolver` uses GitHub release metadata and a source-authorized
attestation for the exact release, asset URL, transport digest and unpacked
critical hashes. Missing expected hashes block resolution. A newly downloaded
binary's self-computed hash grants neither integrity expectations nor executable
trust. The snapshot preserves release ID/tag, asset ID/URL and normalized hashes;
rollback reuses those locks and CAS bytes without querying latest. Local discovery
checks path hints and optional inventory candidates against key-file hashes,
returns volume IDs, and never changes storefront file permissions.

`ManifestFetcher` fetches bounded source indices and verifies raw manifest
BLAKE3 before upcasting. A hash-keyed cache is rehashed before reuse. Its private
`VerifiedManifestBinding` ties the raw digest to the exact canonical input and
game identity. Pass `verified.snapshot_binding()` through
`WorkspaceBuildOptions::verified_manifest` when assembling a fetched game;
an unrelated canonical manifest cannot reuse that binding. Local-only snapshots
without a binding cannot launch a strict-sync graph. `BoundManifestSyncVerifier`
always fetches the registered source index afresh before such a launch; offline,
stale or mismatched indices block it.

`PreLaunchIntegrityGuard` rechecks every final critical file and original local
key, independently of CAS/container verification. Missing injector projections
return `MissingInjectorDll`; other missing/corrupt executables or libraries return
`CorruptedCriticalBinary`; changed/missing local keys return `LocalBaseModified`.
The separate `ExecutableTrust` adapter must approve exact Fusion-namespaced
BLAKE3 identities. Verified file descriptors, ancestor directory guards and
no-follow handles to critical local projection links remain held until the
process group finishes, binding the launch path to the checked bytes.

`ProcessSupervisor` launches direct paths with argument vectors, orders process
graphs by dependencies and applies isolated APPDATA/LOCALAPPDATA/TEMP/TMP after
manifest environment values. It inherits only minimal OS environment. Windows
processes join a kill-on-close Job Object atomically at creation; only explicit
stdio pipe handles are inherited. Other platforms fail with `UnsupportedPlatform`
rather than claim equivalent tree supervision. Root exit, cancellation, partial
startup failure and a dropped waiting future terminate the group and wait for
every descendant before save reconciliation and lease release. An ownership
thread survives caller cancellation to complete this sequence. Log queues bound
total bytes/lines and per-stream partial lines; process count is bounded too.

## Stage 3: private crash reporting and extensions

Raw log rings, path context and process outcomes are internal Rust types without
serialization. `CrashReportBuilder` emits a private-field, Serialize-only
`SanitizedCrashReport` for nonzero primary exits, excluding user cancellation.
Mandatory redaction covers workspace/local bases, actual isolated process paths,
OS profile/username, email, IPv4/IPv6 and common authorization/session tokens.
Host custom sanitizers add redaction and cannot replace the core sanitizer.
Only whitelisted technical metadata, final executable/layer hashes and cleaned
log tails enter reports. Runtime identity includes local-locator keys and final
critical-file overrides. Signatures normalize timestamps and common Windows
memory-address forms before deterministic BLAKE3 hashing.

The supervisor checks `DeduplicationLedger` before returning a report. Its bounded,
atomically written local state stores signatures and numeric cooldown/window
counters only; suppressed reports are omitted while the decision remains
available. This core performs no automatic report upload. `CrashReportExporter`
is an interface for a future explicit user action; SQLite, Tauri IPC, report UI
and provider/plugin wiring are outside this stage.

`WasmExtensionHost` specifies memory-buffer-only input/output and memory/time
budgets. `UnsupportedWasmHost` always blocks execution and advertises no capability;
declared built-in template fallbacks continue through capability negotiation.

## Stage 4: storage, commands and catalog

The transactional v3→v4 SQLite migration adds isolated Fusion sources, compact
entries, verified manifests, CAS references, installations, workspace/save
snapshots, operations and cleaned crash reports. Legacy rows and `get_catalog`
retain their existing contracts. A disconnected source reserves its ID, URL and
public key: reconnecting cannot silently replace its origin. Adding an index
does not fetch every manifest or install any game.

`FusionService` injects `FusionStorage`, `ArtifactTransport`, `ProgressSink` and
host environment values. The Tauri adapter exposes `fusion_source_*`,
`fusion_game_*` and `fusion_crash_*` commands with camelCase DTOs and structured
`FusionError` values. Layer progress uses `fusion-source:operation-progress`,
including an operation ID, game identity, sequence, phase and optional layer.
Operations own their cancellation token independently of the IPC waiter. SQLite
guards concurrent operations, while UI sequence/retired-ID checks reject stale
progress. Startup marks interrupted operations explicitly.

Installed games retain their library and optional external workspace roots.
The root is recorded before assembly; filesystem activation remains the recovery
authority if the app exits between its atomic switch and the SQLite transaction.
Failed updates leave the prior installation active. Removal uses recorded file
ownership and preserves saves, CAS objects and history; unknown files are retained.
Changing the library root is blocked while a Fusion operation is active.

The mixed catalog discriminates `legacy` and `fusion_source` entries. Fusion
details fetch and verify manifests lazily, present independent readiness,
addons/runtime choices, user file inputs, update/rollback and exact-hash trust.
Native games need no invented console platform or emulator profile. Installed
non-strict games can use a verified snapshot offline; strict-sync launch refreshes
the root and transitive parent indices and checks the exact resolved manifest.

Crash IPC exposes only a validated, re-sanitized stored report. Sending requires
the user's explicit action and the same endpoint shown in the preview. Uploads
are bounded HTTPS requests through the existing target/DNS policy; redirects
are rejected. No automatic report sending is performed. The browser preview
simulates these flows without launching payloads or submitting reports.

The host configures HTTPS, GitHub Releases, Windows storefront discovery and
Pixeldrain. Magnet support requires a successful native artifact-protocol probe;
patch formats require locally configured, pinned and trusted tools. Wasm execution
and automatic patch-tool provisioning are not advertised. An unavailable required
adapter blocks its game with a typed reason. Sources with a public key require
an Ed25519 signature checked against the persisted source identity.

An update can rebuild a workspace whose critical executable/DLL was deleted or
whose local base changed. It reads ownership-validated snapshot metadata to reuse
unchanged CAS layers; that metadata grants no process lease or launch authority.
The old integrity error remains visible, new critical files are verified before
activation, and current saves are reconciled and backed up. Unsafe ownership or
path errors cannot use this repair path.

Windows metadata publication allocates and writes an explicit UTF-16 NUL after
the destination name, independently of allocation padding. Regression coverage
checks every name-alignment remainder, Unicode and extended-length paths, exact
destination names and absence of leftover staging files.

## Stage 5A: offline source authoring

`fusion_source::packer::PackerEngine` and the separate `fusion-packer` binary
build and verify repositories from JSON/TOML recipes. The authoring engine shares
the launcher's schema, negotiation, planning, template and safe archive/CAS
contracts. It hashes exact published manifest bytes and commits an incremental
source index after immutable artifacts and manifests are available. See
[the fusion-packer guide](fusion-packer.md) for recipes and CLI examples.

Round-trip tests cover source fetching and verified manifest binding, CAS import,
workspace activation and config rendering, critical-file integrity, native
Windows supervision, runtime-only updates, base reuse and rollback. Release
attestations remain source-policy inputs; signing an index grants no executable
trust.

## Stage 5B: host adapters and index authentication

Windows inventory reads Steam registry roots and bounded old/new
`libraryfolders.vdf`, app manifests and immediate `steamapps/common` directories.
GOG inventory reads installed-game registry paths; Epic reads `InstallLocation`
from `%ProgramData%/Epic/EpicGamesLauncher/Data/Manifests/*.item`. Metadata is
limited to 1 MiB per file and VDF nesting to 32 levels; enumeration is bounded.
Missing and malformed individual records are skipped. Discovery never launches
a storefront, recurses through reparse points, modifies files or changes their
permissions. Every candidate still passes all BLAKE3 key-file checks and volume
identity checks in `LocalLocatorResolver`.

Patch configuration is read once per launcher session from
`<app_data_dir>/fusion-source-host.json`, with a 64 KiB limit:

```json
{
  "schema_version": 1,
  "patch_tools": [{
    "format": "xdelta3",
    "executable": "C:\\Tools\\xdelta3.exe",
    "sha256": "<exact 64-character SHA-256>",
    "version": "approved-release-version",
    "trusted": true
  }]
}
```

`bsdiff` names the bspatch format. Absolute paths are required; the recommended
managed location is `<app_local_data_dir>/fusion-source-tools`. Binaries are not
downloaded automatically. `trusted` defaults to false. The version is the release
label bound to the approved exact SHA-256, not a binary self-report. Host tool
trust is independent of source/game executable approvals. Only verified formats
are advertised; retained Windows descriptors block replacement until the registry
is released, and execution rechecks the digest. Restart after changing the config.
Configuration errors disable optional tools and are logged.

The native torrent probe requires `artifact_v1` and explicit WebSeed support.
Artifact sessions disable resume state and unapproved embedded seeds, validate
single-file metadata before enabling payload writes, and use private staging
outside the cascade destination. A session owner retains staging until sidecar
writers and relay handlers have stopped, including caller cancellation or abort.
Unconfirmed process termination preserves the private directory rather than
deleting files potentially in use. The cascade verifies the transport digest
before CAS import.

Approved BEP 19 URLs are represented by per-session token URLs on a loopback
relay. Upstream requests stream through the same HTTPS connector DNS and redirect
policy as normal downloads; libtorrent never receives remote WebSeed URLs.
Ranges, response lengths, concurrency and deadlines are bounded. A mirror without
valid range support remains an independent sequential fallback.
`pixeldrain://<case-sensitive-id>` resolves to the direct HTTPS API file stream.
Provider registration determines cloud capabilities; providers cannot bypass the
secure HTTPS transport.

Optional `OperationProgress.transfer` contains `downloadedBytes`, `totalBytes`
and `speedBytesPerSecond` for HTTPS, cloud and torrent layer downloads. A zero total
means the server has not supplied its length; completion supplies the final total.
Events use the existing operation identity and monotonic
sequence; terminal and superseded operations reject later transfer callbacks.

An index may contain top-level `signature`, standard Base64 of a 64-byte Ed25519
signature. The signed bytes are `FusionSourceIndex.v1` followed by a NUL byte and
compact UTF-8 JSON with recursively sorted object keys and the top-level signature
removed. Array order, original relative URLs, string case and omitted/null fields
are preserved; duplicate JSON object keys are rejected. This check runs before
upcasting or URL resolution. Canonical indices are not re-signed or used as proofs.

First connection verifies the advertised key/signature and pins that identity
(trust on first use). This protects later updates; it does not independently
authenticate the first hosting response. Refresh, strict synchronization, launch
verification and reconnect use SQLite's retained key, including disconnected
sources. Missing/invalid signatures and key or identity replacement return
`invalid_source_signature` before catalog changes. Key rotation is unsupported.
Sources without public keys retain unsigned operation; a source which previously
declared a metadata-only public key must publish a valid signature to refresh.
