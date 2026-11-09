mod agent_tools;
mod commands;
mod confirmations;
mod db;
mod git;
mod llm;
mod models;
mod process_manager;
mod security;

use commands::AppState;
use confirmations::ConfirmationManager;
use db::Database;
use process_manager::ProcessManager;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database = Database::open(&data_dir.join("nomad.sqlite3"))?;
            app.manage(AppState {
                db: Arc::new(database),
                confirmations: Arc::new(ConfirmationManager::default()),
                processes: Arc::new(ProcessManager::default()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::request_confirmation,
            commands::cancel_confirmation,
            commands::confirm_action,
            commands::create_project,
            commands::open_project,
            commands::list_projects,
            commands::list_project_files,
            commands::read_file,
            commands::search_files,
            commands::write_file,
            commands::apply_patch,
            commands::run_command,
            commands::git_diff,
            commands::list_messages,
            commands::list_proposals,
            commands::create_manual_proposal,
            commands::reject_proposal,
            commands::accept_proposal,
            commands::chat_stream,
            commands::save_api_key,
            commands::has_api_key,
            commands::get_app_setting,
            commands::set_app_setting,
            commands::get_project_setting,
            commands::set_project_setting,
            commands::start_dev_server,
            commands::stop_dev_server,
            commands::get_process_status,
            commands::create_snapshot,
            commands::list_snapshots,
            commands::rollback_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("Nomad nu a putut porni");
}
