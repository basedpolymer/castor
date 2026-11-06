//! Files & archive tools. Owner: agent E — see docs/specs/E-create-files.md.

use std::sync::Arc;

use castor_core::Tool;

mod archive;
mod checksum;
pub mod common;
mod dates;
mod duplicates;
mod extract;
pub mod identify;
mod magika_table;
mod metadata;
mod rename;
mod sort;

pub use identify::{Identity, identify};

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(rename::Rename::new()),
        Arc::new(archive::Archive::new()),
        Arc::new(extract::Extract::new()),
        Arc::new(dates::Dates::new()),
        Arc::new(sort::Sort::new()),
        Arc::new(duplicates::Duplicates::new()),
        Arc::new(metadata::Metadata::new()),
        Arc::new(identify::Identify::new()),
        Arc::new(checksum::Checksum::new()),
    ]
}
