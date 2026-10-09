//! Owner: lead engine author. Families are discovered by build.rs; no manual mods.
//! Manifest metadata/constants are authoritative; only functions enter effect files.
pub mod contract;
pub mod registry;
pub use contract::*;
pub use registry::{IMPLEMENTATIONS, dispatch_hook};
