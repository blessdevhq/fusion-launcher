use crate::download_operations::OperationLease;
use std::future::Future;

const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Cancellation remains observable while DNS, headers or a body chunk stalls.
async fn download_step<T>(
    future: impl Future<Output = T>,
    limit: Duration,
    cancel: &mut impl FnMut() -> bool,
) -> Result<T, String> {
    tokio::pin!(future);
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(Duration::from_millis(25));
    loop {
        if cancel() {
            return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        tokio::select! {biased;
            _ = poll.tick()=>{},
            result=&mut future=>return Ok(result),
            _=&mut deadline=>return Err("Download stalled: no data within the timeout.".into()),
        }
    }
}

pub async fn download_source_to_file_guarded(
    source: &SourceUri,
    destination: &Path,
    on_progress: impl FnMut(u64, Option<u64>),
    lease: &OperationLease,
) -> Result<DownloadedFile, String> {
    let token = lease.clone();
    match source {
        SourceUri::Http {
            url,
            sha256,
            size_bytes,
        } => {
            download_http_streaming_inner(
                url,
                destination,
                StreamOptions {
                    expected_sha256: if sha256.is_empty() {
                        None
                    } else {
                        Some(sha256)
                    },
                    expected_size_bytes: *size_bytes,
                    max_bytes: None,
                    resume: true,
                },
                on_progress,
                move || token.is_cancelled(),
                Some(lease),
            )
            .await
        }
        SourceUri::Bundled {
            path,
            sha256,
            size_bytes,
        } => {
            let path = path.clone();
            let sha = sha256.clone();
            let size = *size_bytes;
            let destination = destination.to_path_buf();
            let held = lease.clone();
            tokio::task::spawn_blocking(move || {
                if held.is_cancelled() {
                    return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
                }
                let _protection = held.protect_writes()?;
                let bytes = builtin_demo::asset_bytes(&path).ok_or("Unknown bundled asset.")?;
                let actual = hex::encode(Sha256::digest(bytes));
                if !actual.eq_ignore_ascii_case(&sha) {
                    return Err("Bundled asset SHA256 mismatch.".into());
                }
                validate_size(bytes.len() as u64, size)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .map_err(|e| e.to_string())?;
                std::io::Write::write_all(&mut file, bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                Ok(DownloadedFile {
                    path: destination,
                    sha256: actual,
                })
            })
            .await
            .map_err(|e| e.to_string())?
        }
        _ => Err("Only direct and bundled sources use the guarded downloader.".into()),
    }
}

pub async fn download_http_streaming(
    url: &str,
    destination: &Path,
    options: StreamOptions<'_>,
    on_progress: impl FnMut(u64, Option<u64>),
    is_cancelled: impl FnMut() -> bool,
) -> Result<DownloadedFile, String> {
    download_http_streaming_inner(url, destination, options, on_progress, is_cancelled, None).await
}

async fn download_http_streaming_inner(
    url: &str,
    destination: &Path,
    options: StreamOptions<'_>,
    on_progress: impl FnMut(u64, Option<u64>),
    mut is_cancelled: impl FnMut() -> bool,
    lease: Option<&OperationLease>,
) -> Result<DownloadedFile, String> {
    let parsed = Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https" {
        return Err("Insecure URL scheme: only HTTPS is permitted.".into());
    }
    if parsed
        .host_str()
        .is_some_and(crate::net::is_forbidden_target_host)
    {
        return Err("Download target host is forbidden.".into());
    }
    if lease.is_none() {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    let part = part_path_for(destination);
    if is_cancelled() {
        return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    let (file, hasher, offset) = prepare_partial(&part, options.resume, lease).await?;
    let mut headers = reqwest::header::HeaderMap::new();
    if offset > 0 {
        headers.insert(
            RANGE,
            format!("bytes={offset}-")
                .parse()
                .map_err(|_| "Invalid range header.")?,
        );
    }
    let response = download_step(
        crate::net::get_external_with_headers(&parsed, false, DOWNLOAD_CONNECT_TIMEOUT, headers),
        DOWNLOAD_CONNECT_TIMEOUT,
        &mut is_cancelled,
    )
    .await;
    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(error),
        Err(error) => {
            drop(file);
            return Err(error);
        }
    };
    write_http_response(
        response,
        DownloadSession {
            destination: destination.to_path_buf(),
            part,
            hasher,
            offset,
            file,
        },
        options,
        on_progress,
        is_cancelled,
        DOWNLOAD_IDLE_TIMEOUT,
    )
    .await
}

struct LeasedFile {
    // Drop order is intentional: descriptors and path locks close before done.
    file: std::fs::File,
    protection: Option<crate::install_ownership::WriteProtection>,
    lease: Option<OperationLease>,
}
struct DownloadSession {
    destination: PathBuf,
    part: PathBuf,
    hasher: Sha256,
    offset: u64,
    file: LeasedFile,
}

async fn write_http_response(
    mut response: reqwest::Response,
    mut session: DownloadSession,
    options: StreamOptions<'_>,
    mut on_progress: impl FnMut(u64, Option<u64>),
    mut is_cancelled: impl FnMut() -> bool,
    idle_timeout: Duration,
) -> Result<DownloadedFile, String> {
    let append = session.offset > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if append {
        let range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|s| s.to_str().ok())
            .ok_or("Resume response lacks Content-Range.")?;
        let start = range
            .strip_prefix("bytes ")
            .and_then(|s| s.split_once('-'))
            .and_then(|(s, _)| s.parse::<u64>().ok());
        if start != Some(session.offset) {
            return Err("Resume response has incorrect Content-Range.".into());
        }
    } else {
        if response.status() == StatusCode::PARTIAL_CONTENT {
            return Err("Unexpected partial HTTP response.".into());
        }
        session.offset = 0;
        session.hasher = Sha256::new();
    }
    response = response.error_for_status().map_err(|e| e.to_string())?;
    let total = response
        .content_length()
        .and_then(|len| len.checked_add(session.offset))
        .or(options.expected_size_bytes);
    let mut downloaded = session.offset;
    let mut file = session.file;
    if !append {
        file.file.set_len(0).map_err(|e| e.to_string())?;
    }
    std::io::Seek::seek(&mut file.file, std::io::SeekFrom::Start(session.offset))
        .map_err(|e| e.to_string())?;
    on_progress(downloaded, total);
    let mut last = Instant::now();
    loop {
        let next = download_step(response.chunk(), idle_timeout, &mut is_cancelled).await;
        let chunk = match next {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => return Err(format!("Failed to read source body: {error}")),
            Err(error) => {
                drop(file);
                return Err(error);
            }
        };
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or("Download byte count overflow.")?;
        if options.max_bytes.is_some_and(|max| downloaded > max) {
            return Err("Download exceeded the size limit.".into());
        }
        session.hasher.update(&chunk);
        let (returned, result) = tokio::task::spawn_blocking(move || {
            let result = std::io::Write::write_all(&mut file.file, &chunk);
            (file, result)
        })
        .await
        .map_err(|e| e.to_string())?;
        file = returned;
        result.map_err(|e| e.to_string())?;
        if last.elapsed() >= PROGRESS_REPORT_INTERVAL {
            on_progress(downloaded, total);
            last = Instant::now();
        }
    }
    if is_cancelled() {
        return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
    }
    validate_size(downloaded, options.expected_size_bytes)?;
    let sha = hex::encode(session.hasher.finalize());
    if options
        .expected_sha256
        .is_some_and(|expected| !sha.eq_ignore_ascii_case(expected))
    {
        return Err(format!("SHA256 mismatch: got {sha}"));
    }
    // Atomic no-replace publication. A newly appeared destination is never overwritten.
    let _part = session.part;
    let destination = session.destination.clone();
    tokio::task::spawn_blocking(move || {
        let LeasedFile {
            mut file,
            protection,
            lease,
        } = file;
        std::io::Write::flush(&mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        if lease.as_ref().is_some_and(|l| l.is_cancelled()) {
            return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
        }
        crate::install_ownership::publish_rename_no_replace(&file, &destination)?;
        drop(file);
        drop(protection);
        drop(lease);
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    on_progress(downloaded, total);
    Ok(DownloadedFile {
        path: session.destination,
        sha256: sha,
    })
}

async fn prepare_partial(
    path: &Path,
    resume: bool,
    lease: Option<&OperationLease>,
) -> Result<(LeasedFile, Sha256, u64), String> {
    let path = path.to_path_buf();
    let lease = lease.cloned();
    tokio::task::spawn_blocking(move || {
        let protection = lease
            .as_ref()
            .map(|l| l.protect_writes())
            .transpose()?
            .flatten();
        let exists = std::fs::symlink_metadata(&path).is_ok();
        if exists
            && std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("Partial is a symlink.".into());
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(!exists);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options
                .access_mode(0xC0010000)
                .share_mode(1)
                .custom_flags(0x00200000);
        }
        let file = options.open(&path).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if file
                .metadata()
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Partial is a reparse point.".into());
            }
        }
        if exists && lease.is_some() {
            protection
                .as_ref()
                .ok_or("Resume ownership missing.")?
                .verify_partial(&path, &file)?;
        }
        let mut file = LeasedFile {
            file,
            protection,
            lease,
        };
        let mut hasher = Sha256::new();
        let mut total = 0;
        let mut buffer = [0u8; 64 * 1024];
        if exists && resume {
            loop {
                if file.lease.as_ref().is_some_and(|l| l.is_cancelled()) {
                    return Err(DOWNLOAD_CANCELLED_MESSAGE.into());
                }
                let n = file.file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                total += n as u64;
            }
        }
        Ok((file, hasher, total))
    })
    .await
    .map_err(|e| e.to_string())?
}
