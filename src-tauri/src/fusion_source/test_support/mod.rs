//! Shared Creator integration fixtures. Deliberately absent from production.
#![cfg(test)]

pub(crate) mod creator;
pub(crate) use creator::{
    publication_hashes, CreatorHarness, BASE, CONTENT, RUNTIME, SOURCE_ID, USER_DATA,
};
