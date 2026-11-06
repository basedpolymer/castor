//! Files & archive tools. Owner: see docs/specs.

use std::sync::Arc;

use castor_core::Tool;

mod checksum;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(checksum::Checksum::new())]
}
