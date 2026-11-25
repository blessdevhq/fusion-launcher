//! Per-subject ownership. The final lease release, not a cancellation flag,
//! confirms that all cooperating filesystem writers have stopped.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

#[derive(Clone, Default)]
pub struct OperationCoordinator(Arc<Mutex<HashMap<String, Slot>>>);

enum Slot {
    Active {
        attempt: String,
        cancel: watch::Sender<bool>,
        done: watch::Receiver<bool>,
        deleting: bool,
    },
    Deleting,
}

#[derive(Clone)]
pub struct OperationLease(Arc<Owner>);

struct Owner {
    id: String,
    attempt: String,
    cancel: watch::Receiver<bool>,
    done: watch::Sender<bool>,
    registry: OperationCoordinator,
    installation: Mutex<Option<crate::install_ownership::InstallationOwnership>>,
}

pub struct DeleteTicket {
    id: String,
    registry: OperationCoordinator,
    done: Option<watch::Receiver<bool>>,
}

fn next_attempt() -> String {
    static NONCE: OnceLock<String> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let nonce = NONCE.get_or_init(|| {
        format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    format!("{nonce}-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

impl OperationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, id: &str) -> Result<OperationLease, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("Download id cannot be empty.".into());
        }
        let mut slots = self
            .0
            .lock()
            .map_err(|_| "Download coordinator lock poisoned.")?;
        if slots.contains_key(id) {
            return Err(format!("Download is active or being removed: {id}"));
        }
        let attempt = next_attempt();
        let (cancel, receiver) = watch::channel(false);
        let (done, completion) = watch::channel(false);
        slots.insert(
            id.into(),
            Slot::Active {
                attempt: attempt.clone(),
                cancel,
                done: completion,
                deleting: false,
            },
        );
        Ok(OperationLease(Arc::new(Owner {
            id: id.into(),
            attempt,
            cancel: receiver,
            done,
            registry: self.clone(),
            installation: Mutex::new(None),
        })))
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let slots = self
            .0
            .lock()
            .map_err(|_| "Download coordinator lock poisoned.")?;
        if let Some(Slot::Active { cancel, .. }) = slots.get(id) {
            cancel.send_replace(true);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn ensure_active(&self, lease: &OperationLease) -> Result<(), String> {
        let slots = self
            .0
            .lock()
            .map_err(|_| "Download coordinator lock poisoned.")?;
        match slots.get(&lease.0.id) {
            Some(Slot::Active {
                attempt,
                deleting: false,
                ..
            }) if attempt == lease.attempt() && !lease.is_cancelled() => Ok(()),
            _ => Err("Download cancelled or being removed.".into()),
        }
    }

    pub fn begin_delete(&self, id: &str) -> Result<DeleteTicket, String> {
        let mut slots = self
            .0
            .lock()
            .map_err(|_| "Download coordinator lock poisoned.")?;
        let done = match slots.get_mut(id) {
            Some(Slot::Deleting | Slot::Active { deleting: true, .. }) => {
                return Err("Download removal already in progress.".into())
            }
            Some(Slot::Active {
                cancel,
                done,
                deleting,
                ..
            }) => {
                *deleting = true;
                cancel.send_replace(true);
                Some(done.clone())
            }
            None => {
                slots.insert(id.into(), Slot::Deleting);
                None
            }
        };
        Ok(DeleteTicket {
            id: id.into(),
            registry: self.clone(),
            done,
        })
    }
}

impl OperationLease {
    pub(crate) fn set_installation(
        &self,
        record: &crate::install_ownership::InstallationOwnership,
    ) -> Result<(), String> {
        *self
            .0
            .installation
            .lock()
            .map_err(|_| "Installation lease poisoned.")? = Some(record.clone());
        Ok(())
    }
    pub(crate) fn protect_writes(
        &self,
    ) -> Result<Option<crate::install_ownership::WriteProtection>, String> {
        self.0
            .installation
            .lock()
            .map_err(|_| "Installation lease poisoned.")?
            .as_ref()
            .map(crate::install_ownership::protect_writes)
            .transpose()
    }
    pub fn attempt(&self) -> &str {
        &self.0.attempt
    }
    pub(crate) fn protect_torrent_writes(
        &self,
    ) -> Result<crate::install_ownership::WriteProtection, String> {
        let installation = self
            .0
            .installation
            .lock()
            .map_err(|_| "Installation lease poisoned.")?;
        let record = installation.as_ref().ok_or("Torrent ownership missing.")?;
        crate::install_ownership::protect_torrent_writes(record)
    }
    pub fn token(&self) -> Self {
        self.clone()
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.cancel.borrow()
    }
    pub async fn cancelled(&self) {
        let mut rx = self.0.cancel.clone();
        let _ = rx.wait_for(|value| *value).await;
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.registry.0.lock() {
            if let Some(Slot::Active {
                attempt, deleting, ..
            }) = slots.get(&self.id)
            {
                if attempt == &self.attempt {
                    if *deleting {
                        slots.insert(self.id.clone(), Slot::Deleting);
                    } else {
                        slots.remove(&self.id);
                    }
                }
            }
        }
        self.done.send_replace(true);
    }
}

impl DeleteTicket {
    pub async fn wait_owner(&self, limit: Duration) -> Result<(), String> {
        let Some(mut rx) = self.done.clone() else {
            return Ok(());
        };
        let result = tokio::time::timeout(limit, rx.wait_for(|done| *done)).await;
        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => Err("Download owner disappeared without completion.".into()),
            Err(_) => Err("Download stop timed out; files and records preserved.".into()),
        }
    }
}

impl Drop for DeleteTicket {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.registry.0.lock() {
            match slots.get_mut(&self.id) {
                Some(Slot::Deleting) => {
                    slots.remove(&self.id);
                }
                Some(Slot::Active { deleting, .. }) => *deleting = false,
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn removal_waits_for_all_writers_and_blocks_restart_until_cleanup() {
        let registry = OperationCoordinator::new();
        let owner = registry.start("game").unwrap();
        let writer = owner.clone();
        let old_attempt = owner.attempt().to_string();
        let ticket = registry.begin_delete("game").unwrap();
        assert!(owner.is_cancelled());
        drop(owner);
        assert!(ticket.wait_owner(Duration::from_millis(1)).await.is_err());
        assert!(registry.start("game").is_err());
        drop(writer);
        ticket.wait_owner(Duration::from_secs(1)).await.unwrap();
        assert!(registry.start("game").is_err());
        drop(ticket);
        assert_ne!(registry.start("game").unwrap().attempt(), old_attempt);
    }
    #[tokio::test]
    async fn timeout_does_not_release_running_owner_and_retry_observes_it() {
        let registry = OperationCoordinator::new();
        let owner = registry.start("game").unwrap();
        let ticket = registry.begin_delete("game").unwrap();
        assert!(ticket.wait_owner(Duration::from_millis(1)).await.is_err());
        drop(ticket);
        assert!(registry.start("game").is_err());
        let retry = registry.begin_delete("game").unwrap();
        drop(owner);
        retry.wait_owner(Duration::from_secs(1)).await.unwrap();
    }
}
