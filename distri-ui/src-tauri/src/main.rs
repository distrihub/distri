// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use tauri::{menu::{Menu, MenuItem}, tray::TrayIconBuilder};

// Custom commands for Tauri
#[tauri::command]
async fn get_system_info() -> Result<serde_json::Value, String> {
    use std::env;
    
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let family = env::consts::FAMILY;
    
    Ok(serde_json::json!({
        "os": os,
        "arch": arch,
        "family": family
    }))
}

fn main() {
    let context = tauri::generate_context!();
    
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_system_info
        ])
        .setup(|app| {
            
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;
            TrayIconBuilder::new()
            .menu(&menu)
            .on_menu_event(|app, e| if e.id.as_ref() == "quit" { app.exit(0); })
            .build(app)?;
            Ok(())
        })
        .run(context)
        .expect("error while running tauri application");
}