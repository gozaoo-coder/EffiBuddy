//! `#[plugin_entry]` macro. Generates the `extern "C" fn _plugin_init`
//! symbol Core looks up via `libloading`.
//!
//! Usage:
//! ```ignore
//! #[plugin_entry]
//! struct ClockPlugin { /* ... */ }
//!
//! impl PluginTrait for ClockPlugin { /* ... */ }
//! ```
//!
//! Expands to a `_plugin_init` returning a heap-allocated boxed pointer.

use crate::traits::PluginTrait;

/// Attribute marker. The actual codegen happens in the impl block below;
/// we keep the attribute so users have a clear hook point and so future
/// proc-macro versions can swap in without changing call sites.
pub fn plugin_entry<T: PluginTrait + Default + 'static>() -> *mut dyn PluginTrait {
    let boxed: Box<dyn PluginTrait> = Box::new(T::default());
    Box::into_raw(boxed)
}

#[macro_export]
macro_rules! plugin_entry {
    ($ty:ty) => {
        #[no_mangle]
        pub extern "C" fn _plugin_init() -> *mut dyn $crate::PluginTrait {
            $crate::macros::plugin_entry::<$ty>()
        }
    };
}
