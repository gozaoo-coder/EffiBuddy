//! Plugin runtime: dynamic library loading, manifest parsing, registry,
//! permissions, lifecycle, and asset serving for plugin frontends.

pub mod asset_server;
pub mod lifecycle;
pub mod loader;
pub mod manifest;
pub mod permissions;
pub mod registry;
