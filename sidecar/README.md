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

`externalBin` is intentionally **not** in the base `tauri.conf.json` — that would
force every `cargo`/`tauri dev` build to have the sidecar present (Tauri copies
external binaries at compile time). Instead it lives in the overlay
`src-tauri/tauri.sidecar.conf.json`, applied only when packaging:

```
npm run sidecar:build
npm run tauri:build:bundled        # tauri build --config src-tauri/tauri.sidecar.conf.json
```

So normal development, tests, and the CI rust job work without the sidecar; only
the installer build (and anyone testing the libtorrent engine) needs it.

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
