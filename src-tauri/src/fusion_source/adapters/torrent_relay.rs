//! Per-artifact BEP 19 relay. Native libtorrent sees only opaque loopback URLs;
//! every upstream byte uses Fusion's checked HTTPS connector and redirect policy.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use rand::RngCore;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

use super::{https_url, Cancellation, TransferLimits};
use crate::fusion_source::errors::{ErrorCode, FusionError};

type RelayBody = http_body_util::combinators::UnsyncBoxBody<Bytes, io::Error>;
const MAX_CONNECTIONS: usize = 8;
const MAX_REQUESTS: usize = 4096;
const MAX_HEADER_BYTES: usize = 8192;

struct RelayState {
    mappings: HashMap<String, url::Url>,
    size: u64,
    deadline: Instant,
    cancel: Cancellation,
    upstream: Arc<dyn RelayUpstream>,
}

#[async_trait]
trait RelayUpstream: Send + Sync {
    async fn fetch(
        &self,
        url: &url::Url,
        timeout: Duration,
        headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Response, String>;
}

struct SecureRelayUpstream;
#[async_trait]
impl RelayUpstream for SecureRelayUpstream {
    async fn fetch(
        &self,
        url: &url::Url,
        timeout: Duration,
        headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Response, String> {
        crate::net::get_external_with_headers(url, false, timeout, headers).await
    }
}

pub(crate) struct WebSeedRelay {
    pub(crate) seeds: Vec<String>,
    cancel: Cancellation,
    task: Option<JoinHandle<()>>,
}

impl WebSeedRelay {
    pub(crate) async fn start(
        urls: &[String],
        size: u64,
        limits: &TransferLimits,
    ) -> Result<Self, FusionError> {
        Self::start_with_upstream(urls, size, limits, Arc::new(SecureRelayUpstream)).await
    }

    async fn start_with_upstream(
        urls: &[String],
        size: u64,
        limits: &TransferLimits,
        upstream: Arc<dyn RelayUpstream>,
    ) -> Result<Self, FusionError> {
        limits.validate()?;
        if size > limits.max_bytes || urls.len() > 32 {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "WebSeed limits exceeded.",
            ));
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| {
                FusionError::new(
                    ErrorCode::Network,
                    format!("Cannot bind WebSeed relay: {error}"),
                )
            })?;
        let address = listener
            .local_addr()
            .map_err(|error| FusionError::new(ErrorCode::Network, error.to_string()))?;
        let mut mappings = HashMap::new();
        let mut seeds = Vec::new();
        for value in urls {
            let upstream = https_url(value)?;
            let mut token = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut token);
            let path = format!("/{}", hex::encode(token));
            seeds.push(format!("http://{address}{path}"));
            mappings.insert(path, upstream);
        }
        let cancel = Cancellation::default();
        let state = Arc::new(RelayState {
            mappings,
            size,
            deadline: Instant::now() + limits.timeout,
            cancel: cancel.clone(),
            upstream,
        });
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let mut accepted = 0;
            loop {
                tokio::select! {
                    _ = state.cancel.cancelled() => break,
                    _ = tokio::time::sleep_until(state.deadline) => break,
                    _ = connections.join_next(), if !connections.is_empty() => {},
                    result = listener.accept(), if connections.len() < MAX_CONNECTIONS && accepted < MAX_REQUESTS => {
                        let Ok((socket, peer)) = result else { break; };
                        if !peer.ip().is_loopback() { continue; }
                        accepted += 1;
                        let shared = Arc::clone(&state);
                        connections.spawn(async move {
                            let handler_state = Arc::clone(&shared);
                            let service = service_fn(move |request| serve(request, Arc::clone(&handler_state)));
                            let mut builder = hyper::server::conn::http1::Builder::new();
                            builder.keep_alive(false).max_headers(16).max_buf_size(MAX_HEADER_BYTES)
                                .timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(5));
                            let connection = builder.serve_connection(TokioIo::new(socket), service);
                            tokio::select! {
                                _ = shared.cancel.cancelled() => {},
                                _ = tokio::time::sleep_until(shared.deadline) => {},
                                _ = connection => {},
                            }
                        });
                    }
                }
                if accepted >= MAX_REQUESTS && connections.is_empty() {
                    break;
                }
            }
            // Dropping a connection drops its response stream and in-flight
            // upstream request. Join every owned task before acknowledging stop.
            state.cancel.cancel();
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Ok(Self {
            seeds,
            cancel,
            task: Some(task),
        })
    }

    pub(crate) async fn stop(mut self) -> Result<(), FusionError> {
        self.cancel.cancel();
        self.task
            .take()
            .expect("relay task exists")
            .await
            .map_err(|error| {
                FusionError::new(
                    ErrorCode::ProcessSupervisionFailed,
                    format!("WebSeed relay shutdown failed: {error}"),
                )
            })
    }
}

impl Drop for WebSeedRelay {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn failure(status: StatusCode) -> Response<RelayBody> {
    let body = Full::new(Bytes::new())
        .map_err(|never: Infallible| match never {})
        .boxed_unsync();
    Response::builder()
        .status(status)
        .header(hyper::header::CONNECTION, "close")
        .body(body)
        .expect("constant response")
}

fn requested_range(value: &str, size: u64) -> Result<(u64, u64), ()> {
    let (start, end) = value
        .strip_prefix("bytes=")
        .ok_or(())?
        .split_once('-')
        .ok_or(())?;
    // Suffix/multiple ranges are intentionally unsupported. BEP 19 uses concrete
    // piece offsets; permitting arbitrary range syntax adds no capability.
    if start.is_empty()
        || size == 0
        || !start.bytes().all(|b| b.is_ascii_digit())
        || !end.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(());
    }
    let start: u64 = start.parse().map_err(|_| ())?;
    let end: u64 = if end.is_empty() {
        size - 1
    } else {
        end.parse().map_err(|_| ())?
    };
    if start > end || end >= size {
        return Err(());
    }
    Ok((start, end))
}

fn valid_response(
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
    range: Option<(u64, u64)>,
    size: u64,
) -> Result<u64, ()> {
    if headers.contains_key(reqwest::header::CONTENT_ENCODING) {
        return Err(());
    }
    let expected = match range {
        Some((start, end)) => {
            if status != StatusCode::PARTIAL_CONTENT {
                return Err(());
            }
            let expected_header = format!("bytes {start}-{end}/{size}");
            if headers
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                != Some(expected_header.as_str())
            {
                return Err(());
            }
            end - start + 1
        }
        None => {
            if status != StatusCode::OK || headers.contains_key(reqwest::header::CONTENT_RANGE) {
                return Err(());
            }
            size
        }
    };
    if let Some(length) = headers.get(reqwest::header::CONTENT_LENGTH) {
        if length.to_str().ok().and_then(|v| v.parse::<u64>().ok()) != Some(expected) {
            return Err(());
        }
    }
    Ok(expected)
}

async fn serve(
    request: Request<Incoming>,
    state: Arc<RelayState>,
) -> Result<Response<RelayBody>, Infallible> {
    if request.method() != hyper::Method::GET
        || request.uri().query().is_some()
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().len() > 16
        || request
            .headers()
            .iter()
            .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
            .sum::<usize>()
            > MAX_HEADER_BYTES
        || request
            .headers()
            .contains_key(hyper::header::TRANSFER_ENCODING)
        || request
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .is_some_and(|value| value != "0")
    {
        return Ok(failure(StatusCode::BAD_REQUEST));
    }
    let Some(url) = state.mappings.get(request.uri().path()) else {
        return Ok(failure(StatusCode::NOT_FOUND));
    };
    let range = match request.headers().get(hyper::header::RANGE) {
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|value| requested_range(value, state.size).ok())
        {
            Some(range) => Some(range),
            None => return Ok(failure(StatusCode::RANGE_NOT_SATISFIABLE)),
        },
        None => None,
    };
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some((start, end)) = range {
        headers.insert(
            reqwest::header::RANGE,
            format!("bytes={start}-{end}")
                .parse()
                .expect("numeric range"),
        );
    }
    let Some(timeout) = state.deadline.checked_duration_since(Instant::now()) else {
        return Ok(failure(StatusCode::GATEWAY_TIMEOUT));
    };
    let upstream = tokio::select! {
        _ = state.cancel.cancelled() => return Ok(failure(StatusCode::SERVICE_UNAVAILABLE)),
        response = state.upstream.fetch(url, timeout, headers) => match response {
            Ok(response) => response, Err(_) => return Ok(failure(StatusCode::BAD_GATEWAY)),
        }
    };
    let Ok(expected) = valid_response(upstream.status(), upstream.headers(), range, state.size)
    else {
        return Ok(failure(StatusCode::BAD_GATEWAY));
    };
    let body_state = Arc::clone(&state);
    let stream = stream::try_unfold(
        (upstream, 0u64, false),
        move |(mut response, count, done)| {
            let state = Arc::clone(&body_state);
            async move {
                if done {
                    return Ok(None);
                }
                let chunk = tokio::select! {
                    _ = state.cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted,"Relay cancelled")),
                    _ = tokio::time::sleep_until(state.deadline) => return Err(io::Error::new(io::ErrorKind::TimedOut,"Relay deadline exceeded")),
                    chunk = response.chunk() => chunk.map_err(io::Error::other)?,
                };
                match chunk {
                    Some(bytes) => {
                        let next = count
                            .checked_add(bytes.len() as u64)
                            .ok_or_else(|| io::Error::other("WebSeed byte counter overflow"))?;
                        if next > expected {
                            return Err(io::Error::other(
                                "WebSeed response exceeds declared range",
                            ));
                        }
                        Ok(Some((Frame::data(bytes), (response, next, false))))
                    }
                    None if count == expected => Ok(None),
                    None => Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Truncated WebSeed response",
                    )),
                }
            }
        },
    );
    let body = StreamBody::new(Box::pin(stream)).boxed_unsync();
    let mut builder = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(hyper::header::CONTENT_LENGTH, expected)
        .header(hyper::header::CONNECTION, "close");
    if let Some((start, end)) = range {
        builder = builder.header(
            hyper::header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", state.size),
        );
    }
    Ok(builder.body(body).expect("validated response headers"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct FixtureUpstream(String);
    #[async_trait]
    impl RelayUpstream for FixtureUpstream {
        async fn fetch(
            &self,
            _: &url::Url,
            timeout: Duration,
            headers: reqwest::header::HeaderMap,
        ) -> Result<reqwest::Response, String> {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap()
                .get(&self.0)
                .timeout(timeout)
                .headers(headers)
                .send()
                .await
                .map_err(|error| error.to_string())
        }
    }

    struct FixtureServer {
        endpoint: String,
        task: JoinHandle<()>,
        request: Option<oneshot::Receiver<String>>,
    }
    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    use tokio::sync::oneshot;

    async fn fixture_server(response: &[u8], stalled: bool) -> FixtureServer {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let endpoint = format!("http://{}/fixture", listener.local_addr().unwrap());
        let response = response.to_vec();
        let (sender, request) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while request.len() < 8192 && !request.ends_with(b"\r\n\r\n") {
                if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            let _ = sender.send(String::from_utf8(request).unwrap());
            socket.write_all(&response).await.unwrap();
            socket.flush().await.unwrap();
            if stalled {
                std::future::pending::<()>().await;
            }
        });
        FixtureServer {
            endpoint,
            task,
            request: Some(request),
        }
    }

    async fn fixture_relay(server: &FixtureServer) -> WebSeedRelay {
        WebSeedRelay::start_with_upstream(
            &["https://fixture.example/payload.bin".into()],
            7,
            &TransferLimits {
                max_bytes: 7,
                timeout: Duration::from_secs(5),
            },
            Arc::new(FixtureUpstream(server.endpoint.clone())),
        )
        .await
        .unwrap()
    }

    fn local_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn relay_streams_full_and_range_bodies_without_forwarding_credentials() {
        for partial in [false, true] {
            let response = if partial {
                b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-3/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nixt".as_slice()
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture"
                    .as_slice()
            };
            let mut server = fixture_server(response, false).await;
            let relay = fixture_relay(&server).await;
            let mut request = local_client()
                .get(&relay.seeds[0])
                .header("Authorization", "local-secret")
                .header("Cookie", "private=value");
            if partial {
                request = request.header("Range", "bytes=1-3");
            }
            let received = request.send().await.unwrap();
            assert_eq!(
                received.status(),
                if partial {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                }
            );
            assert_eq!(
                &received.bytes().await.unwrap()[..],
                if partial {
                    b"ixt".as_slice()
                } else {
                    b"fixture".as_slice()
                }
            );
            let upstream_request = server
                .request
                .take()
                .unwrap()
                .await
                .unwrap()
                .to_ascii_lowercase();
            assert!(!upstream_request.contains("local-secret"));
            assert!(!upstream_request.contains("private=value"));
            assert_eq!(upstream_request.contains("range: bytes=1-3"), partial);
            relay.stop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn relay_rejects_wrong_range_headers_and_truncated_or_expanded_streams() {
        for (response, rejected_headers) in [
            (b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture".as_slice(), true),
            (b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nfix".as_slice(), true),
            (b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-3/7\r\nContent-Length: 4\r\nConnection: close\r\n\r\nixtx".as_slice(), true),
            (b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-3/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nix".as_slice(), false),
            (b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-3/7\r\nConnection: close\r\n\r\nixtx".as_slice(), false),
        ] {
            let server = fixture_server(response, false).await;
            let relay = fixture_relay(&server).await;
            let received = local_client().get(&relay.seeds[0]).header("Range", "bytes=1-3").send().await;
            if rejected_headers {
                assert_eq!(received.unwrap().status(), StatusCode::BAD_GATEWAY);
            } else if let Ok(received) = received {
                assert!(received.bytes().await.is_err());
            }
            relay.stop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn relay_cancellation_joins_inflight_upstream_and_blocks_private_registration() {
        for unsafe_url in [
            "https://127.0.0.1/payload.bin",
            "https://192.168.0.1/payload.bin",
            "http://example.org/payload.bin",
        ] {
            assert!(WebSeedRelay::start(
                &[unsafe_url.into()],
                7,
                &TransferLimits {
                    max_bytes: 7,
                    timeout: Duration::from_secs(5)
                }
            )
            .await
            .is_err());
        }
        let server = fixture_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n",
            true,
        )
        .await;
        let relay = fixture_relay(&server).await;
        let received = local_client().get(&relay.seeds[0]).send().await.unwrap();
        let reading = tokio::spawn(async move { received.bytes().await });
        relay.stop().await.unwrap();
        assert!(reading.await.unwrap().is_err());
    }

    #[test]
    fn ranges_are_single_bounded_piece_offsets() {
        assert_eq!(requested_range("bytes=1-3", 5), Ok((1, 3)));
        assert_eq!(requested_range("bytes=1-", 5), Ok((1, 4)));
        for value in [
            "bytes=-1",
            "bytes=4-5",
            "bytes=2-1",
            "bytes=1-2,3-4",
            "bytes=+1-2",
            "bytes=1 -2",
        ] {
            assert!(requested_range(value, 5).is_err(), "{value}");
        }
    }

    #[test]
    fn upstream_cannot_redirect_or_ignore_or_expand_range() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_RANGE,
            "bytes 1-3/5".parse().unwrap(),
        );
        assert_eq!(
            valid_response(StatusCode::PARTIAL_CONTENT, &headers, Some((1, 3)), 5),
            Ok(3)
        );
        assert!(valid_response(StatusCode::OK, &headers, Some((1, 3)), 5).is_err());
        assert!(valid_response(StatusCode::TEMPORARY_REDIRECT, &headers, None, 5).is_err());
        assert!(valid_response(StatusCode::PARTIAL_CONTENT, &headers, Some((1, 4)), 5).is_err());
        headers.insert(reqwest::header::CONTENT_LENGTH, "4".parse().unwrap());
        assert!(valid_response(StatusCode::PARTIAL_CONTENT, &headers, Some((1, 3)), 5).is_err());
    }

    #[tokio::test]
    async fn tokens_are_private_and_relay_joins_on_shutdown() {
        let limits = TransferLimits {
            max_bytes: 10,
            timeout: Duration::from_secs(5),
        };
        let relay = WebSeedRelay::start(&["https://example.org/payload.bin".into()], 5, &limits)
            .await
            .unwrap();
        let url = url::Url::parse(&relay.seeds[0]).unwrap();
        assert_eq!(url.path().len(), 65);
        let bad = url.join("/unknown").unwrap();
        let response = local_client().get(bad.clone()).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        relay.stop().await.unwrap();
        assert!(local_client().get(bad).send().await.is_err());
    }
}
