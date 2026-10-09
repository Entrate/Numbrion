//! Owner: lead engine author. Families are discovered by build.rs; no manual mods.
//! Manifest metadata/constants are authoritative; only functions enter effect files.
pub mod contract;
pub mod registry;
pub mod support;
pub use contract::*;
pub use registry::{
    HOOK_TRACING_ENABLED, IMPLEMENTATIONS, INCOMPLETE_PORT_WAIVER, audit_registry,
    clear_reached_hooks, dispatch_hook, hook_coverage, pending_hooks, reached_hooks,
    require_complete_registry,
};
