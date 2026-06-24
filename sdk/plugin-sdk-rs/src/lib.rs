//! plugin-sdk-rs
//! Plugin development SDK (Rust side). Provides PluginTrait, CoreContext,
//! event types, and the `#[plugin_entry]` macro for generating `_plugin_init`.

pub mod context;
pub mod events;
pub mod macros;
pub mod traits;

pub use context::{CoreContext, CoreRequest};
pub use events::{CoreEvent, PluginResponse};
pub use traits::PluginTrait;
