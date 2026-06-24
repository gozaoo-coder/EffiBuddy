//! clock-widget backend.
//!
//! Implements `PluginTrait` and exports `_plugin_init`. On each `Tick` event
//! it returns the current time as a `Frontend` response so the Vue widget
//! can render it.

use plugin_sdk_rs::{
    plugin_entry, CoreContext, CoreEvent, PluginResponse, PluginTrait,
};

mod clock;

pub struct ClockPlugin {
    last_ts: i64,
}

impl Default for ClockPlugin {
    fn default() -> Self {
        Self { last_ts: 0 }
    }
}

impl PluginTrait for ClockPlugin {
    fn id(&self) -> &str {
        "com.desktopsuite.clock"
    }

    fn on_enable(&mut self, ctx: &mut CoreContext) {
        ctx.log("info", "clock-widget enabled");
    }

    fn on_disable(&mut self) {
        // no resources to release
    }

    fn handle_event(&mut self, event: CoreEvent) -> Option<PluginResponse> {
        match event {
            CoreEvent::Tick { ts_ms } => {
                self.last_ts = ts_ms;
                let payload = clock::format_time(ts_ms);
                Some(PluginResponse::Frontend {
                    event: "clock:tick".into(),
                    payload: serde_json::json!({
                        "ts_ms": ts_ms,
                        "formatted": payload.formatted,
                        "date": payload.date,
                        "weekday": payload.weekday,
                    }),
                })
            }
            _ => None,
        }
    }
}

plugin_entry!(ClockPlugin);
