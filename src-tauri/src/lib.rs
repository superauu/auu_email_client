mod credentials;
mod error;
mod mail_store;
mod mail_verify;
mod skins;
mod storage;

use error::AppError;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

struct AppPaths {
    data: PathBuf,
    skins: PathBuf,
}

struct Db(std::sync::Arc<Mutex<rusqlite::Connection>>);

/// UTC ISO-8601 timestamp for seeding bookkeeping; message times come from the payload.
fn now_utc() -> String {
    // No chrono dependency: build from the Unix epoch with the std library.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm; good until the year 2147483647.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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

#[tauri::command]
async fn mail_seed_workspace(
    db: State<'_, Db>,
    seed: mail_store::SeedWorkspace,
) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::seed_workspace(&mut conn, &seed)
    })
    .await
    .map_err(|_| AppError::storage("写入任务未完成"))?
}

#[tauri::command]
async fn mail_load_workspace(db: State<'_, Db>) -> Result<mail_store::WorkspaceBase, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::load_workspace(&conn)
    })
    .await
    .map_err(|_| AppError::storage("读取任务未完成"))?
}

#[tauri::command]
async fn mail_page_messages(
    db: State<'_, Db>,
    query: mail_store::MessageQuery,
) -> Result<mail_store::MessagePage, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::page_messages(&conn, &query)
    })
    .await
    .map_err(|_| AppError::storage("读取任务未完成"))?
}

#[tauri::command]
async fn mail_verify_account(
    endpoints: mail_verify::Endpoints,
    secret: String,
) -> Result<mail_verify::VerifyReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || Ok(mail_verify::verify(&endpoints, &secret)))
        .await
        .map_err(|_| AppError::network("验证任务未完成"))?
}

/// Verifies, writes the secret to the OS keyring, then persists the account.
/// A failed DB write compensates by deleting the just-written credential.
#[tauri::command]
async fn mail_save_account(
    db: State<'_, Db>,
    account: mail_store::RealAccount,
    secret: String,
) -> Result<String, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let endpoints = mail_verify::Endpoints {
            imap_host: account.imap_host.clone(),
            imap_port: account.imap_port,
            smtp_host: account.smtp_host.clone(),
            smtp_port: account.smtp_port,
            auth_user: account.auth_user.clone(),
        };
        mail_verify::ensure_verified(&mail_verify::verify(&endpoints, &secret))?;
        let id = account
            .id
            .clone()
            .unwrap_or_else(|| format!("imap:{}", uuid::Uuid::new_v4()));
        credentials::set("imap", &id, &secret)?;
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        match mail_store::save_real_account(
            &mut conn,
            &mail_store::RealAccount {
                id: Some(id.clone()),
                ..account
            },
        ) {
            Ok(saved) => Ok(saved),
            Err(error) => {
                let _ = credentials::delete("imap", &id);
                Err(error)
            }
        }
    })
    .await
    .map_err(|_| AppError::storage("保存任务未完成"))?
}

#[tauri::command]
async fn mail_delete_account(db: State<'_, Db>, id: String) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::delete_real_account(&mut conn, &id)?;
        credentials::delete("imap", &id)
    })
    .await
    .map_err(|_| AppError::storage("删除任务未完成"))?
}

#[tauri::command]
async fn credential_status(provider: String, account_id: String) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || credentials::exists(&provider, &account_id))
        .await
        .map_err(|_| AppError::credentials("凭据检查任务未完成"))?
}

#[tauri::command]
async fn credential_save(
    provider: String,
    account_id: String,
    secret: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || credentials::set(&provider, &account_id, &secret))
        .await
        .map_err(|_| AppError::credentials("凭据任务未完成"))?
}

#[tauri::command]
async fn credential_delete(provider: String, account_id: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || credentials::delete(&provider, &account_id))
        .await
        .map_err(|_| AppError::credentials("凭据任务未完成"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let skins = data.join("skins");
            std::fs::create_dir_all(&skins)?;
            let conn = storage::initialize(&data.join("mail.db")).map_err(std::io::Error::other)?;
            app.manage(Db(std::sync::Arc::new(Mutex::new(conn))));
            app.manage(AppPaths { data, skins });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            list_user_skins,
            open_skin_directory,
            mail_seed_workspace,
            mail_load_workspace,
            mail_page_messages,
            mail_verify_account,
            mail_save_account,
            mail_delete_account,
            credential_status,
            credential_save,
            credential_delete
        ])
        .run(tauri::generate_context!())
        .expect("AuuMail 启动失败");
}
