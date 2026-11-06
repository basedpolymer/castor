//! See docs/specs for the owner and the tool list of this crate.

use std::sync::Arc;

use castor_core::Tool;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![]
}
