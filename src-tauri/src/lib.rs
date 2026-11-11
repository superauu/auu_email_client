mod account_service;
mod credentials;
mod drafts;
mod error;
mod imap_body;
mod imap_condstore;
mod imap_download;
mod imap_jobs;
mod imap_limits;
mod imap_settings;
mod imap_transport;
mod imap_writeback;
mod mail_attachments;
mod mail_cache;
mod mail_oauth;
mod mail_recovery;
mod mail_store;
mod mail_sync;
mod mail_verify;
mod message_changes;
mod skins;
mod storage;
mod sync_control;
mod sync_scheduler;

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
#[tauri::command]
fn mail_oauth_start(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    account: mail_store::RealAccount,
    config: mail_oauth::Config,
) -> Result<String, AppError> {
    mail_oauth::start(app, db.inner().0.clone(), account, config)
}
#[tauri::command]
fn mail_oauth_status(flow_id: String) -> Result<mail_oauth::Snapshot, AppError> {
    mail_oauth::status(&flow_id)
}
#[tauri::command]
fn mail_oauth_cancel(flow_id: String) -> Result<mail_oauth::Snapshot, AppError> {
    mail_oauth::cancel(&flow_id)
}
#[tauri::command]
fn mail_request_body(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    revision: i64,
) -> Result<mail_store::MessageRow, AppError> {
    let mut conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    imap_body::request(&mut conn, &account_id, &message_id, revision)
}
#[tauri::command]
async fn mail_export_attachment(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    index: usize,
) -> Result<bool, AppError> {
    mail_attachments::export(app, db.inner().0.clone(), account_id, message_id, index).await
}

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
async fn mail_request_recovery(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    revision: i64,
    action: mail_recovery::Action,
) -> Result<mail_store::MessageRow, AppError> {
    let db = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_recovery::request(&mut conn, &account_id, &message_id, revision, action)
    })
    .await
    .map_err(|_| AppError::storage("核对请求未能保存"))?
}

#[tauri::command]
async fn mail_update_message(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    patch: message_changes::Patch,
) -> Result<mail_store::MessageRow, AppError> {
    let db = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        message_changes::update(&mut conn, &account_id, &message_id, &patch)
    })
    .await
    .map_err(|_| AppError::storage("邮件状态保存任务未完成"))?
}

#[tauri::command]
async fn mail_keep_message_local(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
) -> Result<mail_store::MessageRow, AppError> {
    let db = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        message_changes::keep_local(&mut conn, &account_id, &message_id)
    })
    .await
    .map_err(|_| AppError::storage("邮件状态保存任务未完成"))?
}

#[tauri::command]
async fn mail_list_drafts(db: State<'_, Db>) -> Result<Vec<mail_store::MessageRow>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::list(&conn)
    })
    .await
    .map_err(|_| AppError::storage("草稿读取任务未完成"))?
}

#[tauri::command]
async fn mail_save_draft(
    db: State<'_, Db>,
    draft: drafts::Draft,
    previous_account_id: String,
) -> Result<mail_store::MessageRow, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::save(&mut conn, &draft, &previous_account_id)
    })
    .await
    .map_err(|_| AppError::storage("草稿保存任务未完成"))?
}

#[tauri::command]
async fn mail_import_legacy_drafts(
    db: State<'_, Db>,
    drafts: Vec<drafts::Draft>,
) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::import_legacy(&mut conn, &drafts)
    })
    .await
    .map_err(|_| AppError::storage("草稿导入任务未完成"))?
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

#[tauri::command]
fn mail_account_config(db: State<'_, Db>, id: String) -> Result<mail_store::RealAccount, AppError> {
    let conn = db.0.lock().map_err(|_| AppError::storage("数据库忙"))?;
    account_service::configuration(&conn, &id)
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
        if account.oauth.is_some() {
            return Err(AppError::invalid("OAuth 账号请使用浏览器授权入口"));
        }
        let expected = {
            let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
            account_service::existing_generation(&conn, &account)?
        };
        let endpoints = mail_verify::Endpoints {
            receive_only: account.receive_only,
            imap_host: account.imap_host.clone(),
            imap_port: account.imap_port,
            imap_security: account.imap_security,
            smtp_host: account.smtp_host.clone(),
            smtp_port: account.smtp_port,
            auth_user: account.auth_user.clone(),
        };
        mail_verify::ensure_verified(&mail_verify::verify(&endpoints, &secret))?;
        account_service::persist(&conn, account, expected, secret)
    })
    .await
    .map_err(|_| AppError::storage("保存任务未完成"))?
}

#[tauri::command]
async fn mail_delete_account(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    id: String,
) -> Result<(), AppError> {
    scheduler.cancel_account(&id)?;
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        credentials::with_account("imap", &id, || {
            {
                let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
                mail_store::delete_real_account(&mut conn, &id)?;
            }
            mail_sync::forget_session(&id);
            mail_oauth::forget_token(&id);
            credentials::delete("imap", &id)
        })
    })
    .await
    .map_err(|_| AppError::storage("删除任务未完成"))?
}

/// Reads the stored credential from the OS keyring and runs an incremental sync.
/// The secret never leaves this task.
#[tauri::command]
async fn mail_sync_account(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    id: String,
) -> Result<mail_sync::SyncOutcome, AppError> {
    mail_sync::validate_accounts(&db.inner().0, std::slice::from_ref(&id))?;
    {
        let mut conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        imap_jobs::request(&mut conn, std::slice::from_ref(&id))?;
    }
    let ticket = scheduler.ticket(id)?;
    tauri::async_runtime::spawn_blocking(move || ticket.wait())
        .await
        .map_err(|_| AppError::storage("同步任务未完成"))?
}

#[tauri::command]
async fn mail_enqueue_sync(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    ids: Vec<String>,
) -> Result<sync_scheduler::QueueSnapshot, AppError> {
    if ids.is_empty() || ids.len() > 128 {
        return Err(AppError::invalid("每次请选择 1–128 个账号"));
    }
    let conn = db.inner().0.clone();
    let checked = ids.clone();
    tauri::async_runtime::spawn_blocking(move || {
        mail_sync::validate_accounts(&conn, &checked)?;
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        imap_jobs::request(&mut conn, &checked)
    })
    .await
    .map_err(|_| AppError::storage("账号检查未完成"))??;
    scheduler.enqueue(&ids)
}

#[tauri::command]
fn mail_sync_jobs(
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
) -> Result<sync_scheduler::QueueSnapshot, AppError> {
    scheduler.snapshot()
}

#[tauri::command]
fn mail_cancel_sync(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    job_id: String,
) -> Result<sync_scheduler::QueueSnapshot, AppError> {
    let snapshot = scheduler.cancel(&job_id)?;
    if let Some(job) = snapshot.jobs.iter().find(|job| {
        job.id == job_id
            && (job.status == sync_scheduler::JobStatus::Cancelled || job.cancel_requested)
    }) {
        let conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        imap_jobs::pause(&conn, &job.account_id)?;
        drop(conn);
        mail_sync::forget_session(&job.account_id);
    }
    Ok(snapshot)
}
#[tauri::command]
fn mail_imap_settings(
    db: State<'_, Db>,
    account_id: String,
) -> Result<imap_settings::Settings, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    imap_settings::read(&conn, &account_id)
}
#[tauri::command]
fn mail_save_imap_settings(
    db: State<'_, Db>,
    account_id: String,
    settings: imap_settings::Settings,
) -> Result<(), AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    imap_settings::save(&conn, &account_id, &settings)
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
    tauri::async_runtime::spawn_blocking(move || {
        credentials::with_account(&provider, &account_id, || {
            mail_oauth::forget_token(&account_id);
            credentials::set(&provider, &account_id, &secret)
        })
    })
    .await
    .map_err(|_| AppError::credentials("凭据任务未完成"))?
}

#[tauri::command]
async fn credential_delete(provider: String, account_id: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        credentials::with_account(&provider, &account_id, || {
            mail_oauth::forget_token(&account_id);
            mail_sync::forget_session(&account_id);
            credentials::delete(&provider, &account_id)
        })
    })
    .await
    .map_err(|_| AppError::credentials("凭据任务未完成"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let skins = data.join("skins");
            std::fs::create_dir_all(&skins)?;
            mail_cache::recover(&data.join("mail-cache"))
                .map_err(|error| std::io::Error::other(error.message))?;
            let conn = storage::initialize(&data.join("mail.db")).map_err(std::io::Error::other)?;
            let conn = std::sync::Arc::new(Mutex::new(conn));
            let sync_db = conn.clone();
            let scheduler =
                sync_scheduler::SyncScheduler::new(move |id| imap_jobs::run(&sync_db, id))
                    .map_err(|error| std::io::Error::other(error.message))?;
            imap_jobs::spawn(conn.clone(), scheduler.clone())
                .map_err(|error| std::io::Error::other(error.message))?;
            app.manage(scheduler);
            app.manage(Db(conn));
            app.manage(AppPaths { data, skins });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            mail_export_attachment,
            mail_request_body,
            mail_oauth_start,
            mail_account_config,
            mail_oauth_status,
            mail_oauth_cancel,
            mail_imap_settings,
            mail_save_imap_settings,
            mail_request_recovery,
            mail_update_message,
            mail_keep_message_local,
            get_app_info,
            list_user_skins,
            open_skin_directory,
            mail_seed_workspace,
            mail_load_workspace,
            mail_page_messages,
            mail_list_drafts,
            mail_save_draft,
            mail_import_legacy_drafts,
            mail_verify_account,
            mail_save_account,
            mail_delete_account,
            mail_sync_account,
            mail_enqueue_sync,
            mail_sync_jobs,
            mail_cancel_sync,
            credential_status,
            credential_save,
            credential_delete
        ])
        .run(tauri::generate_context!())
        .expect("AuuMail 启动失败");
}
