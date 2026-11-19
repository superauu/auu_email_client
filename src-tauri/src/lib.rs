#[cfg(test)]
mod acceptance;
mod account_service;
mod compose_attach;
mod credentials;
mod drafts;
mod error;
mod imap_body;
mod imap_condstore;
mod imap_download;
mod imap_drafts;
mod imap_folders;
mod imap_jobs;
mod imap_limits;
mod imap_settings;
mod imap_transport;
mod imap_writeback;
mod mail_attachments;
mod mail_cache;
mod mail_html;
mod mail_labels;
mod mail_oauth;
mod mail_recovery;
mod mail_store;
mod mail_sync;
mod mail_trust;
mod mail_verify;
mod message_changes;
#[cfg(test)]
#[path = "../build_support.rs"]
mod oauth_build_tests;
mod outbox;
mod pop3;
mod signature;
mod skins;
mod smtp_send;
mod storage;
mod sync_control;
mod sync_scheduler;

use error::AppError;
use rusqlite::params;
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

/// 发送入口：登记 outbox（语义见 docs/SENDING.md）并唤醒账号队列；发送本身由 runner 执行。
#[tauri::command]
fn mail_send_draft(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    draft_id: String,
    revision: u32,
) -> Result<outbox::Entry, AppError> {
    let entry = {
        let conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        outbox::enqueue(&conn, &draft_id, revision)?
    };
    scheduler.enqueue(std::slice::from_ref(&entry.account_id))?;
    Ok(entry)
}

#[tauri::command]
fn mail_outbox_list(db: State<'_, Db>) -> Result<Vec<outbox::Entry>, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    outbox::list(&conn)
}

#[tauri::command]
fn mail_outbox_cancel(db: State<'_, Db>, id: String) -> Result<outbox::Entry, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    outbox::cancel(&conn, &id)
}

#[tauri::command]
fn mail_outbox_return_draft(db: State<'_, Db>, id: String) -> Result<outbox::Entry, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    outbox::return_to_draft(&conn, &id)
}

/// unknown 的两个用户出口：仅保留本机（failed）或确认重试（queued 并唤醒队列）。
#[tauri::command]
fn mail_outbox_resolve(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    id: String,
    action: String,
) -> Result<outbox::Entry, AppError> {
    let entry = {
        let conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        outbox::resolve(&conn, &id, &action)?
    };
    if entry.state == "queued" {
        scheduler.enqueue(std::slice::from_ref(&entry.account_id))?;
    }
    Ok(entry)
}

#[tauri::command]
fn mail_retry_outbox(
    db: State<'_, Db>,
    scheduler: State<'_, sync_scheduler::SyncScheduler>,
    id: String,
) -> Result<outbox::Entry, AppError> {
    let entry = {
        let conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        outbox::retry(&conn, &id)?
    };
    scheduler.enqueue(std::slice::from_ref(&entry.account_id))?;
    Ok(entry)
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

#[derive(Serialize)]
struct MailHtmlPayload {
    html: String,
}

/// 事件完成状态持久化（mail_events.completed；重开数据库后保留）。
#[tauri::command]
fn mail_update_event(db: State<'_, Db>, id: String, completed: bool) -> Result<(), AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    conn.execute(
        "UPDATE mail_events SET completed=?2 WHERE id=?1",
        params![id, i64::from(completed)],
    )
    .map_err(|_| AppError::storage("无法更新事件状态"))?;
    Ok(())
}

/// 邮件 HTML 正文：从已验证的完整原文缓存按需提取、消毒并拼装沙箱 srcdoc。
#[tauri::command]
fn mail_message_html(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    allow_remote_images: bool,
    // 当前主题模式（light/dark），决定沙箱正文的默认底色与文字颜色。
    mode: Option<String>,
) -> Result<MailHtmlPayload, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    let dark = mode.as_deref() == Some("dark");
    mail_html::render(&conn, &account_id, &message_id, allow_remote_images, dark)
        .map(|html| MailHtmlPayload { html })
}

/// 回复 / 转发引用：消毒后的正文 HTML 片段，直接插入撰写编辑器。
#[tauri::command]
async fn mail_message_html_fragment(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
) -> Result<String, AppError> {
    let (root, key) = {
        let conn = db
            .inner()
            .0
            .lock()
            .map_err(|_| AppError::storage("数据库忙"))?;
        mail_html::fragment_source(&conn, &account_id, &message_id)?
    };
    tauri::async_runtime::spawn_blocking(move || mail_html::render_fragment(&root, &key))
        .await
        .map_err(|_| AppError::invalid("无法准备原邮件引用"))?
}

/// 邮件内链接只在系统默认应用打开；scheme 白名单外的地址一律拒绝。
#[tauri::command]
fn open_external(app: tauri::AppHandle, url: String) -> Result<(), AppError> {
    if url.len() > 2048 || url.chars().any(char::is_control) {
        return Err(AppError::invalid("链接无效"));
    }
    let parsed = url::Url::parse(&url).map_err(|_| AppError::invalid("链接格式无效"))?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(AppError::invalid("不支持打开此类型的链接"));
    }
    app.opener()
        .open_url(parsed.as_str().to_string(), None::<&str>)
        .map_err(|_| AppError::invalid("无法调用系统应用打开链接"))
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
async fn mail_create_folder(
    db: State<'_, Db>,
    account_id: String,
    name: String,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || imap_folders::create(&conn, &account_id, &name))
        .await
        .map_err(|_| AppError::storage("新建目录任务未完成"))?
}

#[tauri::command]
async fn mail_rename_folder(
    db: State<'_, Db>,
    account_id: String,
    folder_id: String,
    new_name: String,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        imap_folders::rename(&conn, &account_id, &folder_id, &new_name)
    })
    .await
    .map_err(|_| AppError::storage("目录重命名任务未完成"))?
}

#[tauri::command]
async fn mail_delete_folder(
    db: State<'_, Db>,
    account_id: String,
    folder_id: String,
) -> Result<Vec<mail_store::FolderRow>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        imap_folders::delete(&conn, &account_id, &folder_id)
    })
    .await
    .map_err(|_| AppError::storage("删除目录任务未完成"))?
}

// 用户自建标签：纯本机注解（labels 表 scope='local' 无账号），无 IMAP 回写。
#[tauri::command]
async fn mail_create_label(
    db: State<'_, Db>,
    name: String,
    color: String,
) -> Result<mail_store::LabelRow, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_labels::create(&conn, &name, &color)
    })
    .await
    .map_err(|_| AppError::storage("新建标签任务未完成"))?
}

#[tauri::command]
async fn mail_rename_label(
    db: State<'_, Db>,
    id: String,
    name: String,
    color: Option<String>,
) -> Result<mail_store::LabelRow, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_labels::rename(&conn, &id, &name, color.as_deref())
    })
    .await
    .map_err(|_| AppError::storage("标签重命名任务未完成"))?
}

#[tauri::command]
async fn mail_delete_label(db: State<'_, Db>, id: String) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_labels::delete(&conn, &id)
    })
    .await
    .map_err(|_| AppError::storage("删除标签任务未完成"))?
}

#[tauri::command]
async fn mail_set_message_labels(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
    label_ids: Vec<String>,
) -> Result<mail_store::MessageRow, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_labels::set_for_message(&mut conn, &account_id, &message_id, &label_ids)
    })
    .await
    .map_err(|_| AppError::storage("保存邮件标签任务未完成"))?
}

#[tauri::command]
async fn mail_purge_message(
    db: State<'_, Db>,
    account_id: String,
    message_id: String,
) -> Result<message_changes::PurgeOutcome, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        message_changes::purge(&mut conn, &account_id, &message_id)
    })
    .await
    .map_err(|_| AppError::storage("永久删除任务未完成"))?
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
async fn mail_folder_counts(db: State<'_, Db>) -> Result<Vec<mail_store::FolderCount>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::folder_counts(&conn)
    })
    .await
    .map_err(|_| AppError::storage("视图计数读取任务未完成"))?
}

#[tauri::command]
async fn mail_contact_suggestions(
    db: State<'_, Db>,
    account_id: String,
) -> Result<Vec<mail_store::ContactRow>, AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        mail_store::contact_suggestions(&conn, &account_id, 100)
    })
    .await
    .map_err(|_| AppError::storage("联系人建议读取任务未完成"))?
}

#[tauri::command]
async fn mail_save_draft(
    db: State<'_, Db>,
    staging: State<'_, compose_attach::ComposeStaging>,
    draft: drafts::Draft,
    previous_account_id: String,
) -> Result<mail_store::MessageRow, AppError> {
    let conn = db.inner().0.clone();
    let staging = staging.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::save(&mut conn, &draft, &previous_account_id, &staging)
    })
    .await
    .map_err(|_| AppError::storage("草稿保存任务未完成"))?
}

/// 放弃草稿：用户显式点击后从本机删除草稿行（已进入发送流程的草稿拒绝删除）。
#[tauri::command]
async fn mail_delete_draft(
    db: State<'_, Db>,
    staging: State<'_, compose_attach::ComposeStaging>,
    draft_id: String,
) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    let staging = staging.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::delete(&mut conn, &draft_id, &staging)
    })
    .await
    .map_err(|_| AppError::storage("草稿删除任务未完成"))?
}

#[tauri::command]
async fn mail_import_legacy_drafts(
    db: State<'_, Db>,
    staging: State<'_, compose_attach::ComposeStaging>,
    drafts: Vec<drafts::Draft>,
) -> Result<(), AppError> {
    let conn = db.inner().0.clone();
    let staging = staging.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = conn.lock().map_err(|_| AppError::storage("数据库忙"))?;
        drafts::import_legacy(&mut conn, &drafts, &staging)
    })
    .await
    .map_err(|_| AppError::storage("草稿导入任务未完成"))?
}

/// 撰写附件选取：Rust 侧多选文件对话框（前端不接触路径），读入暂存区后仅返回元数据。
#[tauri::command]
async fn mail_compose_pick_attachments(
    staging: State<'_, compose_attach::ComposeStaging>,
    app: tauri::AppHandle,
    draft_id: String,
    current_total_bytes: usize,
    current_count: usize,
) -> Result<Vec<drafts::DraftAttachment>, AppError> {
    compose_attach::pick(
        staging.inner().clone(),
        app,
        draft_id,
        current_total_bytes,
        current_count,
    )
    .await
}

/// 服务器草稿上传：本机草稿 → IMAP 草稿目录副本（\Draft \Seen，独立短会话，锁外网络）。
#[tauri::command]
async fn mail_upload_draft(
    db: State<'_, Db>,
    account_id: String,
    draft_id: String,
    revision: u32,
) -> Result<(usize, String), AppError> {
    let conn = db.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        imap_drafts::upload(&conn, &account_id, &draft_id, revision)
    })
    .await
    .map_err(|_| AppError::storage("草稿上传任务未完成"))?
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
            provider: account.provider.clone(),
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
fn mail_account_signature(
    db: State<'_, Db>,
    account_id: String,
) -> Result<signature::Signature, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    signature::read(&conn, &account_id)
}
#[tauri::command]
fn mail_save_account_signature(
    db: State<'_, Db>,
    account_id: String,
    signature: signature::Signature,
) -> Result<(), AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    signature::save(&conn, &account_id, &signature)
}
/// 签名图片选取：Rust 侧文件对话框，返回 data URI（取消返回 null）。
#[tauri::command]
async fn mail_compose_pick_image(
    app: tauri::AppHandle,
) -> Result<Option<signature::PickedImage>, AppError> {
    signature::pick_image(app).await
}

#[tauri::command]
fn mail_trusted_senders(db: State<'_, Db>) -> Result<Vec<mail_trust::TrustedEntry>, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    mail_trust::list(&conn)
}
#[tauri::command]
fn mail_trust_add(
    db: State<'_, Db>,
    kind: String,
    value: String,
) -> Result<mail_trust::TrustedEntry, AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    mail_trust::add(&conn, &kind, &value)
}
#[tauri::command]
fn mail_trust_remove(db: State<'_, Db>, kind: String, value: String) -> Result<(), AppError> {
    let conn = db
        .inner()
        .0
        .lock()
        .map_err(|_| AppError::storage("数据库忙"))?;
    mail_trust::remove(&conn, &kind, &value)
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
            // 调度 runner：同账号先处理 queued 发送（docs/SENDING.md），再执行同步；
            // 同步完成后核对 Sent 目录，命中则把 unknown 发送自动转为已发送。
            let scheduler = sync_scheduler::SyncScheduler::new(move |id| {
                let sends = outbox::run(&sync_db, id)?;
                let mut outcome = imap_jobs::run(&sync_db, id)?;
                {
                    let conn = sync_db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                    outbox::resolve_unknown_after_sync(&conn)?;
                }
                if sends {
                    outcome.refresh_needed = true;
                }
                Ok(outcome)
            })
            .map_err(|error| std::io::Error::other(error.message))?;
            imap_jobs::spawn(conn.clone(), scheduler.clone())
                .map_err(|error| std::io::Error::other(error.message))?;
            app.manage(scheduler);
            app.manage(Db(conn));
            app.manage(compose_attach::ComposeStaging::default());
            app.manage(AppPaths { data, skins });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            mail_export_attachment,
            mail_message_html,
            mail_message_html_fragment,
            mail_update_event,
            mail_request_body,
            mail_send_draft,
            mail_outbox_list,
            mail_outbox_cancel,
            mail_outbox_return_draft,
            mail_outbox_resolve,
            mail_retry_outbox,
            open_external,
            mail_oauth_start,
            mail_account_config,
            mail_oauth_status,
            mail_oauth_cancel,
            mail_imap_settings,
            mail_save_imap_settings,
            mail_account_signature,
            mail_save_account_signature,
            mail_compose_pick_image,
            mail_request_recovery,
            mail_update_message,
            mail_purge_message,
            mail_create_folder,
            mail_rename_folder,
            mail_delete_folder,
            mail_create_label,
            mail_rename_label,
            mail_delete_label,
            mail_set_message_labels,
            mail_keep_message_local,
            get_app_info,
            list_user_skins,
            open_skin_directory,
            mail_seed_workspace,
            mail_load_workspace,
            mail_page_messages,
            mail_folder_counts,
            mail_contact_suggestions,
            mail_list_drafts,
            mail_save_draft,
            mail_delete_draft,
            mail_import_legacy_drafts,
            mail_compose_pick_attachments,
            mail_trusted_senders,
            mail_upload_draft,
            mail_trust_add,
            mail_trust_remove,
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
