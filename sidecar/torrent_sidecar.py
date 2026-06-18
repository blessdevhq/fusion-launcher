#!/usr/bin/env python3
"""Fusion Launcher P2P download sidecar.

A standalone libtorrent-backed downloader spawned by the Tauri app. It exists
because librqbit has no MSE/PE handshake obfuscation, so ISP/datacenter DPI
(TSPU and similar) resets every plaintext BitTorrent handshake. libtorrent
forces encrypted peer connections, which slips past that filtering.

Contract with the parent process:
  * Inputs come from the command line: <magnet_link> <save_path>.
  * stdout carries ONLY newline-delimited JSON, one object per second:
        {"progress": 0.0-100.0, "speed_bytes": <int>, "peers": <int>,
         "state": "checking|downloading|seeding|finished",
         "downloaded_bytes": <int>, "total_bytes": <int>}
  * Nothing else is ever written to stdout. Diagnostics go to stderr so the
    parser on the Rust side never sees malformed lines.
"""

import argparse
import json
import sys
import time

import libtorrent as lt


# libtorrent exposes more granular states than the four the parent cares about.
# Anything to do with hashing/allocating maps to "checking"; pulling the magnet
# metadata is reported as "downloading" since the UI shows it as in-progress.
def _build_state_map():
    states = lt.torrent_status.states
    mapping = {
        states.checking_files: "checking",
        states.downloading_metadata: "downloading",
        states.downloading: "downloading",
        states.finished: "finished",
        states.seeding: "seeding",
        states.checking_resume_data: "checking",
    }
    # These two were removed/renamed across libtorrent versions; add defensively.
    for name in ("allocating", "queued_for_checking"):
        state = getattr(states, name, None)
        if state is not None:
            mapping[state] = "checking"
    return mapping


STATE_MAP = _build_state_map()

# Public DHT bootstrap routers. DHT itself is UDP and not what DPI targets, so it
# reliably warms up and supplies peers even when tracker UDP is shaped.
DHT_ROUTERS = [
    ("router.bittorrent.com", 6881),
    ("dht.transmissionbt.com", 6881),
    ("router.utorrent.com", 6881),
    ("dht.libtorrent.org", 25401),
]


def log(message):
    """Diagnostics go to stderr only; stdout stays pure JSON."""
    print(message, file=sys.stderr, flush=True)


def make_session():
    # NOTE: pe_forced makes libtorrent refuse unencrypted peer connections in
    # both directions. This is the whole point of the sidecar — the encrypted
    # handshake is what evades DPI handshake resets.
    settings = {
        "listen_interfaces": "0.0.0.0:6881,[::]:6881",
        # --- DPI / TSPU evasion ---
        "out_enc_policy": lt.enc_policy.forced,  # forced encryption, outgoing
        "in_enc_policy": lt.enc_policy.forced,   # forced encryption, incoming
        "allowed_enc_level": lt.enc_level.both,  # RC4 or plaintext-after-handshake
        "prefer_rc4": True,                      # prefer full-stream RC4 obfuscation
        # --- peer transports ---
        "enable_outgoing_utp": True,
        "enable_incoming_utp": True,
        "enable_outgoing_tcp": True,
        "enable_incoming_tcp": True,
        # --- peer discovery ---
        "enable_dht": True,
        "enable_lsd": True,
        "enable_upnp": True,
        "enable_natpmp": True,
        # Bootstrap DHT via the settings pack (libtorrent 2.0 deprecated the
        # add_dht_router() method in favour of this).
        "dht_bootstrap_nodes": ",".join(f"{host}:{port}" for host, port in DHT_ROUTERS),
        # Keep the alert pipe shallow; we never read alerts, we poll status.
        "alert_mask": 0,
    }
    return lt.session(settings)


def add_magnet(session, magnet_link, save_path):
    # parse_magnet_uri returns an add_torrent_params object on libtorrent 1.2+.
    params = lt.parse_magnet_uri(magnet_link)
    params.save_path = save_path
    # Auto-managed + apply IP filter defaults are fine; just hand it off.
    return session.add_torrent(params)


def status_payload(handle):
    status = handle.status()
    state = STATE_MAP.get(status.state, "downloading")
    # total_wanted / total_wanted_done respect file selection and are 0 until
    # metadata resolves, which is exactly what the UI should show meanwhile.
    return {
        "progress": round(status.progress * 100.0, 2),
        "speed_bytes": int(status.download_rate),
        "peers": int(status.num_peers),
        "state": state,
        "downloaded_bytes": int(status.total_wanted_done),
        "total_bytes": int(status.total_wanted),
    }


def emit(payload):
    sys.stdout.write(json.dumps(payload))
    sys.stdout.write("\n")
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(
        description="Fusion Launcher libtorrent download sidecar."
    )
    parser.add_argument("magnet_link", help="magnet: URI to download")
    parser.add_argument("save_path", help="destination directory for the files")
    args = parser.parse_args()

    if not args.magnet_link.startswith("magnet:"):
        log("error: first argument must be a magnet: URI")
        return 2

    session = make_session()
    handle = add_magnet(session, args.magnet_link, args.save_path)

    try:
        while True:
            emit(status_payload(handle))
            time.sleep(1)
    except KeyboardInterrupt:
        # Parent asked us to stop; exit quietly with no stdout noise.
        return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - last-resort guard
        log(f"fatal: {error}")
        sys.exit(1)
