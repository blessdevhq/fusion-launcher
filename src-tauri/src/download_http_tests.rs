use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn stalled_response(headers_only: bool) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = socket.read(&mut buf).await;
        let reply = if headers_only {
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\n".as_slice()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc".as_slice()
        };
        socket.write_all(reply).await.unwrap();
        std::future::pending::<()>().await;
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{addr}/file"))
        .send()
        .await
        .unwrap();
    (response, task)
}

fn options() -> StreamOptions<'static> {
    StreamOptions {
        expected_sha256: None,
        expected_size_bytes: Some(6),
        max_bytes: None,
        resume: true,
    }
}

#[tokio::test]
async fn http_releases_library_ancestor_handles_after_terminal_paths_and_panic() {
    for terminal in ["success", "cancel", "error", "panic"] {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        std::fs::create_dir(&library).unwrap();
        let registry = DirectDownloadRegistry::new();
        let owner = registry.start("game").unwrap();
        let ownership =
            crate::install_ownership::allocate_directory(&library, "game", owner.attempt())
                .unwrap();
        owner.set_installation(&ownership).unwrap();
        let root = ownership.root().to_path_buf();
        let destination = root.join("game.bin");
        let part = part_path_for(&destination);
        let (file, hasher, offset) = prepare_partial(&part, true, Some(&owner)).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 2048];
            let _ = socket.read(&mut buffer).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabcdef")
                .await
                .unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/file"))
            .send()
            .await
            .unwrap();
        let task = tokio::spawn(async move {
            let mut expected = options();
            if terminal == "error" {
                expected.expected_size_bytes = Some(999);
            }
            write_http_response(
                response,
                DownloadSession {
                    destination,
                    part,
                    file,
                    hasher,
                    offset,
                },
                expected,
                move |_, _| {
                    assert_ne!(terminal, "panic", "injected progress panic");
                },
                move || terminal == "cancel",
                Duration::from_secs(1),
            )
            .await
        });
        let result = task.await;
        if terminal == "success" {
            assert!(result.unwrap().is_ok());
        } else if terminal == "panic" {
            assert!(result.unwrap_err().is_panic());
        } else {
            assert!(result.unwrap().is_err());
        }
        server.await.unwrap();
        // The lease may stay alive for status/history, but carries no path lock.
        let moved = temp.path().join("released");
        std::fs::rename(&library, &moved).unwrap();
        let install = moved.join(root.file_name().unwrap());
        for entry in std::fs::read_dir(&install).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(install).unwrap();
        std::fs::remove_dir(moved).unwrap();
        drop(owner);
    }
}

#[tokio::test]
async fn stalled_body_preserves_partial_and_closes_file_for_retry() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let destination = temp.path().join("game.bin");
    let part = part_path_for(&destination);
    let (file, hasher, offset) = prepare_partial(&part, true, None).await.unwrap();
    let (response, server) = stalled_response(false).await;
    let error = write_http_response(
        response,
        DownloadSession {
            destination: destination.clone(),
            part: part.clone(),
            file,
            hasher,
            offset,
        },
        options(),
        |_, _| {},
        || false,
        Duration::from_millis(80),
    )
    .await
    .unwrap_err();
    assert!(error.contains("stalled"));
    assert_eq!(std::fs::read(&part).unwrap(), b"abc");
    assert!(!destination.exists());
    std::fs::rename(&part, temp.path().join("retry.part")).unwrap();
    server.abort();
}

#[tokio::test]
async fn cancellation_preserves_guarded_partial_for_remove_without_files() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let registry = DirectDownloadRegistry::new();
    let owner = registry.start("game").unwrap();
    let ownership =
        crate::install_ownership::allocate_directory(temp.path(), "game", owner.attempt()).unwrap();
    owner.set_installation(&ownership).unwrap();
    let destination = ownership.root().join("game.bin");
    let part = part_path_for(&destination);
    let (file, hasher, offset) = prepare_partial(&part, true, Some(&owner)).await.unwrap();
    let (response, server) = stalled_response(false).await;
    let cancel = registry.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel.cancel("game").unwrap();
    });
    let token = owner.clone();
    let error = write_http_response(
        response,
        DownloadSession {
            destination,
            part: part.clone(),
            file,
            hasher,
            offset,
        },
        options(),
        |_, _| {},
        move || token.is_cancelled(),
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert!(is_download_cancelled_error(&error));
    assert_eq!(std::fs::read(&part).unwrap(), b"abc");
    std::fs::rename(&part, ownership.root().join("unlocked.part")).unwrap();
    server.abort();
}

#[tokio::test]
async fn pending_headers_are_cancellable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let flag = Arc::new(AtomicBool::new(false));
    let cancel = Arc::clone(&flag);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.store(true, Ordering::SeqCst);
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let error = download_step(
        client.get(format!("http://{addr}/file")).send(),
        Duration::from_secs(2),
        &mut || flag.load(Ordering::SeqCst),
    )
    .await
    .unwrap_err();
    assert!(is_download_cancelled_error(&error));
    server.abort();
}

#[tokio::test]
async fn replaced_or_aliased_partial_is_never_resumed() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let registry = DirectDownloadRegistry::new();
    let owner = registry.start("game").unwrap();
    let mut ownership =
        crate::install_ownership::allocate_directory(temp.path(), "game", owner.attempt()).unwrap();
    let part = ownership.root().join("game.bin.part");
    std::fs::write(&part, b"owned").unwrap();
    crate::install_ownership::snapshot_files(&mut ownership).unwrap();
    owner.set_installation(&ownership).unwrap();
    std::fs::remove_file(&part).unwrap();
    let foreign = temp.path().join("foreign");
    std::fs::write(&foreign, b"keep").unwrap();
    std::fs::hard_link(&foreign, &part).unwrap();
    assert!(prepare_partial(&part, true, Some(&owner)).await.is_err());
    assert_eq!(std::fs::read(foreign).unwrap(), b"keep");
}

#[tokio::test]
async fn resume_ignored_200_restarts_counters_and_publishes_without_overwrite() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let destination = temp.path().join("game.bin");
    let part = part_path_for(&destination);
    std::fs::write(&part, b"old").unwrap();
    let (file, hasher, offset) = prepare_partial(&part, true, None).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 1024];
        let _ = socket.read(&mut buf).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabcdef")
            .await
            .unwrap();
    });
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap();
    let mut totals = Vec::new();
    write_http_response(
        response,
        DownloadSession {
            destination: destination.clone(),
            part,
            file,
            hasher,
            offset,
        },
        options(),
        |done, total| totals.push((done, total)),
        || false,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"abcdef");
    assert!(totals.iter().all(|(_, t)| *t == Some(6)));
    server.await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn publication_retains_verified_handle_across_source_substitution_attempt() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let part = temp.path().join("payload.part");
    let destination = temp.path().join("payload.bin");
    std::fs::write(&part, b"verified").unwrap();
    let (file, hasher, size) = prepare_partial(&part, true, None).await.unwrap();
    assert_eq!(size, 8);
    assert_eq!(
        hex::encode(hasher.finalize()),
        hex::encode(Sha256::digest(b"verified"))
    );

    // This is the former vulnerable boundary: hashing has finished but the
    // final name has not been published. The retained descriptor prevents both
    // replacement and modification, and publication uses that same handle.
    assert!(std::fs::rename(&part, temp.path().join("original.part")).is_err());
    assert!(std::fs::write(&part, b"foreign!").is_err());
    crate::install_ownership::publish_rename_no_replace(&file.file, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"verified");
    assert!(!part.exists());
    drop(file);
}

#[cfg(windows)]
#[tokio::test]
async fn publication_never_replaces_a_destination_created_after_verification() {
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let part = temp.path().join("payload.part");
    let destination = temp.path().join("payload.bin");
    std::fs::write(&part, b"verified").unwrap();
    let (file, _, _) = prepare_partial(&part, true, None).await.unwrap();
    std::fs::write(&destination, b"foreign").unwrap();
    assert!(crate::install_ownership::publish_rename_no_replace(&file.file, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"foreign");
    assert_eq!(std::fs::read(&part).unwrap(), b"verified");
}
