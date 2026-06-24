//! Desktop Suite Core library. Wires modules and exposes `run()`.

pub mod commands;
pub mod core;
pub mod plugin;
pub mod store;
pub mod traits;

use tauri::Manager;

/// Boot the Core application.
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    log::info!("Desktop Suite Core starting...");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            // Initialize core services and stash in app state.
            let event_bus = core::event_bus::EventBus::new();
            let window_mgr = core::window_mgr::WindowManager::new();
            let config_store = core::config::ConfigStore::load()?;
            let plugin_registry = plugin::registry::PluginRegistry::new();
            let asset_server = plugin::asset_server::AssetServer::new();

            app.manage(event_bus);
            app.manage(window_mgr);
            app.manage(config_store);
            app.manage(plugin_registry);
            app.manage(asset_server);

            // Auto-load enabled plugins from the packages dir.
            tauri::async_runtime::block_on(async {
                if let Err(e) = plugin::lifecycle::autoload_enabled().await {
                    log::warn!("plugin autoload failed: {e}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // window
            commands::window::create_window,
            commands::window::close_window,
            commands::window::show_window,
            commands::window::hide_window,
            commands::window::set_always_on_top,
            commands::window::start_dragging,
            // system
            commands::system::get_system_info,
            // package
            commands::package::list_packages,
            commands::package::install_package,
            commands::package::uninstall_package,
            commands::package::enable_plugin,
            commands::package::disable_plugin,
            commands::package::get_plugin_manifest,
            // config
            commands::config::get_config,
            commands::config::set_config,
        ]);

    app.run(tauri::generate_context!())
        .expect("error while running tauri application");
}
