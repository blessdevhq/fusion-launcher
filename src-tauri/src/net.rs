use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const HTTP_USER_AGENT: &str = concat!("FusionLauncher/", env!("CARGO_PKG_VERSION"));
pub(crate) const JSON_BODY_LIMIT: usize = 16 * 1024 * 1024;
pub(crate) const ARTWORK_BODY_LIMIT: usize = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// The resolver is installed in the actual connector rather than doing a DNS
/// preflight followed by an independent lookup. Every address returned for a
/// direct target is checked, and resolution failure is fail-closed.
///
/// Reqwest retains its system-proxy selection (including NO_PROXY). If it
/// resolves a different host, that host is the selected proxy, a trusted network
/// intermediary which may legitimately be local. A proxy can resolve/route the
/// destination remotely; this policy makes no claim about its final IP. Fake-IP
/// routing is likewise trusted, preserving Clash/mihomo compatibility.
struct TargetResolver {
    target: String,
    allow_dev_loopback: bool,
}

impl reqwest::dns::Resolve for TargetResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let validate = host.eq_ignore_ascii_case(&self.target) && !self.allow_dev_loopback;
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| std::io::Error::other(format!(
                    "DNS lookup for '{host}' failed. Check your DNS resolver and system proxy settings: {error}"
                )))?
                .collect();
            if validate {
                validate_resolved_addresses(&host, &addresses)?;
            } else if addresses.is_empty() {
                return Err(std::io::Error::other("DNS returned no addresses").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn validate_resolved_addresses(
    host: &str,
    addresses: &[std::net::SocketAddr],
) -> Result<(), std::io::Error> {
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| is_forbidden_ip(address.ip()))
    {
        return Err(std::io::Error::other(format!(
            "DNS for '{host}' returned no usable public addresses, or included a private/loopback address. Check your DNS resolver and system proxy settings; Fusion cannot safely connect to this destination."
        )));
    }
    Ok(())
}

fn validate_external_url(url: &url::Url, allow_dev_http: bool) -> Result<(), String> {
    crate::security::validate_http_url(url, allow_dev_http)
}

fn redirect_target(
    current: &url::Url,
    location: &str,
    allow_dev_http: bool,
) -> Result<url::Url, String> {
    let next = current
        .join(location)
        .map_err(|error| format!("Invalid redirect Location: {error}"))?;
    validate_external_url(&next, allow_dev_http)?;
    if current.scheme() == "https" && next.scheme() != "https" {
        return Err("HTTPS downgrade redirect rejected".to_string());
    }
    Ok(next)
}

/// GET an external URL with a single deadline and manual, checked redirects.
/// Each hop uses a fresh client/request, so authorization, cookies and other
/// origin credentials cannot be forwarded to a new origin.
pub(crate) async fn get_external(
    url: &url::Url,
    allow_dev_http: bool,
    timeout: Duration,
) -> Result<reqwest::Response, String> {
    get_external_with_headers(
        url,
        allow_dev_http,
        timeout,
        reqwest::header::HeaderMap::new(),
    )
    .await
}

/// Only resume-related headers are accepted. In particular no caller can
/// accidentally forward Authorization or Cookie across redirect origins.
pub(crate) async fn get_external_with_headers(
    url: &url::Url,
    allow_dev_http: bool,
    timeout: Duration,
    headers: reqwest::header::HeaderMap,
) -> Result<reqwest::Response, String> {
    if headers
        .keys()
        .any(|name| name != reqwest::header::RANGE && name != reqwest::header::IF_RANGE)
    {
        return Err("Only Range and If-Range external request headers are supported".to_string());
    }
    tokio::time::timeout(
        timeout,
        get_external_inner(url, allow_dev_http, timeout, headers),
    )
    .await
    .map_err(|_| "External request timed out".to_string())?
}

async fn get_external_inner(
    url: &url::Url,
    allow_dev_http: bool,
    timeout: Duration,
    headers: reqwest::header::HeaderMap,
) -> Result<reqwest::Response, String> {
    let mut current = url.clone();
    let mut visited = std::collections::HashSet::new();
    for hop in 0..=MAX_REDIRECTS {
        validate_external_url(&current, allow_dev_http)?;
        // Fragments do not participate in an HTTP request or redirect identity.
        current.set_fragment(None);
        if !visited.insert(current.to_string()) {
            return Err("Redirect loop rejected".to_string());
        }
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout)
            .user_agent(HTTP_USER_AGENT);
        let builder = target_resolver_builder(builder, &current, allow_dev_http)?;
        let client = builder.build().map_err(|error| error.to_string())?;
        let response = client
            .get(current.clone())
            .headers(headers.clone())
            .send()
            .await
            .map_err(|error| format_reqwest_error("external resource", &error))?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == MAX_REDIRECTS {
                return Err("Too many redirects".to_string());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| "Redirect is missing Location".to_string())?
                .to_str()
                .map_err(|_| "Invalid redirect Location header".to_string())?;
            current = redirect_target(&current, location, allow_dev_http)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "External request returned HTTP {}",
                response.status()
            ));
        }
        return Ok(response);
    }
    Err("Too many redirects".to_string())
}

pub(crate) async fn fetch_external_bytes(
    url: &url::Url,
    allow_dev_http: bool,
    timeout: Duration,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    tokio::time::timeout(timeout, async {
        let response = get_external(url, allow_dev_http, timeout).await?;
        read_bounded_body(response, max_bytes).await
    })
    .await
    .map_err(|_| "External resource timed out while reading body".to_string())?
}

/// A report upload is an explicit user action. Reject redirects entirely so a
/// source cannot relay even sanitized report data to an unreviewed destination.
pub(crate) async fn post_external_json(
    url: &url::Url,
    body: &[u8],
    timeout: Duration,
) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err("Report delivery requires HTTPS".into());
    }
    if body.len() > 1024 * 1024 {
        return Err("Report exceeds the upload limit".into());
    }
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout)
        .user_agent(HTTP_USER_AGENT);
    let client = target_resolver_builder(builder, url, false)?
        .build()
        .map_err(|error| error.to_string())?;
    let response = tokio::time::timeout(
        timeout,
        client
            .post(url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_owned())
            .send(),
    )
    .await
    .map_err(|_| "Report delivery timed out".to_owned())?
    .map_err(|error| format_reqwest_error("report delivery", &error))?;
    if !response.status().is_success() {
        return Err(format!(
            "Report delivery returned HTTP {}",
            response.status()
        ));
    }
    Ok(())
}

async fn read_bounded_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        return Err(format!("External resource exceeds {max_bytes} byte limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format_reqwest_error("external resource body", &error))?
    {
        if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
            return Err(format!("External resource exceeds {max_bytes} byte limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn target_resolver_builder(
    builder: reqwest::ClientBuilder,
    url: &url::Url,
    allow_dev_http: bool,
) -> Result<reqwest::ClientBuilder, String> {
    validate_external_url(url, allow_dev_http)?;
    let host = url.host_str().ok_or("URL has no host")?;
    Ok(builder.dns_resolver(Arc::new(TargetResolver {
        target: host.to_string(),
        allow_dev_loopback: allow_dev_http && url.scheme() == "http",
    })))
}

pub(crate) fn format_reqwest_error(context: &str, error: &reqwest::Error) -> String {
    let target = error
        .url()
        .map(|url| url.as_str())
        .unwrap_or("the requested URL");
    let action = if error.is_timeout() {
        "Timed out"
    } else if error.is_connect() {
        "Could not connect"
    } else if error.is_status() {
        "HTTP request failed"
    } else if error.is_decode() {
        "Invalid response"
    } else {
        "Network request failed"
    };
    let mut message = format!("{action} while fetching {context} from {target}");

    let mut source = error.source();
    while let Some(cause) = source {
        let cause_message = cause.to_string();
        if !cause_message.is_empty() && !message.contains(&cause_message) {
            message.push_str(": ");
            message.push_str(&cause_message);
        }
        source = cause.source();
    }

    if !message.contains(&error.to_string()) {
        message.push_str(": ");
        message.push_str(&error.to_string());
    }

    message
}

pub(crate) fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 0.0.0.0/8 (Current network)
            octets[0] == 0
                // 127.0.0.0/8 (Loopback)
                || ipv4.is_loopback()
                // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 (Private)
                || ipv4.is_private()
                // 169.254.0.0/16 (Link-local)
                || ipv4.is_link_local()
                // 255.255.255.255 (Broadcast)
                || ipv4.is_broadcast()
                // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 (Documentation)
                || ipv4.is_documentation()
                // 100.64.0.0/10 (Shared Address Space / CGNAT RFC 6598: 100.64.0.0 - 100.127.255.255)
                || (octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000)
                // Note: 198.18.0.0/15 is intentionally NOT blocked here because Clash/mihomo
                // transparent proxy Fake-IP mode maps external target hosts into this range.
                // 224.0.0.0/4 (Multicast)
                || ipv4.is_multicast()
                // 240.0.0.0/4 (Reserved RFC 1112)
                || octets[0] >= 240
        }
        std::net::IpAddr::V6(ipv6) => {
            let octets = ipv6.octets();
            // Loopback (::1) or Unspecified (::)
            if ipv6.is_loopback() || ipv6.is_unspecified() {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:0:0/96)
            if octets[0..10] == [0; 10] && octets[10] == 0xff && octets[11] == 0xff {
                let mapped =
                    std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return is_forbidden_ip(std::net::IpAddr::V4(mapped));
            }
            // IPv4-compatible IPv6 (deprecated ::a.b.c.d)
            if octets[0..12] == [0; 12]
                && (octets[12] != 0 || octets[13] != 0 || octets[14] != 0 || octets[15] != 1)
            {
                let mapped =
                    std::net::Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return is_forbidden_ip(std::net::IpAddr::V4(mapped));
            }
            // IPv6 Unique Local Address (ULA: fc00::/7 -> fc00.. - fdff..)
            if (octets[0] & 0xfe) == 0xfc {
                return true;
            }
            // IPv6 Link-Local Unicast (fe80::/10 -> fe80.. - febf..)
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 {
                return true;
            }
            // IPv6 Site-Local (deprecated: fec0::/10)
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0xc0 {
                return true;
            }
            // IPv6 Multicast (ff00::/8)
            if octets[0] == 0xff {
                return true;
            }
            // IPv6 Documentation (2001:db8::/32)
            if octets[0] == 0x20 && octets[1] == 0x01 && octets[2] == 0x0d && octets[3] == 0xb8 {
                return true;
            }
            // IPv6 Discard Prefix (100::/64)
            if octets[0] == 0x01 && octets[1] == 0x00 && octets[2..8] == [0; 6] {
                return true;
            }
            false
        }
    }
}

pub(crate) fn is_forbidden_target_host(host_str: &str) -> bool {
    let lower = host_str.trim().trim_end_matches('.').to_ascii_lowercase();
    let unbracketed = lower
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&lower);

    if unbracketed == "localhost"
        || unbracketed.ends_with(".localhost")
        || unbracketed.ends_with(".local")
        || unbracketed.ends_with(".internal")
    {
        return true;
    }

    if let Ok(ip) = unbracketed.parse::<std::net::IpAddr>() {
        return is_forbidden_ip(ip);
    }

    false
}

/// Strict policy for GitHub resolvers: strictly HTTPS, max 5 hops,
/// allowlisted strictly to github.com and githubusercontent.com.
pub(crate) fn github_redirect_policy() -> reqwest::redirect::Policy {
    const ALLOWED: &[&str] = &["github.com", "githubusercontent.com"];
    reqwest::redirect::Policy::custom(|attempt| {
        let is_https = attempt.url().scheme() == "https";
        let is_allowed = attempt.url().host_str().is_some_and(|h| {
            ALLOWED
                .iter()
                .any(|a| h == *a || h.ends_with(&format!(".{a}")))
        });
        if !is_https {
            attempt.error("Insecure HTTP redirect rejected")
        } else if !is_allowed {
            attempt.error("Redirect to non-GitHub host rejected")
        } else if attempt.previous().len() >= 5 {
            attempt.error("Too many redirects")
        } else {
            attempt.follow()
        }
    })
}

/// Validate host DNS resolution asynchronously against SSRF, allowing Fake-IP (198.18.0.0/15).
/// If resolution succeeds and all resolved addresses are safe, pins the resolved addresses to the
/// reqwest::ClientBuilder to prevent DNS rebinding attacks.
pub(crate) async fn resolve_and_pin_host(
    mut builder: reqwest::ClientBuilder,
    url: &url::Url,
) -> Result<reqwest::ClientBuilder, String> {
    let Some(host) = url.host_str() else {
        return Err("URL has no host".to_string());
    };

    if is_forbidden_target_host(host) {
        return Err(format!("Forbidden target host: {host}"));
    }

    // If host is an IP literal, it was already checked by is_forbidden_target_host.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(builder);
    }

    let port = url.port_or_known_default().unwrap_or(443);
    let socket_target = format!("{host}:{port}");

    let Ok(addrs) = tokio::net::lookup_host(&socket_target).await else {
        // If DNS lookup fails here (e.g. offline or custom socks proxy), let reqwest handle connection
        return Ok(builder);
    };

    let resolved_list: Vec<std::net::SocketAddr> = addrs.collect();
    if resolved_list.is_empty() {
        return Ok(builder);
    }

    for socket_addr in &resolved_list {
        let ip = socket_addr.ip();
        if is_forbidden_ip(ip) {
            return Err(format!(
                "DNS for '{host}' returned a private or loopback address ({ip}). Check your DNS resolver and system proxy settings; Fusion cannot safely connect to this destination."
            ));
        }
    }

    builder = builder.resolve_to_addrs(host, &resolved_list);
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_fixture(responses: Vec<String>) -> url::Url {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            for response in responses {
                let until = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(_) if std::time::Instant::now() < until => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return,
                    }
                };
                // Accepted Windows sockets can inherit the listener's
                // nonblocking mode; wait for the complete request before reply.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(count) => request.extend_from_slice(&chunk[..count]),
                    }
                    if request.len() > 16384 {
                        return;
                    }
                }
                let _ = stream.write_all(response.as_bytes());
            }
        });
        url::Url::parse(&format!("http://{address}/start")).unwrap()
    }

    #[test]
    fn redirect_targets_reject_downgrade_credentials_and_internal_hosts() {
        let current = url::Url::parse("https://example.com/dir/start").unwrap();
        assert_eq!(
            redirect_target(&current, "../catalog.json", false)
                .unwrap()
                .as_str(),
            "https://example.com/catalog.json"
        );
        for target in [
            "http://example.com/catalog.json",
            "http://localhost/catalog.json",
            "https://user:secret@example.com/catalog.json",
            "https://127.0.0.1/catalog.json",
            "https://[::ffff:127.0.0.1]/catalog.json",
            "https://169.254.169.254/catalog.json",
            "https://localhost./catalog.json",
            "https://server.internal./catalog.json",
            "https://[fe80::1]/catalog.json",
            "https://[fc00::1]/catalog.json",
            "http://[invalid",
        ] {
            assert!(
                redirect_target(&current, target, false).is_err(),
                "{target}"
            );
        }
        // Even the explicit local development exception cannot downgrade HTTPS.
        assert!(redirect_target(&current, "http://localhost/catalog.json", true).is_err());
    }

    #[test]
    fn resolved_addresses_fail_closed_and_reject_mixed_answers() {
        assert!(validate_resolved_addresses("example.com", &[]).is_err());
        let public = "8.8.8.8:443".parse().unwrap();
        let private = "10.0.0.1:443".parse().unwrap();
        assert!(validate_resolved_addresses("example.com", &[public]).is_ok());
        assert!(validate_resolved_addresses("example.com", &[public, private]).is_err());
        assert!(
            validate_resolved_addresses("fakeip", &["198.18.0.1:443".parse().unwrap()]).is_ok()
        );
    }

    #[tokio::test]
    async fn actual_connector_resolver_blocks_target_dns_but_allows_trusted_proxy_dns() {
        use reqwest::dns::Resolve;
        let resolver = TargetResolver {
            target: "localhost".to_string(),
            allow_dev_loopback: false,
        };
        assert!(resolver
            .resolve("localhost".parse().unwrap())
            .await
            .is_err());
        // Only the selected proxy endpoint resolves under this exception; the
        // production client itself chooses whether to use a proxy or NO_PROXY.
        let resolver = TargetResolver {
            target: "example.com".to_string(),
            allow_dev_loopback: false,
        };
        let addresses: Vec<_> = resolver
            .resolve("localhost".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
    }

    #[tokio::test]
    async fn configured_local_proxy_is_used_without_claiming_its_remote_dns_is_validated() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap();
            sender
                .send(String::from_utf8_lossy(&request[..count]).to_string())
                .unwrap();
            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        });
        let target = url::Url::parse("https://example.com/source.json").unwrap();
        // An explicit fixture proxy avoids mutating process-wide environment
        // variables while other tests run. Production retains system selection.
        let builder = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::https(format!("http://localhost:{port}")).unwrap())
            .timeout(Duration::from_secs(2));
        let client = target_resolver_builder(builder, &target, false)
            .unwrap()
            .build()
            .unwrap();
        assert!(client.get(target).send().await.is_err());
        let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("CONNECT example.com:443 "), "{request}");
    }

    #[tokio::test]
    async fn external_get_follows_relative_redirect_and_rejects_loops_and_limits() {
        let endpoint = http_fixture(vec![
            "HTTP/1.1 302 Found\r\nLocation: ../final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string(),
        ]);
        assert_eq!(
            fetch_external_bytes(&endpoint, true, Duration::from_secs(2), 16)
                .await
                .unwrap(),
            b"{}"
        );
        let endpoint = http_fixture(vec![
            "HTTP/1.1 302 Found\r\nLocation: /start\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
        ]);
        assert!(get_external(&endpoint, true, Duration::from_secs(2))
            .await
            .unwrap_err()
            .contains("loop"));
        let endpoint = http_fixture((0..=MAX_REDIRECTS).map(|n| format!(
            "HTTP/1.1 302 Found\r\nLocation: /hop{n}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )).collect());
        // Several fresh TLS clients run alongside the rest of the suite. The
        // fixture deadline must cover all redirect hops, not just one request.
        let error = get_external(&endpoint, true, Duration::from_secs(10))
            .await
            .unwrap_err();
        assert!(error.contains("Too many"), "{error}");
    }

    #[tokio::test]
    async fn external_get_rejects_missing_location_credentials_and_production_localhost() {
        let endpoint = http_fixture(vec![
            "HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
        ]);
        assert!(get_external(&endpoint, true, Duration::from_secs(2))
            .await
            .unwrap_err()
            .contains("Location"));
        assert!(get_external(&endpoint, false, Duration::from_secs(2))
            .await
            .is_err());
        let endpoint = url::Url::parse("https://user:password@example.com/").unwrap();
        assert!(get_external(&endpoint, false, Duration::from_secs(1))
            .await
            .is_err());
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, "secret".parse().unwrap());
        assert!(
            get_external_with_headers(&endpoint, false, Duration::from_secs(1), headers)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn actual_chunked_body_is_capped_for_json_and_artwork_without_content_length() {
        for limit in [JSON_BODY_LIMIT, ARTWORK_BODY_LIMIT] {
            let payload = "x".repeat(limit + 1);
            let endpoint = http_fixture(vec![format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{payload}\r\n0\r\n\r\n", payload.len()
            )]);
            // Test the real bounded reader over a socket; only this fixture
            // client uses no_proxy/local HTTP, production URLs stay restricted.
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(endpoint)
                .send()
                .await
                .unwrap();
            let error = read_bounded_body(response, limit).await.unwrap_err();
            assert!(error.contains("byte limit"), "{error}");
        }
    }

    #[tokio::test]
    async fn full_fetch_budget_includes_stalled_body() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx");
            std::thread::sleep(Duration::from_secs(1));
        });
        let endpoint = url::Url::parse(&format!("http://{address}/stall")).unwrap();
        assert!(
            fetch_external_bytes(&endpoint, true, Duration::from_millis(100), 1024)
                .await
                .unwrap_err()
                .contains("timed out")
        );
    }

    #[tokio::test]
    async fn get_budget_includes_stalled_headers() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_secs(1));
        });
        let endpoint = url::Url::parse(&format!("http://{address}/stall")).unwrap();
        assert!(get_external(&endpoint, true, Duration::from_millis(100))
            .await
            .unwrap_err()
            .contains("timed out"));
    }

    #[test]
    fn forbidden_hosts_detects_ssrf_and_private_ranges() {
        assert!(is_forbidden_target_host("localhost"));
        assert!(is_forbidden_target_host("sub.localhost"));
        assert!(is_forbidden_target_host("server.local"));
        assert!(is_forbidden_target_host("router.internal"));
        assert!(is_forbidden_target_host("127.0.0.1"));
        assert!(is_forbidden_target_host("10.0.0.1"));
        assert!(is_forbidden_target_host("192.168.1.1"));
        assert!(is_forbidden_target_host("172.16.0.1"));
        assert!(is_forbidden_target_host("169.254.169.254"));
        assert!(is_forbidden_target_host("0.0.0.0"));
        assert!(is_forbidden_target_host("0.1.2.3"));
        assert!(is_forbidden_target_host("100.64.0.1")); // CGNAT RFC 6598
        assert!(is_forbidden_target_host("100.127.255.254")); // CGNAT RFC 6598
        assert!(is_forbidden_target_host("::1"));
        assert!(is_forbidden_target_host("[::1]"));
        assert!(is_forbidden_target_host("fc00::1")); // IPv6 ULA
        assert!(is_forbidden_target_host("fd12:3456:789a::1")); // IPv6 ULA
        assert!(is_forbidden_target_host("fe80::1")); // IPv6 Link-Local
        assert!(is_forbidden_target_host("[fe80::1]"));
        assert!(is_forbidden_target_host("::ffff:127.0.0.1")); // IPv4-mapped loopback
        assert!(is_forbidden_target_host("::ffff:192.168.1.1")); // IPv4-mapped private
        assert!(is_forbidden_target_host("[::ffff:10.0.0.1]")); // IPv4-mapped private bracketed

        // Fake-IP proxy addresses (198.18.0.0/15) MUST NOT be blocked, preserving Clash/mihomo compatibility
        assert!(!is_forbidden_target_host("198.18.0.1"));
        assert!(!is_forbidden_target_host("198.19.255.254"));

        assert!(!is_forbidden_target_host("github.com"));
        assert!(!is_forbidden_target_host("objects.githubusercontent.com"));
        assert!(!is_forbidden_target_host("example.com"));
    }

    #[test]
    fn includes_context_for_request_build_errors() {
        let error = reqwest::Client::new()
            .get("https://example.com/repository.json")
            .header("bad header", "\n")
            .build()
            .unwrap_err();

        let message = format_reqwest_error("source library", &error);

        assert!(message.contains("source library"));
        assert!(message.contains("Network request failed"));
    }

    #[tokio::test]
    async fn report_upload_rejects_unsafe_destinations_and_oversize_before_connecting() {
        for target in [
            "http://example.com/report",
            "https://127.0.0.1/report",
            "https://192.168.1.1/report",
            "https://user:password@example.com/report",
        ] {
            assert!(post_external_json(
                &url::Url::parse(target).unwrap(),
                b"{}",
                Duration::from_secs(1)
            )
            .await
            .is_err());
        }
        let endpoint = url::Url::parse("https://example.com/report").unwrap();
        assert!(
            post_external_json(&endpoint, &vec![0; 1024 * 1024 + 1], Duration::from_secs(1))
                .await
                .unwrap_err()
                .contains("upload limit")
        );
    }

    #[tokio::test]
    async fn resolve_and_pin_host_rejects_forbidden_targets() {
        let loopback_url = url::Url::parse("https://127.0.0.1/catalog.json").unwrap();
        assert!(
            resolve_and_pin_host(reqwest::Client::builder(), &loopback_url)
                .await
                .is_err()
        );

        let localhost_url = url::Url::parse("https://localhost/catalog.json").unwrap();
        assert!(
            resolve_and_pin_host(reqwest::Client::builder(), &localhost_url)
                .await
                .is_err()
        );

        // Fake-IP address is allowed through
        let fake_ip_url = url::Url::parse("https://198.18.0.10/catalog.json").unwrap();
        assert!(
            resolve_and_pin_host(reqwest::Client::builder(), &fake_ip_url)
                .await
                .is_ok()
        );
    }
}
