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

#[cfg(test)]
mod tests {
    use super::*;

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
}
