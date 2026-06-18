# Third-Party Notices

Fusion Launcher is licensed under GPL-3.0-or-later (see `LICENSE`). It bundles
the components below. All are GPL-3.0-compatible.

## Torrent download sidecar (`sidecar/`)

The P2P downloader ships as a standalone executable (`fusion-torrent`) built from
`sidecar/torrent_sidecar.py` with PyInstaller. It bundles:

- **libtorrent-rasterbar** (Python bindings, `libtorrent` on PyPI) —
  BSD-3-Clause. Copyright © Arvid Norberg and contributors.
  https://github.com/arvidn/libtorrent
- **Boost** (transitively, via libtorrent) — Boost Software License 1.0.
- **OpenSSL** (transitively, via libtorrent) — Apache License 2.0 (OpenSSL 3.x).
- **CPython** runtime — Python Software Foundation License 2.0.
- **PyInstaller** bootloader — GPL-2.0-or-later with the PyInstaller bootloader
  exception that permits bundling applications under any license.

The sidecar is an independent process invoked over stdio; it is not linked into
the main application binary.

## Why the sidecar exists

librqbit (the previous in-process engine) does not implement MSE/PE peer
encryption, so DPI-based traffic shaping resets every plaintext BitTorrent
handshake. libtorrent forces encrypted handshakes (`pe_forced`), which restores
working downloads. See `sidecar/README.md` and the project CLAUDE.md.
