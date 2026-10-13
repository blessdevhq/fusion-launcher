use crate::download_operations::OperationLease;
use crate::install_ownership::{self, InstallationOwnership};
use crate::storage::RepositoryStore;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn key(id: &str) -> String {
    format!("download_ownership:{id}")
}

fn history_key(id: &str) -> String {
    format!("download_ownership_history:{id}")
}

fn previous_records(
    store: &RepositoryStore,
    id: &str,
) -> Result<Vec<(String, InstallationOwnership)>, String> {
    store
        .list_config_prefix(&format!("download_previous_ownership:{id}:"))?
        .into_iter()
        .map(|(key, raw)| {
            serde_json::from_str::<InstallationOwnership>(&raw)
                .map(|record| (key, record))
                .map_err(|e| format!("Invalid previous installation ownership: {e}"))
        })
        .filter(|result| {
            result
                .as_ref()
                .map_or(true, |(_, record)| record.subject() == id)
        })
        .collect()
}

fn read_history(store: &RepositoryStore, id: &str) -> Result<Vec<InstallationOwnership>, String> {
    let mut records = store
        .get_config(&history_key(id))?
        .map(|raw| {
            serde_json::from_str::<Vec<InstallationOwnership>>(&raw)
                .map_err(|e| format!("Invalid installation ownership history: {e}"))
        })
        .transpose()?
        .unwrap_or_default();
    if records.iter().any(|record| record.subject() != id) {
        return Err("Installation ownership subject mismatch.".into());
    }
    records.extend(
        previous_records(store, id)?
            .into_iter()
            .map(|(_, record)| record),
    );
    Ok(records)
}

/// All trusted manifests remain available after a new attempt chooses a new root.
pub(crate) fn read_all(
    store: &RepositoryStore,
    id: &str,
) -> Result<Vec<InstallationOwnership>, String> {
    let mut records = read_history(store, id)?;
    if let Some(current) = read(store, id)? {
        if current.subject() != id {
            return Err("Installation ownership subject mismatch.".into());
        }
        records.push(current);
    }
    Ok(records)
}

/// Call while the subject is reserved; callers retain only manifests needing retry.
pub(crate) fn replace_history(
    store: &RepositoryStore,
    id: &str,
    records: &[InstallationOwnership],
) -> Result<(), String> {
    if records.iter().any(|record| record.subject() != id) {
        return Err("Installation ownership subject mismatch.".into());
    }
    // Persist before retiring legacy records: a failed migration never loses authority.
    store.set_config(
        &history_key(id),
        &serde_json::to_string(records).map_err(|e| e.to_string())?,
    )?;
    for (key, _) in previous_records(store, id)? {
        store.remove_config(&key)?;
    }
    Ok(())
}

pub(crate) fn allocate(
    store: &RepositoryStore,
    id: &str,
    lease: &OperationLease,
    parent: &Path,
) -> Result<InstallationOwnership, String> {
    if lease.is_cancelled() {
        return Err(crate::downloads::DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let previous = read(store, id)?;
    if previous
        .as_ref()
        .is_some_and(|record| record.subject() != id)
    {
        return Err("Installation ownership subject mismatch.".into());
    }
    if store
        .get_torrent_download(id)?
        .is_some_and(|r| matches!(r.status.as_str(), "error" | "interrupted"))
    {
        if let Some(mut previous) = previous.clone() {
            let parent = parent.canonicalize().map_err(|e| e.to_string())?;
            let has_partial = std::fs::read_dir(previous.root())
                .ok()
                .is_some_and(|entries| {
                    entries
                        .flatten()
                        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "part"))
                });
            if previous.root().parent() == Some(parent.as_path()) && has_partial {
                install_ownership::renew_attempt(&mut previous, id, lease.attempt())?;
                lease.set_installation(&previous)?;
                store.set_config(
                    &key(id),
                    &serde_json::to_string(&previous).map_err(|e| e.to_string())?,
                )?;
                return Ok(previous);
            }
        }
    }
    if let Some(previous) = previous {
        let mut history = read_history(store, id)?;
        if !history
            .iter()
            .any(|record| record.root() == previous.root())
        {
            history.push(previous);
        }
        replace_history(store, id, &history)?;
    }
    let record = install_ownership::allocate_directory(parent, id, lease.attempt())?;
    lease.set_installation(&record)?;
    store.set_config(
        &key(id),
        &serde_json::to_string(&record).map_err(|e| e.to_string())?,
    )?;
    Ok(record)
}

pub(crate) fn read(
    store: &RepositoryStore,
    id: &str,
) -> Result<Option<InstallationOwnership>, String> {
    store
        .get_config(&key(id))?
        .map(|raw| {
            serde_json::from_str(&raw).map_err(|e| format!("Invalid installation ownership: {e}"))
        })
        .transpose()
}

pub(crate) async fn snapshot(
    store: Arc<Mutex<RepositoryStore>>,
    id: String,
    lease: OperationLease,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut record = {
            let guard = store.lock().map_err(|_| "Repository lock poisoned.")?;
            read(&guard, &id)?
        }
        .ok_or("Installation ownership missing.")?;
        if record.subject() != id || record.attempt() != lease.attempt() {
            return Err("download_superseded".into());
        }
        install_ownership::snapshot_files(&mut record)?;
        let guard = store.lock().map_err(|_| "Repository lock poisoned.")?;
        if guard
            .download_attempt(&id)?
            .is_none_or(|a| a.attempt != lease.attempt())
        {
            return Err("download_superseded".into());
        }
        guard.set_config(
            &key(&id),
            &serde_json::to_string(&record).map_err(|e| e.to_string())?,
        )?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) async fn cleanup(record: InstallationOwnership) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let report = install_ownership::cleanup_files(&record);
        report
            .entries
            .into_iter()
            .filter_map(|(path, outcome)| match outcome {
                install_ownership::CleanupOutcome::Preserved(reason) => {
                    Some(format!("{}: {reason}", path.display()))
                }
                _ => None,
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) fn renew_restored(
    store: &RepositoryStore,
    id: &str,
    lease: &OperationLease,
) -> Result<(), String> {
    if let Some(mut record) = read(store, id)? {
        install_ownership::renew_attempt(&mut record, id, lease.attempt())?;
        lease.set_installation(&record)?;
        store.set_config(
            &key(id),
            &serde_json::to_string(&record).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download_operations::OperationCoordinator;
    use crate::storage::DownloadAttempt;

    #[test]
    fn replacing_completed_install_keeps_its_exact_cleanup_authority() {
        let temp = tempfile::tempdir().unwrap();
        let store = RepositoryStore::open(&temp.path().join("db.sqlite")).unwrap();
        let coordinator = OperationCoordinator::new();
        let first = coordinator.start("game").unwrap();
        let old = allocate(&store, "game", &first, temp.path()).unwrap();
        store
            .record_direct_game_download_started("game", "http", "old", 1)
            .unwrap();
        let identity = DownloadAttempt {
            subject: "game".into(),
            attempt: first.attempt().into(),
            run: 1,
        };
        store.assign_download_attempt(&identity).unwrap();
        store
            .complete_attempt(&identity, "game", "old", None, 1)
            .unwrap();
        drop(first);
        let second = coordinator.start("game").unwrap();
        let new = allocate(&store, "game", &second, temp.path()).unwrap();
        assert_ne!(old.root(), new.root());
        let all = read_all(&store, "game").unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|record| record.root() == old.root()));
        assert!(all.iter().any(|record| record.root() == new.root()));
        store.delete_attempt_state("game", false).unwrap();
        replace_history(&store, "game", &[]).unwrap();
        assert!(read_all(&store, "game").unwrap().is_empty());
        assert!(old.root().exists());
        assert!(new.root().exists());
    }

    #[test]
    fn migrates_legacy_history_without_retiring_another_colon_subject() {
        let temp = tempfile::tempdir().unwrap();
        let store = RepositoryStore::open(&temp.path().join("db.sqlite")).unwrap();
        let own = install_ownership::allocate_directory(temp.path(), "game", "A").unwrap();
        let other = install_ownership::allocate_directory(temp.path(), "game:other", "B").unwrap();
        let own_key = "download_previous_ownership:game:A";
        let other_key = "download_previous_ownership:game:other:B";
        store
            .set_config(own_key, &serde_json::to_string(&own).unwrap())
            .unwrap();
        store
            .set_config(other_key, &serde_json::to_string(&other).unwrap())
            .unwrap();
        let all = read_all(&store, "game").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), "game");
        replace_history(&store, "game", &all).unwrap();
        assert!(store.get_config(own_key).unwrap().is_none());
        assert!(store.get_config(other_key).unwrap().is_some());
        assert_eq!(read_all(&store, "game").unwrap().len(), 1);
        replace_history(&store, "game", &[]).unwrap();
        assert!(read_all(&store, "game").unwrap().is_empty());
        assert!(store.get_config(other_key).unwrap().is_some());
    }
}
