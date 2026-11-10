mod error;
mod skins;
mod storage;

use error::AppError;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

struct AppPaths {
    data: PathBuf,
    skins: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    data_directory: String,
    skin_directory: String,
    database_ready: bool,
}

#[tauri::command]
fn get_app_info(paths: State<'_, AppPaths>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_directory: paths.data.to_string_lossy().into_owned(),
        skin_directory: paths.skins.to_string_lossy().into_owned(),
        database_ready: true,
    }
}

#[tauri::command]
async fn list_user_skins(app: tauri::AppHandle) -> Result<skins::SkinCatalog, AppError> {
    let root = app.state::<AppPaths>().skins.clone();
    tauri::async_runtime::spawn_blocking(move || skins::scan(&root))
        .await
        .map_err(|_| AppError::storage("扫描任务未完成"))?
        .map_err(AppError::storage)
}

#[tauri::command]
fn open_skin_directory(app: tauri::AppHandle, paths: State<'_, AppPaths>) -> Result<(), AppError> {
    // No caller-supplied path or shell command is accepted.
    app.opener()
        .open_path(paths.skins.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::storage("无法打开皮肤目录"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let skins = data.join("skins");
            std::fs::create_dir_all(&skins)?;
            storage::initialize(&data.join("mail.db")).map_err(std::io::Error::other)?;
            app.manage(AppPaths { data, skins });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            list_user_skins,
            open_skin_directory
        ])
        .run(tauri::generate_context!())
        .expect("AuuMail 启动失败");
}
