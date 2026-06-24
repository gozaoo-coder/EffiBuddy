//! Core-side trait re-exports + helpers.
//!
//! The canonical `PluginTrait` lives in `plugin-sdk-rs` so plugins compile
//! against the same definition. Core re-exports it here for convenience and
//! adds the FFI function pointer type used by `loader.rs`.

pub use plugin_sdk_rs::{
    CoreContext, CoreEvent, CoreRequest, PluginResponse, PluginTrait,
};

/// FFI signature Core looks up in each backend dynamic library.
/// Note: returning a fat pointer (`*mut dyn PluginTrait`) is not formally
/// FFI-safe, but works reliably for same-compiler Rust-to-Rust calls.
#[allow(improper_ctypes_definitions)]
pub type PluginInitFn = unsafe extern "C" fn() -> *mut dyn PluginTrait;
