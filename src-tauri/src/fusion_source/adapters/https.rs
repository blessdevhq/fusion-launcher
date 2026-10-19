use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use super::{
    https_url, ArtifactTransport, Cancellation, TransferLimits, TransferProgress,
    TransferProgressSink, TransferReceipt,
};
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;

/// Reuses the existing connector's DNS policy and manually validated redirects.
/// Archives stream through a bounded queue to a dedicated writer, never a full Vec.
#[derive(Debug, Default)]
pub struct SecureHttpsTransport;

#[async_trait]
impl ArtifactTransport for SecureHttpsTransport {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
    ) -> Result<TransferReceipt, FusionError> {
        self.fetch_with_progress(url, destination, limits, cancel, None)
            .await
    }

    async fn fetch_with_progress(
        &self,
        url: &str,
        destination: &Path,
        limits: &TransferLimits,
        cancel: &Cancellation,
        progress: Option<Arc<dyn TransferProgressSink>>,
    ) -> Result<TransferReceipt, FusionError> {
        limits.validate()?;
        cancel.check()?;
        let url = https_url(url)?;
        let operation = async {
            let mut response = crate::net::get_external(&url, false, limits.timeout)
                .await
                .map_err(|message| FusionError::new(ErrorCode::Network, message))?;
            if response
                .content_length()
                .is_some_and(|size| size > limits.max_bytes)
            {
                return Err(FusionError::new(
                    ErrorCode::TransferLimitExceeded,
                    "The HTTP resource exceeds its byte limit.",
                ));
            }
            let mut writer = TransferWriter::start(destination, limits.max_bytes, cancel.clone())?;
            let mut received = 0u64;
            let total_bytes = response.content_length().unwrap_or(0);
            let started = Instant::now();
            let mut last_progress = None::<Instant>;
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                FusionError::new(ErrorCode::Network, "Reading the HTTP body failed.")
                    .with_detail("cause", error.to_string())
            })? {
                cancel.check()?;
                if chunk.len() as u64 > limits.max_bytes.saturating_sub(received) {
                    return Err(FusionError::new(
                        ErrorCode::TransferLimitExceeded,
                        "The streamed resource exceeds its byte limit.",
                    ));
                }
                received += chunk.len() as u64;
                for piece in chunk.chunks(64 * 1024) {
                    writer.write(piece.to_vec()).await?;
                }
                if let Some(sink) = &progress {
                    let now = Instant::now();
                    if last_progress.is_none_or(|last| now.duration_since(last).as_millis() >= 250)
                    {
                        sink.emit(TransferProgress {
                            downloaded_bytes: received,
                            total_bytes,
                            speed_bytes_per_second: (received as f64
                                / started.elapsed().as_secs_f64().max(0.001))
                                as u64,
                        })?;
                        last_progress = Some(now);
                    }
                }
            }
            let mut completed = writer.finish().await?;
            cancel.check()?;
            if completed.bytes != received {
                return Err(FusionError::new(
                    ErrorCode::Io,
                    "The transfer writer returned an inconsistent byte count.",
                ));
            }
            if let Some(sink) = &progress {
                sink.emit(TransferProgress {
                    downloaded_bytes: received,
                    total_bytes: received,
                    speed_bytes_per_second: (received as f64
                        / started.elapsed().as_secs_f64().max(0.001))
                        as u64,
                })?;
            }
            // Ownership is handed back only after sync_all and all writes end.
            completed.keep = true;
            Ok(TransferReceipt {
                bytes: completed.bytes,
                final_url: response.url().to_string(),
            })
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(FusionError::new(ErrorCode::Cancelled,"Delivery was cancelled.")),
            result = tokio::time::timeout(limits.timeout, operation) => result.map_err(|_| {
                FusionError::new(ErrorCode::Network,"The HTTP transfer deadline expired.").with_detail("timeout",true)
            })?,
        }
    }
}

/// The descriptor, directory guard and deletion authority travel together from
/// the writer to the consumer. Dropping a future deletes its exact uncommitted
/// Windows file by handle, even if the worker completed just before cancellation.
struct PendingFile {
    file: File,
    #[cfg(not(windows))]
    path: std::path::PathBuf,
    _parent: fs_safety::DirectoryGuard,
    bytes: u64,
    keep: bool,
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.keep {
            #[cfg(windows)]
            let _ = crate::install_ownership::delete_open_file(&self.file);
            #[cfg(not(windows))]
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct TransferWriter {
    sender: Option<mpsc::Sender<Vec<u8>>>,
    result: Option<oneshot::Receiver<Result<PendingFile, FusionError>>>,
    worker: Option<JoinHandle<()>>,
    abort: Cancellation,
    finish_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TransferWriter {
    fn start(path: &Path, max_bytes: u64, cancel: Cancellation) -> Result<Self, FusionError> {
        let (sender, receiver) = mpsc::channel(4);
        let (result_sender, result) = oneshot::channel();
        let abort = Cancellation::default();
        let worker_abort = abort.clone();
        let path = path.to_owned();
        let finish_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finish = finish_requested.clone();
        let worker = std::thread::Builder::new()
            .name("fusion-http-writer".into())
            .spawn(move || {
                let result =
                    write_body(&path, max_bytes, receiver, &cancel, &worker_abort, &finish);
                // If the receiver was dropped, the returned PendingFile is dropped
                // here and removes the exact file before the worker exits.
                let _ = result_sender.send(result);
            })
            .map_err(|error| {
                FusionError::new(ErrorCode::Io, format!("Start HTTP writer: {error}"))
            })?;
        Ok(Self {
            sender: Some(sender),
            result: Some(result),
            worker: Some(worker),
            abort,
            finish_requested,
        })
    }

    async fn write(&mut self, bytes: Vec<u8>) -> Result<(), FusionError> {
        self.sender
            .as_ref()
            .expect("writer is open")
            .send(bytes)
            .await
            .map_err(|_| {
                FusionError::new(
                    ErrorCode::Io,
                    "The HTTP writer stopped before completing the body.",
                )
            })
    }

    async fn finish(&mut self) -> Result<PendingFile, FusionError> {
        self.finish_requested
            .store(true, std::sync::atomic::Ordering::Release);
        drop(self.sender.take());
        self.result
            .take()
            .expect("writer result is pending")
            .await
            .map_err(|_| {
                FusionError::new(ErrorCode::Io, "The HTTP writer terminated unexpectedly.")
            })?
    }
}

impl Drop for TransferWriter {
    fn drop(&mut self) {
        self.abort.cancel();
        drop(self.sender.take());
        drop(self.result.take());
        // Normal writes and flushes run entirely off Tokio. Cancellation must
        // await this worker's actual exit; detaching it would permit cleanup or
        // CAS publication while a filesystem writer is still alive.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_body(
    path: &Path,
    max_bytes: u64,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    cancel: &Cancellation,
    abort: &Cancellation,
    finish: &std::sync::atomic::AtomicBool,
) -> Result<PendingFile, FusionError> {
    cancel.check()?;
    abort.check()?;
    let parent = path.parent().ok_or_else(|| {
        FusionError::new(
            ErrorCode::InvalidPath,
            "A delivery destination needs a parent.",
        )
    })?;
    let guard = fs_safety::guard(parent)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80000000 | 0x40000000 | 0x10000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|error| fs_safety::io_error("Create transfer staging", path, error))?;
    let mut pending = PendingFile {
        file,
        #[cfg(not(windows))]
        path: path.to_owned(),
        _parent: guard,
        bytes: 0,
        keep: false,
    };
    while let Some(bytes) = receiver.blocking_recv() {
        cancel.check()?;
        abort.check()?;
        if bytes.len() as u64 > max_bytes.saturating_sub(pending.bytes) {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "The writer byte limit was exceeded.",
            ));
        }
        pending
            .file
            .write_all(&bytes)
            .map_err(|error| fs_safety::io_error("Write transfer staging", path, error))?;
        pending.bytes += bytes.len() as u64;
    }
    cancel.check()?;
    abort.check()?;
    if !finish.load(std::sync::atomic::Ordering::Acquire) {
        return Err(FusionError::new(
            ErrorCode::Cancelled,
            "The HTTP body writer was abandoned.",
        ));
    }
    pending
        .file
        .sync_all()
        .map_err(|error| fs_safety::io_error("Flush transfer staging", path, error))?;
    cancel.check()?;
    abort.check()?;
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_writer_flushes_only_after_all_chunks_and_closes_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("download");
        let mut writer = TransferWriter::start(&path, 1024, Cancellation::default()).unwrap();
        writer.write(b"first ".to_vec()).await.unwrap();
        writer.write(b"second".to_vec()).await.unwrap();
        let mut complete = writer.finish().await.unwrap();
        assert_eq!(complete.bytes, 12);
        complete.keep = true;
        drop(complete);
        drop(writer);
        assert_eq!(std::fs::read(&path).unwrap(), b"first second");
        let _exclusive = OpenOptions::new().write(true).open(&path).unwrap();
    }

    #[tokio::test]
    async fn dropping_writer_joins_io_and_removes_owned_partial_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("download");
        let mut writer = TransferWriter::start(&path, 1024, Cancellation::default()).unwrap();
        writer.write(b"partial".to_vec()).await.unwrap();
        drop(writer);
        assert!(!path.exists());
        std::fs::write(&path, b"future operation").unwrap();
    }

    #[tokio::test]
    async fn writer_never_adopts_or_removes_preexisting_destination() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("download");
        std::fs::write(&path, b"external").unwrap();
        let mut writer = TransferWriter::start(&path, 1024, Cancellation::default()).unwrap();
        assert!(writer.finish().await.is_err());
        drop(writer);
        assert_eq!(std::fs::read(path).unwrap(), b"external");
    }

    #[tokio::test]
    async fn completed_but_uncommitted_writer_result_is_removed_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("download");
        let mut writer = TransferWriter::start(&path, 1024, Cancellation::default()).unwrap();
        writer.write(b"completed".to_vec()).await.unwrap();
        let completed = writer.finish().await.unwrap();
        drop(writer);
        drop(completed);
        assert!(!path.exists());
    }
}
