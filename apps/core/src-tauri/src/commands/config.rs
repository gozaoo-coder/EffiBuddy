//! Config read/write commands.

use crate::core::config::ConfigStore;
use serde_json::Value;
use tauri::State;

#[tauri::command]
pub fn get_config(store: State<'_, ConfigStore>, key: Option<String>) -> Value {
    match key {
        Some(k) => store.get(&k).unwrap_or(Value::Null),
        None => store.all(),
    }
}

#[tauri::command]
pub fn set_config(store: State<'_, ConfigStore>, key: String, value: Value) -> Result<(), String> {
    store.set(key, value).map_err(|e| e.to_string())
}
