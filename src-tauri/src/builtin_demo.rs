use sha2::{Digest, Sha256};

use crate::schema::RepositorySchema;
use crate::security::validate_builtin_repository_schema;

pub const BUILTIN_DEMO_REPOSITORY_URL: &str = "fusion-launcher://builtin/demo-repository.json";
pub const LEGACY_BUILTIN_DEMO_REPOSITORY_URL: &str = "retrohydra://builtin/demo-repository.json";

const BUILTIN_DEMO_REPOSITORY_JSON: &str = include_str!("../../public/demo-repository.json");
const BUILTIN_DEMO_ROM_PATH: &str = "demo-content/fusion-launcher-smoke.nes";
const LEGACY_BUILTIN_DEMO_ROM_PATH: &str = "demo-content/retrohydra-smoke.nes";
const BUILTIN_DEMO_ROM_BYTES: &[u8] =
    include_bytes!("../../public/demo-content/fusion-launcher-smoke.nes");
const LEGACY_BUILTIN_DEMO_ROM_BYTES: &[u8] =
    include_bytes!("../../public/demo-content/fusion-launcher-smoke-legacy.nes");

pub fn is_builtin_repository_url(url: &str) -> bool {
    matches!(
        url.trim(),
        BUILTIN_DEMO_REPOSITORY_URL | LEGACY_BUILTIN_DEMO_REPOSITORY_URL
    )
}

pub fn repository_schema() -> Result<RepositorySchema, String> {
    let repo = serde_json::from_str::<RepositorySchema>(BUILTIN_DEMO_REPOSITORY_JSON)
        .map_err(|error| format!("Built-in demo repository JSON is invalid: {error}"))?;
    validate_builtin_repository_schema(&repo)?;
    Ok(repo)
}

pub fn asset_bytes(path: &str) -> Option<&'static [u8]> {
    match path.trim() {
        BUILTIN_DEMO_ROM_PATH => Some(BUILTIN_DEMO_ROM_BYTES),
        LEGACY_BUILTIN_DEMO_ROM_PATH => Some(LEGACY_BUILTIN_DEMO_ROM_BYTES),
        _ => None,
    }
}

pub fn verify_embedded_assets(repo: &RepositorySchema) -> Result<(), String> {
    for game in &repo.catalog {
        for source in &game.downloads {
            if let crate::schema::SourceUri::Bundled { path, sha256, .. } = source {
                let bytes = asset_bytes(path)
                    .ok_or_else(|| format!("Unknown bundled demo asset: {path}"))?;
                let actual = hex::encode(Sha256::digest(bytes));
                if !actual.eq_ignore_ascii_case(sha256) {
                    return Err(format!(
                        "Bundled demo asset hash mismatch for {path}: expected {sha256}, got {actual}"
                    ));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_verifies_built_in_demo_repository() {
        let repo = repository_schema().unwrap();

        assert_eq!(repo.metadata.id, "fusion-launcher-demo");
        assert_eq!(repo.catalog.len(), 1);
        assert_eq!(repo.catalog[0].id, "fusion_launcher_nes_smoke");
        verify_embedded_assets(&repo).unwrap();
    }

    #[test]
    fn identifies_built_in_repository_url() {
        assert!(is_builtin_repository_url(BUILTIN_DEMO_REPOSITORY_URL));
        assert!(is_builtin_repository_url(
            LEGACY_BUILTIN_DEMO_REPOSITORY_URL
        ));
        assert!(!is_builtin_repository_url("https://example.com/repo.json"));
    }

    #[test]
    fn resolves_legacy_bundled_asset_path() {
        let bytes = asset_bytes(LEGACY_BUILTIN_DEMO_ROM_PATH).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(bytes)),
            "904918a63180b96e6ffd7f98ef775e2b59fa92faf6802b7df450623ba07891df"
        );
    }
}
