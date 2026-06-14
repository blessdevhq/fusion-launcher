use glob::Pattern;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResolvedAsset {
    pub url: String,
    pub filename: String,
    pub size: u64,
    pub version: String,
}

pub async fn resolve_github_latest(
    repo: &str,
    asset_pattern: &str,
) -> Result<ResolvedAsset, String> {
    let api_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::new();
    let release = client
        .get(api_url)
        .header("User-Agent", "Fusion Launcher/0.1")
        .send()
        .await
        .map_err(|error| format!("GitHub API error: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub API returned an error: {error}"))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("Failed to parse GitHub release: {error}"))?;

    select_asset(&release, repo, asset_pattern)
}

fn select_asset(
    release: &GithubRelease,
    repo: &str,
    asset_pattern: &str,
) -> Result<ResolvedAsset, String> {
    let pattern =
        Pattern::new(asset_pattern).map_err(|error| format!("Bad asset pattern: {error}"))?;
    let asset = release
        .assets
        .iter()
        .find(|asset| pattern.matches(&asset.name))
        .ok_or_else(|| format!("No asset matching '{asset_pattern}' in {repo}/releases/latest"))?;

    Ok(ResolvedAsset {
        url: asset.browser_download_url.clone(),
        filename: asset.name.clone(),
        size: asset.size,
        version: release.tag_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_selects_matching_release_asset() {
        let release = GithubRelease {
            tag_name: "v2.1.1".to_string(),
            assets: vec![
                GithubAsset {
                    name: "Mesen-Linux-x64.zip".to_string(),
                    browser_download_url: "https://example.invalid/linux".to_string(),
                    size: 10,
                },
                GithubAsset {
                    name: "Mesen-Windows-x64.zip".to_string(),
                    browser_download_url: "https://example.invalid/windows".to_string(),
                    size: 20,
                },
            ],
        };

        let resolved = select_asset(&release, "SourMesen/Mesen2", "Mesen-Windows*.zip").unwrap();

        assert_eq!(resolved.filename, "Mesen-Windows-x64.zip");
        assert_eq!(resolved.version, "v2.1.1");
    }

    #[test]
    fn missing_release_asset_returns_clear_error() {
        let release = GithubRelease {
            tag_name: "v1".to_string(),
            assets: vec![],
        };

        let error = select_asset(&release, "owner/repo", "*.zip").unwrap_err();

        assert!(error.contains("No asset matching"));
    }
}
