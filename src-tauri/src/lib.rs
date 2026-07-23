mod commands;
mod db;
pub mod mcp;
mod mcp_helper;
mod state;
mod tasks;

use mcp_helper::McpHelperState;
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(AppState::new())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(McpHelperState::new(handle.clone()));
            let state = handle.state::<AppState>();
            let handle_for_state = handle.clone();
            tauri::async_runtime::block_on(async {
                state.set_app_handle(handle_for_state).await;
            });

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connection::test_connection,
            commands::connection::connect_database,
            commands::connection::disconnect_database,
            commands::connection::get_default_port,
            commands::query::execute_query,
            commands::query::execute_statements,
            commands::query::get_table_data,
            commands::schema::get_databases,
            commands::schema::get_tables,
            commands::schema::get_columns,
            commands::schema::get_indexes,
            commands::schema::get_enum_values,
            commands::schema::get_schemas,
            commands::tasks::list_tasks,
            commands::tasks::get_task,
            commands::tasks::cancel_task,
            commands::mutation::update_row,
            commands::mutation::delete_rows,
            commands::mutation::insert_row,
            commands::mutation::redis_set_key,
            commands::mutation::redis_delete_key,
            commands::objects::get_views,
            commands::objects::get_functions,
            commands::objects::get_procedures,
            commands::objects::get_triggers,
            commands::objects::get_foreign_keys,
            commands::objects::get_users,
            commands::performance::get_performance_metrics,
            commands::backup::start_backup,
            commands::backup::start_restore,
            commands::table_copy::copy_table,
            commands::export::export_data,
            commands::mcp_helper::mcp_service_status,
            commands::mcp_helper::start_mcp_service,
            commands::mcp_helper::stop_mcp_service,
            commands::mcp_helper::restart_mcp_service,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            let helper = app_handle.state::<McpHelperState>();
            tauri::async_runtime::block_on(helper.shutdown());
        }
    });
}
