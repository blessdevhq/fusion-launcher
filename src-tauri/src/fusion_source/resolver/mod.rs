//! Layer delivery and source discovery independent of Tauri, SQLite and UI.

mod dynamic;
mod local;
mod manifests;
mod mirrors;

pub use dynamic::{
    DynamicAssetAttestation, DynamicProviderResolver, DynamicRuntimeLock, GithubReleaseClient,
    ReleaseAsset, ReleaseMetadata, SecureGithubReleaseClient,
};
pub use local::{LocalLocatorResolver, LocatedBase, StorefrontInventory};
pub use manifests::{
    BoundManifestSyncVerifier, ManifestFetcher, StrictSyncProof, VerifiedManifest,
};
pub use mirrors::{
    CloudUriResolver, DeliveryReceipt, MirrorCascadeResolver, TorrentAdapter, TorrentFile,
    TorrentLayout, TorrentRequest,
};

#[cfg(test)]
mod tests;
