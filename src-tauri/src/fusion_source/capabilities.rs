//! Pure, game-scoped capability negotiation. A registry describes implemented
//! adapters, not interfaces that may be implemented by a future stage.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::canonical::{CanonicalGameManifest, QualifiedGameId};
use super::errors::{ErrorCode, FusionError};
use super::schema::FallbackStrategy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRegistry {
    supported: BTreeSet<String>,
}

impl CapabilityRegistry {
    pub fn new<I, S>(capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            supported: capabilities.into_iter().map(Into::into).collect(),
        }
    }

    /// Only adapters implemented by this client are advertised. Torrent/cloud
    /// plugins and patch-tool availability require explicit host configuration;
    /// the Wasm interface never implies execution support.
    pub fn current_client() -> Self {
        let registry = Self::new([
            "core.schema.v1",
            "core.plan.v1",
            "core.layers.v1",
            "vfs.hardlink.v1",
            "core.config_templates.v1",
            "persistence.saves.v1",
            "resolver.https.v1",
            "local_locator.v1",
            "provider.github_releases.v1",
        ]);
        #[cfg(windows)]
        let registry = registry
            .with("proc.supervisor.v1")
            .with("orchestrator.multi_process.v1")
            .with("orchestrator.strict_sync.v1");
        registry
    }

    pub fn with(mut self, capability: impl Into<String>) -> Self {
        self.supported.insert(capability.into());
        self
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.supported.contains(capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.supported.iter().map(String::as_str)
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::current_client()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CapabilitySelection {
    Preferred,
    Fallback { strategy: FallbackStrategy },
    Omitted,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityWarning {
    pub capability: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationReport {
    pub game: QualifiedGameId,
    pub decisions: BTreeMap<String, CapabilitySelection>,
    pub warnings: Vec<CapabilityWarning>,
    pub blockers: Vec<FusionError>,
}

impl NegotiationReport {
    pub fn is_blocked(&self) -> bool {
        !self.blockers.is_empty()
    }

    pub fn selection(&self, capability: &str) -> Option<&CapabilitySelection> {
        self.decisions.get(capability)
    }

    pub fn uses_fallback(&self, strategy: &FallbackStrategy) -> bool {
        self.decisions.values().any(|selection| {
            matches!(selection, CapabilitySelection::Fallback { strategy: selected } if selected == strategy)
        })
    }

    pub fn omits(&self, capability: &str) -> bool {
        matches!(
            self.selection(capability),
            Some(CapabilitySelection::Omitted)
                | Some(CapabilitySelection::Fallback {
                    strategy: FallbackStrategy::SkipOptional
                })
        )
    }
}

pub struct CapabilityNegotiator;

impl CapabilityNegotiator {
    /// Negotiates one game without rejecting its containing source index.
    /// Fallback dependencies must be implemented directly; recursive fallback
    /// chains cannot accidentally claim an unavailable execution adapter.
    pub fn negotiate(
        manifest: &CanonicalGameManifest,
        registry: &CapabilityRegistry,
    ) -> NegotiationReport {
        let mut report = NegotiationReport {
            game: QualifiedGameId {
                source_id: manifest.source_id.clone(),
                game_id: manifest.id.clone(),
            },
            decisions: BTreeMap::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
        };

        for (capability, required) in manifest
            .capabilities
            .required
            .iter()
            .map(|capability| (capability, true))
            .chain(
                manifest
                    .capabilities
                    .optional
                    .iter()
                    .map(|capability| (capability, false)),
            )
        {
            if registry.supports(capability) {
                report
                    .decisions
                    .insert(capability.clone(), CapabilitySelection::Preferred);
                continue;
            }

            let fallback = manifest
                .fallback
                .iter()
                .find(|fallback| fallback.capability == *capability);
            let selected = fallback.filter(|fallback| {
                !(required && fallback.strategy == FallbackStrategy::SkipOptional)
                    && fallback
                        .requires
                        .iter()
                        .all(|dependency| registry.supports(dependency))
                    && match fallback.strategy {
                        FallbackStrategy::HardlinkProjection => {
                            registry.supports("vfs.hardlink.v1")
                        }
                        FallbackStrategy::BuiltinConfigTemplates => {
                            registry.supports("core.config_templates.v1")
                        }
                        FallbackStrategy::SkipOptional => true,
                    }
            });

            if let Some(fallback) = selected {
                report.decisions.insert(
                    capability.clone(),
                    CapabilitySelection::Fallback {
                        strategy: fallback.strategy,
                    },
                );
                report.warnings.push(CapabilityWarning {
                    capability: capability.clone(),
                    message: format!("Capability {capability} uses its declared fallback"),
                });
            } else if required {
                report
                    .decisions
                    .insert(capability.clone(), CapabilitySelection::Blocked);
                report.blockers.push(unsupported_capability(
                    manifest,
                    capability,
                    "Required capability is unavailable and has no supported fallback",
                ));
            } else {
                report
                    .decisions
                    .insert(capability.clone(), CapabilitySelection::Omitted);
                report.warnings.push(CapabilityWarning {
                    capability: capability.clone(),
                    message: format!(
                        "Optional capability {capability} is unavailable and will be omitted"
                    ),
                });
            }
        }

        report
    }
}

pub(crate) fn unsupported_capability(
    manifest: &CanonicalGameManifest,
    capability: &str,
    message: &str,
) -> FusionError {
    // Structured construction is kept in one helper so the report and planner
    // always retain the game and capability that caused a block.
    FusionError::new(ErrorCode::MissingCapability, message)
        .with_detail("sourceId", &manifest.source_id)
        .with_detail("gameId", &manifest.id)
        .with_detail("capability", capability)
}
