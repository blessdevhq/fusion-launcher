//! Host cloud locators produce HTTPS URLs; all IO belongs to the secure transport.

use async_trait::async_trait;

use super::Cancellation;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::resolver::CloudUriResolver;

/// Modular locators are identifiers, never arbitrary transport or filesystem URLs.
/// Providers remain responsible for their ID syntax and can only resolve to HTTPS.
pub fn validate_cloud_uri(value: &str) -> Result<url::Url, FusionError> {
    let invalid = || FusionError::new(ErrorCode::InvalidUri, "Invalid cloud identifier URI.");
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    let uri = url::Url::parse(value).map_err(|_| invalid())?;
    if matches!(
        uri.scheme(),
        "http"
            | "https"
            | "ftp"
            | "ftps"
            | "file"
            | "data"
            | "javascript"
            | "ws"
            | "wss"
            | "magnet"
    ) || !uri
        .scheme()
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || uri.host_str().is_none_or(str::is_empty)
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.port().is_some()
        || !uri.path().is_empty()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(uri)
}

#[derive(Debug, Default)]
pub struct PixeldrainUriResolver;

#[async_trait]
impl CloudUriResolver for PixeldrainUriResolver {
    fn schemes(&self) -> Vec<&str> {
        vec!["pixeldrain"]
    }

    fn supports_scheme(&self, scheme: &str) -> bool {
        scheme == "pixeldrain"
    }

    async fn resolve(&self, value: &str, cancel: &Cancellation) -> Result<String, FusionError> {
        cancel.check()?;
        let uri = validate_cloud_uri(value)?;
        let id = uri.host_str().unwrap_or_default();
        if uri.scheme() != "pixeldrain"
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "Invalid Pixeldrain file ID.",
            ));
        }
        // Non-special URL hosts preserve case, unlike HTTPS domain names.
        Ok(format!("https://pixeldrain.com/api/file/{id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pixeldrain_preserves_id_case_and_rejects_embedded_url_parts() {
        let resolver = PixeldrainUriResolver;
        assert_eq!(
            resolver
                .resolve("pixeldrain://aBcD1234", &Cancellation::default())
                .await
                .unwrap(),
            "https://pixeldrain.com/api/file/aBcD1234"
        );
        for uri in [
            "pixeldrain://",
            "pixeldrain://user@id",
            "pixeldrain://id:443",
            "pixeldrain://id/path",
            "pixeldrain://id/",
            "pixeldrain://id?q=a",
            "pixeldrain://id#x",
            "pixeldrain://%2fetc",
            "pixeldrain://é",
            "gofile://id",
        ] {
            assert_eq!(
                resolver
                    .resolve(uri, &Cancellation::default())
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidUri,
                "{uri}"
            );
        }
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            resolver
                .resolve("pixeldrain://id", &cancel)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Cancelled
        );
    }
}
