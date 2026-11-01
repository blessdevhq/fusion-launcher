//! Shared implementation of the frozen fusion.declarative 1.0 host contract.
pub mod candidate;
pub mod codecs;
pub mod compiler;
pub mod io;
pub mod runtime;
pub mod state;

#[cfg(test)]
mod integration_tests;
