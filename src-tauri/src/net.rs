use std::error::Error;

pub(crate) const HTTP_USER_AGENT: &str = concat!("FusionLauncher/", env!("CARGO_PKG_VERSION"));

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
    let lower = host_str.trim().to_ascii_lowercase();
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

/// Generic policy for downloads: strictly HTTPS, maximum 5 hops, forbids private/loopback/SSRF targets.
pub(crate) fn https_only_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.url().scheme() != "https" {
            attempt.error("Insecure HTTP redirect rejected")
        } else if attempt
            .url()
            .host_str()
            .is_some_and(is_forbidden_target_host)
        {
            attempt.error("Redirect to private or loopback target rejected")
        } else if attempt.previous().len() >= 5 {
            attempt.error("Too many redirects")
        } else {
            attempt.follow()
        }
    })
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
                "Host '{host}' resolved to forbidden private or loopback IP address {ip} (SSRF blocked)"
            ));
        }
    }

    builder = builder.resolve_to_addrs(host, &resolved_list);
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;

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
