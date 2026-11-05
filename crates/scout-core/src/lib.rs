//! DataPack Scout assessment core.
//!
//! Milestone 1: a trustworthy local inventory. This crate has no network
//! capability by construction: it depends on no networking crates and uses
//! no standard-library socket APIs, which `tests/no_network.rs` enforces.

pub mod analyze;
pub mod detect;
pub mod guard;
pub mod limits;
pub mod model;
pub mod placeholder;
pub mod report;
pub mod scan;

pub use limits::Limits;
pub use report::{build_data_map, DataMap};
pub use scan::{scan, scan_with_progress, Inventory, Progress, ScanOptions};

pub const APP_NAME: &str = "DataPack Scout";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Version of the calculation rules; bump whenever a metric's meaning changes.
pub const RULESET_VERSION: &str = "inventory-2026.11.1";
