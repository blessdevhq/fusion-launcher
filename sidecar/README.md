# Fusion Launcher P2P download sidecar

`torrent_sidecar.py` is a standalone libtorrent downloader spawned by the Tauri
app. We moved P2P out of the main process because librqbit has no MSE/PE
handshake obfuscation, so DPI (TSPU and similar) resets every plaintext
BitTorrent handshake. libtorrent forces **encrypted** peer connections
(`pe_forced`), which slips past that filtering, and adds uTP + DHT.

## Runtime contract

```
fusion-torrent <magnet_link> <save_path>
```

- **stdout** carries ONLY newline-delimited JSON, one object per second:
  ```json
  {"progress": 42.5, "speed_bytes": 1048576, "peers": 15, "state": "downloading", "downloaded_bytes": 440401920, "total_bytes": 1036779520}
  ```
  `progress` is 0.0–100.0, `state` is one of `checking | downloading | seeding | finished`.
  `downloaded_bytes`/`total_bytes` are 0 until metadata resolves.
- **stderr** carries diagnostics/errors only — never parse it as JSON.
- The process runs until killed by the parent (it keeps seeding after finishing).

### Fusion Source artifact protocol

The original positional command remains the legacy download protocol. Fusion
Source uses an independent process for each single-file container:

```text
fusion-torrent --probe
fusion-torrent --artifact [--web-seed http://127.0.0.1:<port>/<private-token>] <magnet> <private-save-path>
```

`--probe` checks the installed native bindings and starts a session with every
network transport, discovery service and listening interface disabled. It emits
one bounded capability document and exits. The host accepts the capability only
after a successful exit of the complete Windows Job process tree within its
probe deadline. The document contains `protocol_version: 1`, `artifact_v1`,
`explicit_webseeds` and the installed `libtorrent` version.

Artifact status lines add `artifact_layout` containing the actual metadata
`info_hash` and one `files` entry with `path` and byte `size`. Before the parent
acknowledges `inventory_approved`, priorities remain zero and upload-only mode
blocks payload requests. The parent validates the identity and size, rejects
multifile, padding, symlink and unsafe path layouts, then exclusively reserves
the file. The sidecar rechecks the metadata before enabling payload delivery.

Artifact mode never loads or saves fastresume data. Embedded and magnet WebSeeds
are overridden; native HTTP seeds are removed. Explicit seeds must use the
parent's private loopback relay. That relay maps unpredictable tokens to admitted
HTTPS URLs and streams full or single-range responses through Fusion's existing
DNS and redirect policy. Native `ssrf_mitigation` stays enabled.

The host owns unique directories under
`<library>/.fusion-source/torrent-staging`. Its supervisor awaits both the native
process tree and relay before publishing a container without replacing an
existing file. Cancellation and an aborted caller signal that private owner;
unconfirmed process termination preserves the private directory. Transport
digest validation and CAS insertion remain the resolver cascade's responsibility.

Native acceptance uses generated data and loopback sockets:

```powershell
python -m venv .tmp/stage5b-python
.tmp/stage5b-python/Scripts/python.exe -m pip install -r sidecar/requirements.txt
$env:PATH = "$PWD\.tmp\stage5b-python\Scripts;$env:PATH"
npm run sidecar:build
$env:FUSION_NATIVE_SIDECAR_EXE = "$PWD\src-tauri\binaries\fusion-torrent-x86_64-pc-windows-msvc.exe"
.tmp/stage5b-python/Scripts/python.exe sidecar/test_native_artifact.py
```

Set `FUSION_NATIVE_ADAPTER_TEST_BINARY` to the current compiled Rust library test
executable to additionally run the ignored native adapter/Windows Job publication
test against the fixture's live seeder. The fixture checks metadata before
approval, a complete peer transfer through the Rust adapter, and a separate
transfer using only the admitted BEP 19 seed after the peer seeder is paused.
The Python lifecycle unit suite requires no sockets:

```powershell
.tmp/stage5b-python/Scripts/python.exe -m unittest discover -s sidecar -p test_torrent_sidecar_lifecycle.py
```

## Building the standalone .exe

The build is automated by `scripts/build-sidecar.mjs`, which runs PyInstaller and
places the binary where Tauri expects it. CI (`.github/workflows/release-windows.yml`)
runs the same steps automatically.

1. **Install Python 3.12** (64-bit). libtorrent publishes prebuilt wheels for
   CPython 3.8–3.12, so no C++ toolchain is required. Avoid 3.13 until a wheel
   exists for it.

2. **Install the build dependencies** (libtorrent + PyInstaller):
   ```powershell
   pip install -r sidecar/requirements.txt
   ```

3. **Build the binary** from the repo root:
   ```powershell
   npm run sidecar:build
   ```
   This produces `src-tauri/binaries/fusion-torrent-<target-triple>.exe`
   (e.g. `fusion-torrent-x86_64-pc-windows-msvc.exe`). The script uses
   `--onefile --console`: a console build keeps stdout/stdin wired so the parent
   can read the JSON. (It must not use `--windowed`/`--noconsole`, which detach
   stdout; Tauri spawns it with `CREATE_NO_WINDOW` so no window flashes.)

4. **Sanity-check it runs** (Ctrl+C to stop):
   ```powershell
   .\src-tauri\binaries\fusion-torrent-x86_64-pc-windows-msvc.exe "magnet:?xt=urn:btih:..." "F:\Temp\dl"
   ```
   You should see one JSON line per second and nothing else on stdout.

## How it's bundled

`externalBin` is intentionally not in the base `tauri.conf.json`. Keeping it in
`src-tauri/tauri.sidecar.conf.json` lets normal development, tests, and the CI
Rust job run without requiring a locally built sidecar.

For a Windows distributable, use the canonical release command:

```powershell
npm run mvp:release:windows
```

That command builds the sidecar, fetches the VC++ redistributable, applies the
sidecar config overlay, and runs package smoke. `npm run tauri:build` alone is
not enough for a tester/release build because it does not include the torrent
sidecar, the redist resource, or the NSIS redist hook.

The equivalent manual sequence is:

```powershell
npm run sidecar:build
npm run redist:fetch
npm run tauri:build:bundled
```

### Testing the libtorrent engine locally

```powershell
npm run sidecar:build
npm run tauri:dev -- --config src-tauri/tauri.sidecar.conf.json
```

(libtorrent is the only/default engine, so no environment flag is needed. Use
`npm run tauri:dev`/`npx tauri` — the `tauri` CLI is a local npm bin, not on PATH.)

## Other platforms

`npm run sidecar:build` works on macOS and Linux too (it derives the target
triple from `rustc -vV`). Build on each target OS — PyInstaller does not
cross-compile.
