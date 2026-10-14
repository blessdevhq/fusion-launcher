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
import os
import sys
import threading
import time

import libtorrent as lt


# libtorrent exposes more granular states than the parent cares about.
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
    alert_mask = 0
    if hasattr(lt, "alert") and hasattr(lt.alert, "category_t"):
        cat = lt.alert.category_t
        alert_mask = getattr(cat, "storage_notification", 0) | getattr(cat, "error_notification", 0)

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
        # Keep metadata peers while all file priorities are held at zero for
        # the parent's ownership approval; that is a temporary upload-only state.
        "close_redundant_connections": False,
        # --- peer discovery ---
        "enable_dht": True,
        "enable_lsd": True,
        "enable_upnp": True,
        "enable_natpmp": True,
        # Bootstrap DHT via the settings pack (libtorrent 2.0 deprecated the
        # add_dht_router() method in favour of this).
        "dht_bootstrap_nodes": ",".join(f"{host}:{port}" for host, port in DHT_ROUTERS),
        "alert_mask": alert_mask,
    }
    return lt.session(settings)


def extract_btih(magnet_link):
    for part in magnet_link.split("&"):
        lower = part.lower()
        if lower.startswith("magnet:?"):
            lower = lower[len("magnet:?"):]
            part = part[len("magnet:?"):]
        if lower.startswith("xt=urn:btih:"):
            hash_val = part.split(":")[-1].strip()
            # BTIH must strictly be 40 hex chars or 32 base32 chars to prevent path traversal
            if len(hash_val) == 40 and all(c in "0123456789abcdefABCDEF" for c in hash_val):
                return hash_val
            if len(hash_val) == 32 and all(
                c in "234567abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ" for c in hash_val
            ):
                return hash_val
            return None
    return None


def write_resume(path, buf):
    # The parent exclusively reserved this file and recorded its identity before
    # enabling writes. Keep that identity across saves; never create/adopt a path.
    with open(path, "r+b") as f:
        f.seek(0)
        f.write(buf)
        f.truncate()
        f.flush()
        os.fsync(f.fileno())


def hold_payload(params):
    # Metadata exchange remains enabled, but no piece requests may be made until
    # Rust has exclusively reserved the metadata-declared paths. Auto-management
    # would otherwise eventually clear upload_mode without our acknowledgement.
    params.flags |= lt.torrent_flags.upload_mode
    params.flags |= lt.torrent_flags.default_dont_download
    params.file_priorities = [0] * len(params.file_priorities)
    params.flags &= ~lt.torrent_flags.auto_managed
    params.flags &= ~lt.torrent_flags.paused
    params.flags &= ~lt.torrent_flags.seed_mode
    return params


def validate_resume(params, magnet_params):
    # Fusion never remaps torrent files. Native resume checking can touch mapped
    # paths before the parent's inventory acknowledgement, so reject all maps.
    if params.renamed_files:
        raise ValueError("fastresume contains unsupported file mappings")
    cached_hashes = [params.info_hashes]
    if params.ti is not None:
        cached_hashes.append(params.ti.info_hashes())
    for family in ("v1", "v2"):
        has_family = "has_" + family
        if not getattr(magnet_params.info_hashes, has_family)():
            continue
        expected = getattr(magnet_params.info_hashes, family)
        for hashes in cached_hashes:
            if not getattr(hashes, has_family)() or getattr(hashes, family) != expected:
                raise ValueError("fastresume identity does not match the requested magnet")


def add_magnet_with_resume(session, magnet_link, save_path, resume_file):
    magnet_params = lt.parse_magnet_uri(magnet_link)
    if resume_file and os.path.isfile(resume_file):
        try:
            with open(resume_file, "rb") as f:
                raw_data = f.read()
            if raw_data:
                params = lt.read_resume_data(raw_data)
                validate_resume(params, magnet_params)
                params.save_path = save_path
                # Resume files may omit metadata and peers (especially after a
                # stop). Preserve explicit discovery from the original magnet
                # rather than waiting indefinitely for public DHT discovery.
                params.peers = list(dict.fromkeys(list(params.peers) + list(magnet_params.peers)))
                trackers = list(params.trackers)
                tiers = list(params.tracker_tiers)
                tiers.extend([0] * (len(trackers) - len(tiers)))
                for index, tracker in enumerate(magnet_params.trackers):
                    if tracker not in trackers:
                        trackers.append(tracker)
                        tiers.append(magnet_params.tracker_tiers[index]
                                     if index < len(magnet_params.tracker_tiers) else 0)
                params.trackers = trackers
                params.tracker_tiers = tiers
                params.url_seeds = list(dict.fromkeys(list(params.url_seeds) + list(magnet_params.url_seeds)))
                log(f"info: loaded fastresume data from {resume_file}")
                return session.add_torrent(hold_payload(params))
        except Exception as error:
            log(f"warning: fastresume load failed: {error}; parsing magnet fresh")

    params = magnet_params
    params.save_path = save_path
    return session.add_torrent(hold_payload(params))


def metadata_inventory(handle, resume_file):
    if not handle.status().has_metadata:
        return None
    files = handle.torrent_file().files()
    paths = []
    for index in range(files.num_files()):
        flags = files.file_flags(index)
        if flags & lt.file_storage.flag_symlink:
            raise ValueError("torrent symlink entries are not supported")
        if flags & lt.file_storage.flag_pad_file:
            continue
        paths.append(files.file_path(index))
    if resume_file:
        paths.append(os.path.basename(resume_file))
    if len(paths) > 32768:
        raise ValueError("torrent inventory exceeds the supported file count")
    return paths


def payload_priorities(handle):
    storage = handle.torrent_file().files()
    # Hybrid/v2 torrents include alignment padding. Native libtorrent keeps
    # those priorities at zero even when asked to set every index to four.
    return [0 if storage.file_flags(index) & lt.file_storage.flag_pad_file else 4
            for index in range(storage.num_files())]


def get_resume_flag(name):
    # Check lt.torrent_handle first (libtorrent 1.2/2.x variations)
    th = getattr(lt, "torrent_handle", None)
    if th and hasattr(th, name):
        return getattr(th, name)
    # Check lt.resume_data_flags_t (libtorrent 2.0+ enum)
    rdf = getattr(lt, "resume_data_flags_t", None)
    if rdf and hasattr(rdf, name):
        return getattr(rdf, name)
    return 0


def request_save_resume(handle, only_if_modified=False):
    if not handle.is_valid():
        return
    try:
        status = handle.status()
        if not status.has_metadata:
            return
        if only_if_modified:
            flags = get_resume_flag("only_if_modified")
        else:
            flags = get_resume_flag("flush_disk_cache")
        # Without this flag, native fastresume can contain piece state but no
        # metadata. Cache the file inventory as well so resume can reauthorize
        # writes before any peer is available.
        flags |= get_resume_flag("save_info_dict")
        handle.save_resume_data(flags)
    except Exception as error:
        log(f"warning: request_save_resume error: {error}")


def drain_and_save_alerts(session, resume_file):
    """Drain session alerts and write fastresume if save alert arrives. Returns True if saved."""
    save_success_class = getattr(lt, "save_resume_data_alert", None)
    save_fail_class = getattr(lt, "save_resume_data_failed_alert", None)
    saved = False
    for alert in session.pop_alerts():
        if alert.category() & lt.alert.category_t.error_notification:
            log("libtorrent: " + alert.message())
        if save_success_class and isinstance(alert, save_success_class):
            if resume_file:
                try:
                    buf = lt.write_resume_data_buf(alert.params)
                    write_resume(resume_file, buf)
                    log(f"info: fastresume persisted ({len(buf)} bytes)")
                    saved = True
                except Exception as error:
                    log(f"warning: could not write fastresume: {error}")
        elif save_fail_class and isinstance(alert, save_fail_class):
            msg = getattr(alert, "message", lambda: "unknown")()
            log(f"warning: save_resume_data_failed_alert: {msg}")
    return saved


def save_and_wait(session, handle, resume_file, timeout=2.5):
    """Request resume data and wait up to timeout seconds for libtorrent to post the alert."""
    if not handle.is_valid():
        return False
    try:
        if not handle.status().has_metadata:
            return False
    except Exception:
        return False

    request_save_resume(handle, only_if_modified=False)
    start = time.time()
    while time.time() - start < timeout:
        if drain_and_save_alerts(session, resume_file):
            return True
        time.sleep(0.05)
    log("warning: fastresume save timed out")
    return False


def status_payload(handle, is_paused=False):
    status = handle.status()
    if is_paused or getattr(status, "paused", False):
        state = "paused"
    else:
        state = STATE_MAP.get(status.state, "downloading")

    return {
        "progress": round(status.progress * 100.0, 2),
        "speed_bytes": 0 if (is_paused or getattr(status, "paused", False)) else int(status.download_rate),
        "peers": int(status.num_peers),
        "state": state,
        "downloaded_bytes": int(status.total_wanted_done),
        "total_bytes": int(status.total_wanted),
    }


def emit(payload):
    sys.stdout.write(json.dumps(payload))
    sys.stdout.write("\n")
    sys.stdout.flush()


def listen_stdin(handle, is_paused_flag, stop_event, inventory_ready=None, inventory_approved=None):
    try:
        for line in sys.stdin:
            text = line.strip()
            if not text:
                continue
            try:
                command = json.loads(text)
                action = command.get("action")
                if action == "inventory_approved":
                    if inventory_ready is None or not inventory_ready.is_set():
                        raise ValueError("inventory approval arrived before metadata")
                    inventory_approved.set()
                elif action == "pause":
                    handle.pause()
                    is_paused_flag[0] = True
                    log("info: torrent pause requested via stdin")
                    stop_event.set()
                    break
                elif action == "resume":
                    handle.resume()
                    is_paused_flag[0] = False
                    log("info: torrent resumed via stdin")
                elif action == "stop":
                    handle.pause()
                    log("info: torrent stop requested via stdin")
                    stop_event.set()
                    break
            except Exception as error:
                log(f"warning: stdin command error: {error}")
    finally:
        # Parent EOF is also a stop request: reach the normal fastresume path.
        stop_event.set()


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
    btih = extract_btih(args.magnet_link)
    resume_file = None
    if btih:
        os.makedirs(args.save_path, exist_ok=True)
        resume_file = os.path.join(args.save_path, f".{btih}.fastresume")

    handle = add_magnet_with_resume(session, args.magnet_link, args.save_path, resume_file)

    is_paused_flag = [False]
    stop_event = threading.Event()
    inventory_ready = threading.Event()
    inventory_approved = threading.Event()
    stdin_thread = threading.Thread(
        target=listen_stdin,
        args=(handle, is_paused_flag, stop_event, inventory_ready, inventory_approved),
        daemon=True
    )
    stdin_thread.start()

    last_periodic_save = time.time()
    PERIODIC_RESUME_INTERVAL_SECS = 30
    inventory = None
    expected_total = None
    desired_priorities = None
    metadata_peers = list(lt.parse_magnet_uri(args.magnet_link).peers)
    payload_requested = False
    payload_enabled = False
    reconnect_setting = None
    last_stall_log = 0

    try:
        while not stop_event.is_set():
            now = time.time()
            if reconnect_setting is not None:
                if now >= reconnect_setting[0]:
                    session.apply_settings({"min_reconnect_time": reconnect_setting[1]})
                    reconnect_setting = None
                else:
                    for peer in metadata_peers:
                        handle.connect_peer(peer)
            if inventory is None:
                inventory = metadata_inventory(handle, resume_file)
                if inventory is not None:
                    storage = handle.torrent_file().files()
                    expected_total = sum(
                        storage.file_size(index)
                        for index in range(storage.num_files())
                        if not storage.file_flags(index) & lt.file_storage.flag_pad_file
                    )
                    desired_priorities = payload_priorities(handle)
                    metadata_peers.extend(peer.ip for peer in handle.get_peer_info())
                    inventory_ready.set()
            # During fastresume checking libtorrent still applies the priorities
            # carried by add_torrent_params. Do not race that deferred zero-priority
            # update with our authorization update: it can otherwise overwrite
            # the new priorities after we have enabled requests.
            checking = STATE_MAP.get(handle.status().state) == "checking"
            if inventory_approved.is_set() and not payload_requested and not checking:
                # Disconnect the metadata-only connections before changing their
                # upload-only role. A remote seeder can otherwise close the old
                # connection after set_upload_mode(false) reset its retry clock,
                # leaving a stale peer entry instead of a payload connection.
                handle.pause()
                handle.prioritize_files(desired_priorities)
                payload_requested = True
            # prioritize_files is asynchronous. All-zero metadata priorities can
            # transiently report finished/100%; do not publish completion or
            # enable piece requests until native priorities have actually applied.
            if (payload_requested and not payload_enabled and not checking
                    and handle.status().paused
                    and handle.get_file_priorities() == desired_priorities
                    and handle.status().total_wanted == expected_total):
                handle.unset_flags(lt.torrent_flags.upload_mode | lt.torrent_flags.default_dont_download)
                handle.resume()
                session.apply_settings({"close_redundant_connections": True})
                # A seeder may disconnect a metadata-only upload-mode peer. Give
                # that peer a short reconnect window after authorization. The
                # old connection may still exist when the first connect_peer
                # runs, so retry until its asynchronous disconnect has settled.
                previous_retry = session.get_settings()["min_reconnect_time"]
                session.apply_settings({"min_reconnect_time": 1})
                reconnect_setting = (now + 10, previous_retry)
                for peer in metadata_peers:
                    handle.connect_peer(peer)
                payload_enabled = True
            if payload_enabled and now - last_periodic_save >= PERIODIC_RESUME_INTERVAL_SECS:
                last_periodic_save = now
                request_save_resume(handle, only_if_modified=True)

            if payload_enabled:
                drain_and_save_alerts(session, resume_file)
                if handle.flags() & lt.torrent_flags.upload_mode and now - last_stall_log >= 5:
                    last_stall_log = now
                    log(f"warning: payload entered upload_mode; error={getattr(handle.status(), 'error', '')}; priorities={handle.get_file_priorities()}")

            payload = status_payload(handle, is_paused=is_paused_flag[0])
            payload["inventory"] = inventory
            if not payload_enabled:
                payload["state"] = "checking"
                payload["progress"] = 0.0
            emit(payload)
            time.sleep(1)

        if payload_enabled:
            save_and_wait(session, handle, resume_file)
        # The last periodic report can lag behind pieces flushed during stop.
        # Publish the final byte count before the event stream closes. Keep the
        # ownership gate in the final report as well when stopped before approval.
        final_payload = status_payload(handle, is_paused=is_paused_flag[0])
        final_payload["inventory"] = inventory
        if not payload_enabled:
            final_payload["state"] = "paused" if is_paused_flag[0] else "checking"
            final_payload["progress"] = 0.0
        emit(final_payload)
        return 0
    except KeyboardInterrupt:
        if payload_enabled:
            save_and_wait(session, handle, resume_file)
        return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - last-resort guard
        log(f"fatal: {error}")
        sys.exit(1)
