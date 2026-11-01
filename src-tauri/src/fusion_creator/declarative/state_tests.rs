use super::*;
use crate::fusion_source::program_schema::{
    ProgramPersistenceExport, ProgramPersistenceKind, ProgramPersistencePolicy,
};
use std::fs;
use tempfile::tempdir;

fn entry(path: &str, kind: ProgramPersistenceKind) -> ProgramPersistence {
    ProgramPersistence {
        path: path.into(),
        kind,
        policy: ProgramPersistencePolicy::PreserveLatest,
        export: ProgramPersistenceExport::Never,
    }
}
#[test]
fn latest_file_and_directory_generation_survives_binary_replacement() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let current = temp.path().join("current");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&current).unwrap();
    fs::create_dir(&candidate).unwrap();
    let entries = vec![
        entry("settings.ini", ProgramPersistenceKind::File),
        entry("saves", ProgramPersistenceKind::Directory),
    ];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    store.rehydrate(&current, &entries).unwrap();
    assert!(
        !current.join("settings.ini").exists(),
        "missing file is not manufactured as a directory"
    );
    fs::write(current.join("settings.ini"), b"first").unwrap();
    fs::create_dir_all(current.join("saves/empty")).unwrap();
    fs::write(current.join("saves/slot"), b"first-save").unwrap();
    store.capture(&current, &entries).unwrap();
    fs::write(current.join("settings.ini"), b"latest").unwrap();
    fs::write(current.join("saves/slot"), b"latest-save").unwrap();
    store.capture(&current, &entries).unwrap();
    fs::write(candidate.join("settings.ini"), b"packaged-default").unwrap();
    fs::create_dir(candidate.join("saves")).unwrap();
    fs::write(candidate.join("saves/stale"), b"must disappear").unwrap();
    let reopened = ProgramPersistenceStore::open(&root).unwrap();
    reopened.rehydrate(&candidate, &entries).unwrap();
    assert_eq!(fs::read(candidate.join("settings.ini")).unwrap(), b"latest");
    assert_eq!(
        fs::read(candidate.join("saves/slot")).unwrap(),
        b"latest-save"
    );
    assert!(candidate.join("saves/empty").is_dir());
    assert!(!candidate.join("saves/stale").exists());
    fs::write(candidate.join("settings.ini"), b"private-edit").unwrap();
    fs::write(candidate.join("saves/slot"), b"private-edit").unwrap();
    assert_eq!(fs::read(current.join("settings.ini")).unwrap(), b"latest");
    reopened.rehydrate(&candidate, &entries).unwrap();
    assert_eq!(fs::read(candidate.join("settings.ini")).unwrap(), b"latest");
}

#[test]
fn generation_inventory_rejects_unrecorded_files_before_modifying_candidate() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("save"), b"saved").unwrap();
    let entries = vec![entry("save", ProgramPersistenceKind::File)];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    store.capture(&candidate, &entries).unwrap();
    let latest: Latest = read_json(&root.join("latest.json")).unwrap();
    fs::write(
        root.join("generations")
            .join(&latest.generation)
            .join("data/unrecorded"),
        b"unexpected",
    )
    .unwrap();
    fs::write(candidate.join("save"), b"candidate unchanged").unwrap();
    assert!(store.rehydrate(&candidate, &entries).is_err());
    assert_eq!(
        fs::read(candidate.join("save")).unwrap(),
        b"candidate unchanged"
    );
}

#[test]
fn captured_deletions_do_not_resurrect_packaged_or_older_saves() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    let entries = vec![
        entry("nested/save", ProgramPersistenceKind::File),
        entry("slots", ProgramPersistenceKind::Directory),
    ];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    fs::create_dir(candidate.join("nested")).unwrap();
    fs::write(candidate.join("nested/save"), b"old").unwrap();
    fs::create_dir(candidate.join("slots")).unwrap();
    fs::write(candidate.join("slots/old"), b"old").unwrap();
    store.capture(&candidate, &entries).unwrap();
    let first: Latest = read_json(&root.join("latest.json")).unwrap();
    fs::remove_file(candidate.join("nested/save")).unwrap();
    fs::remove_dir_all(candidate.join("slots")).unwrap();
    store.capture(&candidate, &entries).unwrap();
    let latest: Latest = read_json(&root.join("latest.json")).unwrap();
    assert_ne!(first.generation, latest.generation);
    assert_eq!(
        fs::read(
            root.join("generations")
                .join(first.generation)
                .join("data/nested/save")
        )
        .unwrap(),
        b"old"
    );
    fs::write(candidate.join("nested/save"), b"packaged").unwrap();
    fs::create_dir(candidate.join("slots")).unwrap();
    fs::write(candidate.join("slots/default"), b"packaged").unwrap();
    store.rehydrate(&candidate, &entries).unwrap();
    assert!(!candidate.join("nested/save").exists());
    assert!(!candidate.join("slots").exists());
}

#[test]
fn rejects_invalid_paths_overlaps_and_kinds_before_any_write() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("good"), b"unchanged").unwrap();
    let store = ProgramPersistenceStore::open(&root).unwrap();
    for entries in [
        vec![
            entry("good", ProgramPersistenceKind::File),
            entry("../escape", ProgramPersistenceKind::File),
        ],
        vec![
            entry("good", ProgramPersistenceKind::File),
            entry("GOOD", ProgramPersistenceKind::File),
        ],
        vec![
            entry("dir", ProgramPersistenceKind::Directory),
            entry("dir/nested", ProgramPersistenceKind::File),
        ],
        vec![
            entry("Dir/a", ProgramPersistenceKind::File),
            entry("dir/b", ProgramPersistenceKind::File),
        ],
        vec![entry("good", ProgramPersistenceKind::Directory)],
        vec![entry("good/missing", ProgramPersistenceKind::File)],
        vec![entry("C:/absolute", ProgramPersistenceKind::File)],
        vec![entry("bad\\path", ProgramPersistenceKind::File)],
    ] {
        assert!(
            store.capture(&candidate, &entries).is_err(),
            "accepted {entries:?}"
        );
        assert!(
            store.rehydrate(&candidate, &entries).is_err(),
            "accepted {entries:?}"
        );
        assert_eq!(fs::read(candidate.join("good")).unwrap(), b"unchanged");
        assert!(!root.join("latest.json").exists());
    }
    assert!(store.capture(&root, &[]).is_err());
}

#[test]
fn failed_capture_preserves_previous_pointer_and_corruption_is_fail_closed() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("save"), b"saved").unwrap();
    let entries = vec![entry("save", ProgramPersistenceKind::File)];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    store.capture(&candidate, &entries).unwrap();
    let pointer = fs::read(root.join("latest.json")).unwrap();
    fs::remove_file(candidate.join("save")).unwrap();
    fs::create_dir(candidate.join("save")).unwrap();
    assert!(store.capture(&candidate, &entries).is_err());
    assert_eq!(fs::read(root.join("latest.json")).unwrap(), pointer);
    fs::remove_dir(candidate.join("save")).unwrap();
    fs::write(candidate.join("save"), b"candidate").unwrap();
    let latest: Latest = read_json(&root.join("latest.json")).unwrap();
    let saved = root
        .join("generations")
        .join(latest.generation)
        .join("data/save");
    assert_ne!(
        fs_safety::file_identity(&saved).unwrap(),
        fs_safety::file_identity(&candidate.join("save")).unwrap()
    );
    fs::write(saved, b"corrupted").unwrap();
    assert!(store.rehydrate(&candidate, &entries).is_err());
    assert_eq!(fs::read(candidate.join("save")).unwrap(), b"candidate");
}

#[test]
fn rehydrate_does_not_change_shared_readonly_file_permissions_or_bytes() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("save"), b"saved").unwrap();
    let entries = vec![entry("save", ProgramPersistenceKind::File)];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    store.capture(&candidate, &entries).unwrap();
    fs::remove_file(candidate.join("save")).unwrap();
    let cas = temp.path().join("cas-file");
    fs::write(&cas, b"immutable").unwrap();
    let mut permissions = fs::metadata(&cas).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&cas, permissions).unwrap();
    fs::hard_link(&cas, candidate.join("save")).unwrap();
    let result = store.rehydrate(&candidate, &entries);
    assert!(fs::metadata(&cas).unwrap().permissions().readonly());
    assert_eq!(fs::read(&cas).unwrap(), b"immutable");
    assert_eq!(fs::read(candidate.join("save")).unwrap(), b"saved");
    let mut cleanup_permissions = fs::metadata(&cas).unwrap().permissions();
    cleanup_permissions.set_readonly(false);
    fs::set_permissions(&cas, cleanup_permissions).unwrap();
    result.unwrap();
}

#[test]
fn rejects_junctions_even_with_a_missing_save_leaf() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    let outside = temp.path().join("outside");
    fs::create_dir(&candidate).unwrap();
    fs::create_dir(&outside).unwrap();
    let link = candidate.join("linked");
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success());
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let store = ProgramPersistenceStore::open(&root).unwrap();
    let entries = vec![entry("linked/missing", ProgramPersistenceKind::File)];
    let captured = store.capture(&candidate, &entries);
    let restored = store.rehydrate(&candidate, &entries);
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    assert!(captured.is_err());
    assert!(restored.is_err());
    assert!(!outside.join("missing").exists());
}

#[cfg(windows)]
#[test]
fn rejects_extended_path_alias_of_the_store_as_candidate() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    fs::create_dir(&root).unwrap();
    let alias = fs::canonicalize(&root).unwrap();
    let store = ProgramPersistenceStore::open(&alias).unwrap();
    assert!(store.capture(&root, &[]).is_err());
    assert!(store.rehydrate(&root, &[]).is_err());
}

#[test]
fn store_lock_serializes_capture_and_rehydrate() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("durable");
    let candidate = temp.path().join("candidate");
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("save"), b"saved").unwrap();
    let entries = vec![entry("save", ProgramPersistenceKind::File)];
    let store = ProgramPersistenceStore::open(&root).unwrap();
    store.capture(&candidate, &entries).unwrap();
    for capture in [true, false] {
        let lock = store.lock().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker_root = root.clone();
        let worker_candidate = candidate.clone();
        let worker_entries = entries.clone();
        let thread = std::thread::spawn(move || {
            let store = ProgramPersistenceStore::open(&worker_root).unwrap();
            started_tx.send(()).unwrap();
            let result = if capture {
                store.capture(&worker_candidate, &worker_entries)
            } else {
                store.rehydrate(&worker_candidate, &worker_entries)
            };
            done_tx.send(result).unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let blocked = done_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err();
        drop(lock);
        assert!(blocked, "operation ignored the process lock");
        done_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
            .unwrap();
        thread.join().unwrap();
    }
}
