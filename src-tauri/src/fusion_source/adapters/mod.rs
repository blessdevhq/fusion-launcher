//! Transport primitives. Implementations own their writers and must close them before
//! returning, including cancellation; these interfaces never mutate installation state.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;

use super::errors::{ErrorCode, FusionError};

mod https;
pub use https::SecureHttpsTransport;
mod cloud;
pub use cloud::{validate_cloud_uri, PixeldrainUriResolver};
pub mod storefront;
pub mod torrent;
pub mod torrent_relay;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub downloaded_bytes: u64,
    /// Zero means the server has not supplied a total yet.
    pub total_bytes: u64,
    pub speed_bytes_per_second: u64,
}

/// Transfer callbacks carry no operation identity; the service binds that authority.
pub trait TransferProgressSink: Send + Sync {
    fn emit(&self, progress: TransferProgress) -> Result<(), FusionError>;
}

#[derive(Clone, Default, Debug)]
pub struct Cancellation {
    inner: Arc<CancellationInner>,
}

#[derive(Default, Debug)]
struct CancellationInner {
    cancelled: AtomicBool,
    changed: Notify,
}

impl Cancellation {
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.changed.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), FusionError> {
        if self.is_cancelled() {
            Err(FusionError::new(
                ErrorCode::Cancelled,
                "Delivery was cancelled.",
            ))
        } else {
            Ok(())
        }
    }

    pub async fn cancelled(&self) {
        loop {
            let notification = self.inner.changed.notified();
            // Register before testing the flag, avoiding a lost notify_waiters.
            tokio::pin!(notification);
            notification.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notification.await;
        }
    }
}

#[derive(Clone, Debug)]
pub struct TransferLimits {
    pub max_bytes: u64,
    pub timeout: Duration,
}

impl TransferLimits {
    pub fn validate(&self) -> Result<(), FusionError> {
        if self.max_bytes == 0 || self.timeout.is_zero() {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "Delivery requires a nonzero byte limit and deadline.",
            ));
        }
        Ok(())
    }

    pub fn json() -> Self {
        Self {
            max_bytes: crate::net::JSON_BODY_LIMIT as u64,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferReceipt {
    pub bytes: u64,
    pub final_url: String,
}

/// Destination is an absent, explicitly owned staging file. A failed transfer may
/// leave that file, but MUST stop all writes before returning. No detached writers
/// may survive dropping the future. Enforce the supplied limits in the adapter.
#[async_trait]
pub trait ArtifactTransport: Send + Sync {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError>;

    async fn fetch_with_progress(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        _progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        self.fetch(url, destination, limits, cancel).await
    }
}

pub(crate) fn https_url(value: &str) -> Result<url::Url, FusionError> {
    let url = url::Url::parse(value).map_err(|_| {
        FusionError::new(ErrorCode::InvalidUri, "An absolute HTTPS URL is required.")
    })?;
    crate::security::validate_http_url(&url, false)
        .map_err(|message| FusionError::new(ErrorCode::InvalidUri, message))?;
    if url.scheme() != "https" {
        return Err(FusionError::new(
            ErrorCode::InvalidUri,
            "HTTPS is required.",
        ));
    }
    Ok(url)
}
