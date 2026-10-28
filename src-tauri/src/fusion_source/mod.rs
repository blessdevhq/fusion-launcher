//! Fusion Source Standard v1.0: contracts, assembly, delivery and supervision.
//!
//! Parsing and planning remain pure. Stage 2 filesystem APIs are synchronous
//! and run on blocking threads in the integration facade. Delivery uses injected
//! transports, hash-bound trust and a Windows process ownership worker. The core
//! facade depends on storage/progress traits; SQLite and Tauri are host adapters.
//!
//! CAS files are read-only. A writable workspace file must be materialized as a
//! separate physical copy; a hardlink never provides copy-on-write isolation.

pub mod adapters;
pub mod canonical;
pub mod capabilities;
pub mod cas;
pub mod config_templates;
pub mod dto;
pub mod errors;
pub mod evolution;
pub mod fs_safety;
pub mod host_tools;
pub mod integrity;
pub mod layers;
pub mod offline;
pub mod packer;
pub mod persistence;
pub mod plan;
pub mod resolver;
pub mod schema;
pub mod service;
pub mod signatures;

pub mod supervisor;
pub mod telemetry;
pub mod transforms;
pub mod wasm;
pub mod workspace;

pub use canonical::{CanonicalGameManifest, CanonicalSourceIndex, QualifiedGameId};
pub use capabilities::{CapabilityNegotiator, CapabilityRegistry, NegotiationReport};
pub use errors::{ErrorCode, FusionError};
pub use evolution::SchemaUpcaster;
pub use plan::{ExecutionPlan, PlanBuilder, PlanOptions};
pub use schema::{GameManifest, SourceIndex};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod service_tests;

#[cfg(test)]
mod workspace_tests;

#[cfg(test)]
mod packer_tests;
