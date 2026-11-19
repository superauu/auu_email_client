//! Outbox：发送队列与结果语义（docs/SENDING.md）。
//! Message-ID 登记时生成一次、全部尝试复用；DATA 终点号后的不明确结果一律 unknown，
//! 不自动重试、不自动判失败；出口只有用户明确确认或同步后 Sent 目录命中。
use crate::{credentials, drafts, error::AppError, imap_transport, mail_oauth, smtp_send};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SENT_DOMAIN: &str = "auumail.local";
const KEEP_LOCAL_NOTE: &str = "已按仅保留本机处理；服务器是否投递未被确认，不会自动重试";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub id: String,
    pub draft_id: String,
    pub account_id: String,
    pub message_id: Option<String>,
    pub state: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub queued_at: Option<String>,
    pub updated_at: String,
    pub resolved_by: Option<String>,
}

const ENTRY_COLUMNS: &str = "id, draft_id, account_id, message_id, state, attempts, last_error, queued_at, updated_at, resolved_by";

fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        draft_id: row.get(1)?,
        account_id: row.get(2)?,
        message_id: row.get(3)?,
        state: row.get(4)?,
        attempts: row.get::<_, i64>(5)? as u32,
        last_error: row.get(6)?,
        queued_at: row.get(7)?,
        updated_at: row.get(8)?,
        resolved_by: row.get(9)?,
    })
}

pub(crate) fn list(conn: &Connection) -> Result<Vec<Entry>, AppError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM outbox
             WHERE state != 'sent' OR id IN (
               SELECT id FROM outbox WHERE state = 'sent' ORDER BY queued_at DESC, id DESC LIMIT 128
             )
             ORDER BY CASE WHEN resolved_by = 'keep-local' THEN 4
               WHEN state = 'unknown' THEN 0 WHEN state = 'failed' THEN 1
               WHEN state IN ('queued', 'sending') THEN 2 ELSE 3 END,
               queued_at DESC, id DESC"
        ))
        .map_err(|_| AppError::storage("无法读取发送队列"))?;
    let entries = statement
        .query_map([], entry)
        .map_err(|_| AppError::storage("无法读取发送队列"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::storage("发送队列数据无法读取"))?;
    Ok(entries)
}

/// 收件人语法：逗号分隔的纯地址（local@domain），拒绝空白、控制字符与多余 @。
pub(crate) fn parse_recipients(raw: &str) -> Result<Vec<String>, AppError> {
    let mut recipients = Vec::new();
    for address in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let well_formed = address.len() <= 320
            && !address.chars().any(|c| c.is_whitespace() || c.is_control())
            && address.matches('@').count() == 1;
        let Some((local, domain)) = address.split_once('@').filter(|_| well_formed) else {
            return Err(AppError::invalid(format!("收件人地址无效：{address}")));
        };
        if local.is_empty()
            || !local
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&c))
            || domain.is_empty()
            || !domain
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
            || domain.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
            })
        {
            return Err(AppError::invalid(format!("收件人地址无效：{address}")));
        }
        recipients.push(address.to_string());
    }
    if recipients.is_empty() {
        return Err(AppError::invalid("请填写至少一个收件人"));
    }
    Ok(recipients)
}

#[derive(Default)]
pub(crate) struct Recipients {
    pub(crate) to: Vec<String>,
    pub(crate) cc: Vec<String>,
    pub(crate) bcc: Vec<String>,
}

impl Recipients {
    fn envelope(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.to
            .iter()
            .chain(&self.cc)
            .chain(&self.bcc)
            .filter(|address| seen.insert(address.to_ascii_lowercase()))
            .cloned()
            .collect()
    }
}

fn draft_recipients(conn: &Connection, draft_id: &str) -> Result<Recipients, AppError> {
    let (to, cc, bcc): (String, String, String) = conn
        .query_row(
            "SELECT recipients_json, cc, bcc FROM drafts WHERE id = ?1",
            [draft_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| AppError::storage("无法读取草稿收件人"))?;
    let to = serde_json::from_str::<Vec<String>>(&to)
        .map_err(|_| AppError::storage("草稿收件人数据损坏"))?
        .join(", ");
    let optional = |raw: &str| {
        if raw.trim().is_empty() {
            Ok(Vec::new())
        } else {
            parse_recipients(raw)
        }
    };
    let result = Recipients {
        to: optional(&to)?,
        cc: optional(&cc)?,
        bcc: optional(&bcc)?,
    };
    if result.envelope().is_empty() {
        return Err(AppError::invalid("请填写至少一个收件人"));
    }
    Ok(result)
}

/// RFC2047-B：非 ASCII 文本按字符累积切块，单个 encoded-word 连同标记不超过 75 字符，
/// 词间以折行空格分隔（解码时忽略 encoded-word 之间的空白）。
pub(crate) fn encode_rfc2047(text: &str) -> String {
    // 仅对完全安全的展示字符集保持原样；其他（含 CJK、冒号、@ 等结构字符）一律编码，
    // 防止显示名 / 主题夹带头部注入或结构歧义。
    const SAFE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .'-";
    if text.chars().all(|c| SAFE.contains(c)) && text.len() <= 75 {
        return text.to_string();
    }
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        let candidate = format!("{current}{character}");
        let encoded = format!("=?utf-8?B?{}?=", BASE64.encode(candidate.as_bytes()));
        if encoded.len() > 75 && !current.is_empty() {
            words.push(current);
            current = character.to_string();
        } else {
            current = candidate;
        }
    }
    words.push(current);
    words
        .iter()
        .map(|chunk| format!("=?utf-8?B?{}?=", BASE64.encode(chunk.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

/// 发送侧附件载荷：内容在短锁快照内从 SQLite 读出，组装在锁外完成。
#[derive(Clone, Debug)]
pub(crate) struct AttachmentPayload {
    pub name: String,
    pub media_type: String,
    pub content: Vec<u8>,
}

/// RFC5322 组装。头部注入防护：显示名 / 主题中的 CR / LF 替换为空格。
fn fold_base64(data: &str) -> String {
    data.as_bytes()
        .chunks(76)
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "")
        .unwrap_or_default()
        .join("\r\n")
}

/// Message-ID 规范化：中和 CR / LF 后补齐尖括号；空白返回 None。
fn safe_message_id(value: &str) -> Option<String> {
    let cleaned: String = value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    })
}

/// 回复草稿的会话关联头：In-Reply-To = 原邮件 Message-ID，
/// References = 原 References 链（逐条规范化）+ 原 Message-ID。
/// 原邮件已被删除、定位失败或没有 Message-ID 时返回 None（诚实降级，不伪造会话）。
fn reply_headers(
    conn: &Connection,
    account_id: &str,
    reply_to_id: &Option<String>,
) -> Option<ReplyHeaders> {
    let reply_to_id = reply_to_id.as_deref()?;
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT header_message_id, header_references FROM messages WHERE account_id = ?1 AND id = ?2",
            params![account_id, reply_to_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let (message_id, references) = row?;
    let in_reply_to = message_id.as_deref().and_then(safe_message_id)?;
    let mut chain: Vec<String> = references
        .map(|value| {
            value
                .split_whitespace()
                .filter_map(safe_message_id)
                .collect()
        })
        .unwrap_or_default();
    // 原链已含原 Message-ID（例如本机缓存本身就是一封回复）时不再重复。
    if chain.last().map(String::as_str) != Some(in_reply_to.as_str()) {
        chain.push(in_reply_to.clone());
    }
    Some(ReplyHeaders {
        in_reply_to,
        references: Some(chain.join(" ")),
    })
}

/// References 可能是长会话链：按空格折行，保证单行不超过 998 字节上限。
fn fold_references(value: &str) -> String {
    const HEADER_LABEL: &str = "References: ";
    const LINE_LIMIT: usize = 900;
    let mut out = String::from(HEADER_LABEL);
    let mut line_len = HEADER_LABEL.len();
    for token in value.split(' ').filter(|token| !token.is_empty()) {
        if line_len + 1 + token.len() > LINE_LIMIT && line_len > HEADER_LABEL.len() {
            out.push_str("\r\n ");
            line_len = 1;
        } else if line_len > HEADER_LABEL.len() {
            out.push(' ');
            line_len += 1;
        }
        out.push_str(token);
        line_len += token.len();
    }
    out.push_str("\r\n");
    out
}

/// 回复草稿的会话关联头（仅回复且原邮件有 Message-ID 时存在）。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReplyHeaders {
    in_reply_to: String,
    references: Option<String>,
}

/// MIME 组装输入：与账号连接 / 信封无关的草稿内容。
pub(crate) struct MimeInput<'a> {
    pub(crate) from_name: &'a str,
    pub(crate) from_email: &'a str,
    pub(crate) recipients: &'a Recipients,
    pub(crate) subject: &'a str,
    pub(crate) body_text: &'a str,
    pub(crate) body_html: Option<&'a str>,
    pub(crate) message_id: &'a str,
    pub(crate) reply: Option<&'a ReplyHeaders>,
    /// 撰写附件：非空时正文外包一层 multipart/mixed（parts 顺序 = 草稿元数据顺序）。
    pub(crate) attachments: &'a [AttachmentPayload],
}

/// RFC 2231 扩展参数值转义：attribute-char 安全集合之外的字节全部百分号编码。
fn percent_encode_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

const FILENAME_SAFE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .()-_";

fn is_ascii_safe_filename(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| FILENAME_SAFE.contains(c)) && name.len() <= 60
}

/// RFC 2231 扩展参数（仅编码路径）。编码值连同 `base` 与前缀能在 998 字节硬限内时
/// 输出单段 `base*=utf-8''<value>`——mail-parser 0.10.2 接收侧把多段续行按段独立解码
/// 后再拼接，段边界落在 UTF-8 多字节字符中间时会损坏文件名（2026-11-18 真实服务器
/// 验收实测；单段解码正确）。仅编码值超过单行预算（约 800 字符）才回退多段续行：
/// `base*0*=utf-8''首段; base*1*=次段…`，分段不切断 %XX 转义、仅首段带 charset 前缀。
fn rfc2231_parameters(base: &str, name: &str) -> String {
    let percent = percent_encode_value(name);
    let single_budget = 800usize.saturating_sub(base.len() + 1 + "utf-8''".len() + 4);
    if percent.len() <= single_budget {
        return format!("{base}*=utf-8''{percent}");
    }
    let segments: Vec<&str> = percent
        .as_bytes()
        .chunks(63)
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let charset = if index == 0 { "utf-8''" } else { "" };
            format!("{base}*{index}*={charset}{segment}")
        })
        .collect::<Vec<_>>()
        .join(";\r\n ")
}

/// 附件头部值消毒：media_type 仅保留 token 字符，非法回退 octet-stream。
/// 文件名中和 CR/LF 后：
/// - ASCII 安全名保持引号串（与既有格式逐字节一致）；
/// - 非 ASCII 名在 Content-Disposition 使用 RFC 2231 扩展参数（严格解析器如
///   Python email policy.default 的标准形式），Content-Type 的 name 使用引号内
///   RFC2047（Gmail 等宽松客户端回退）。两类参数分置两个头：同一头内裸
///   `filename` 与 `filename*` 并存会被 mail-parser 的续行合并误拼接为原名+重复段。
///
/// 修复背景：裸 RFC2047 参数值不符合 MIME 参数语法，Python email policy.default
/// 直接丢失该参数（STATUS 2026-11-16 记录的兼容问题）。
fn attachment_headers(name: &str, media_type: &str, content: &[u8]) -> String {
    let type_token: String = media_type
        .bytes()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'+' | b'.' | b'_'))
        .map(|c| c as char)
        .collect();
    let safe_type = if type_token.contains('/') && !type_token.starts_with('/') {
        type_token
    } else {
        "application/octet-stream".into()
    };
    let cleaned: String = name
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('"');
    let fallback;
    let cleaned = if cleaned.is_empty() {
        fallback = "attachment.bin".to_string();
        fallback.as_str()
    } else {
        cleaned
    };
    let (type_name_param, disposition_param) = if is_ascii_safe_filename(cleaned) {
        (
            format!("name=\"{cleaned}\""),
            format!("filename=\"{cleaned}\""),
        )
    } else {
        (
            format!("name=\"{}\"", encode_rfc2047(cleaned)),
            rfc2231_parameters("filename", cleaned),
        )
    };
    format!(
        "Content-Type: {safe_type}; {type_name_param}\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; {disposition_param}\r\n\r\n{}",
        fold_base64(&BASE64.encode(content))
    )
}

/// 正文 HTML 内嵌图片（签名 / 未来的正文插图以 `data:image/*` URI 存在草稿里），
/// 发送时转成 multipart/related 的 cid: 内联件。
struct InlineImage {
    content_id: String,
    media_type: String,
    extension: &'static str,
    content: Vec<u8>,
}

/// 解析 needle 之后的 `<subtype>;base64,<payload>`；子类型必须是受限 MIME token。
/// 返回 (完整 media type, 文件扩展名, 解码字节)。
fn decode_inline_uri(uri: &str) -> Option<(String, &'static str, Vec<u8>)> {
    let (subtype, payload) = uri.split_once(";base64,")?;
    if subtype.is_empty()
        || subtype.len() > 40
        || !subtype
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    {
        return None;
    }
    let content = BASE64.decode(payload).ok()?;
    if content.is_empty() {
        return None;
    }
    let extension = match subtype {
        "png" => "png",
        "jpeg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "bmp" => "bmp",
        _ => "img",
    };
    Some((format!("image/{subtype}"), extension, content))
}

/// 从正文 HTML 提取内嵌图片（`src="data:image/…;base64,…"`，compose 消毒输出的固定
/// 双引号形式），解码字节按内容去重（同图多处引用只建一个内联件），src 替换为
/// `cid:<id>`。解码失败或子类型异常的 data URI **原样保留**——不建件、不静默改写
/// 正文，收件端按各自策略处理该 URI。
fn extract_inline_images(html: &str) -> (String, Vec<InlineImage>) {
    const NEEDLE: &str = "src=\"data:image/";
    let mut out = String::with_capacity(html.len());
    let mut images: Vec<InlineImage> = Vec::new();
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut rest = html;
    while let Some(position) = rest.find(NEEDLE) {
        let (head, tail) = rest.split_at(position);
        out.push_str(head);
        rest = &tail[NEEDLE.len()..];
        let Some(end) = rest.find('"') else {
            out.push_str(NEEDLE);
            break;
        };
        let uri = &rest[..end];
        rest = &rest[end + 1..];
        match decode_inline_uri(uri) {
            Some((media_type, extension, content)) => {
                let digest = format!("{:x}", Sha256::digest(&content));
                let content_id = match seen.get(&digest) {
                    Some(existing) => existing.clone(),
                    None => {
                        let content_id = format!("{}@{SENT_DOMAIN}", Uuid::new_v4().simple());
                        seen.insert(digest, content_id.clone());
                        images.push(InlineImage {
                            content_id: content_id.clone(),
                            media_type,
                            extension,
                            content,
                        });
                        content_id
                    }
                };
                out.push_str(&format!("src=\"cid:{content_id}\""));
            }
            None => {
                out.push_str(NEEDLE);
                out.push_str(uri);
                out.push('"');
            }
        }
    }
    out.push_str(rest);
    (out, images)
}

/// 内联图片 part（multipart/related 成员）：Content-Disposition: inline + Content-ID；
/// 文件名是我们生成的 ASCII 名，无需 RFC 2231。末尾为 base64（无 CRLF），与
/// attachment_headers 同一约定。
fn inline_image_headers(image: &InlineImage) -> String {
    let name = format!("inline.{}", image.extension);
    format!(
        "Content-Type: {}; name=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: inline; filename=\"{name}\"\r\nContent-ID: <{}>\r\n\r\n{}",
        image.media_type,
        image.content_id,
        fold_base64(&BASE64.encode(&image.content))
    )
}

pub(crate) fn assemble(input: &MimeInput<'_>) -> String {
    let (
        from_name,
        from_email,
        recipients,
        subject,
        body_text,
        body_html,
        message_id,
        reply,
        attachments,
    ) = (
        input.from_name,
        input.from_email,
        input.recipients,
        input.subject,
        input.body_text,
        input.body_html,
        input.message_id,
        input.reply,
        input.attachments,
    );
    let clean = |text: &str| {
        text.chars()
            .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
            .collect::<String>()
    };
    let safe_name = clean(from_name).trim().to_string();
    let from = if safe_name.is_empty() {
        format!("From: <{from_email}>")
    } else {
        format!("From: {} <{from_email}>", encode_rfc2047(&safe_name))
    };
    let safe_subject = clean(subject).trim().to_string();
    let subject_line = if safe_subject.is_empty() {
        String::new()
    } else {
        format!("Subject: {}\r\n", encode_rfc2047(&safe_subject))
    };
    let text_part = format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        fold_base64(&BASE64.encode(body_text.as_bytes()))
    );
    let (mut content_headers, mut body) = match body_html.filter(|html| !html.trim().is_empty()) {
        Some(html) => {
            let (html, inline) = extract_inline_images(html);
            let boundary = format!("=_auumail_{}", Uuid::new_v4().simple());
            let text_leg = format!("--{boundary}\r\n{text_part}");
            if inline.is_empty() {
                let html_part = format!(
                    "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
                    fold_base64(&BASE64.encode(html.as_bytes()))
                );
                (
                    format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n"),
                    format!("{text_leg}\r\n--{boundary}\r\n{html_part}\r\n--{boundary}--\r\n"),
                )
            } else {
                // alternative > related > [html, image*]：收件端按 cid 关联正文内嵌图。
                let related = format!("=_auumail_{}", Uuid::new_v4().simple());
                let html_part = format!(
                    "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
                    fold_base64(&BASE64.encode(html.as_bytes()))
                );
                let mut parts = vec![format!("--{related}\r\n{html_part}")];
                for image in &inline {
                    parts.push(format!("--{related}\r\n{}", inline_image_headers(image)));
                }
                parts.push(format!("--{related}--"));
                let related_headers =
                    format!("Content-Type: multipart/related; boundary=\"{related}\"\r\n");
                (
                    format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n"),
                    format!(
                        "{text_leg}\r\n--{boundary}\r\n{related_headers}\r\n{}\r\n--{boundary}--\r\n",
                        parts.join("\r\n")
                    ),
                )
            }
        }
        None => (
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n"
                .into(),
            format!("{}\r\n", fold_base64(&BASE64.encode(body_text.as_bytes()))),
        ),
    };
    // 有附件时正文外包一层 multipart/mixed；无附件输出与既有格式逐字节一致。
    // 约定与 alternative 层一致：part 末尾不带 CRLF，由分隔的 "\r\n--boundary" 提供。
    if !attachments.is_empty() {
        let mixed = format!("=_auumail_{}", Uuid::new_v4().simple());
        let inner_body = body.trim_end_matches("\r\n");
        let inner = format!("{content_headers}\r\n{inner_body}");
        let mut parts = vec![format!("--{mixed}\r\n{inner}")];
        for attachment in attachments {
            // attachment_headers 末尾为 base64（无 CRLF）；空内容时以空行结束，仍为合法 part。
            parts.push(format!(
                "--{mixed}\r\n{}",
                attachment_headers(
                    &attachment.name,
                    &attachment.media_type,
                    &attachment.content
                )
            ));
        }
        parts.push(format!("--{mixed}--\r\n"));
        content_headers = format!("Content-Type: multipart/mixed; boundary=\"{mixed}\"\r\n");
        body = parts.join("\r\n");
    }
    let to = if recipients.to.is_empty() {
        "undisclosed-recipients:;".into()
    } else {
        recipients.to.join(",\r\n ")
    };
    let cc = if recipients.cc.is_empty() {
        String::new()
    } else {
        format!("Cc: {}\r\n", recipients.cc.join(",\r\n "))
    };
    // 回复关联头：In-Reply-To 指向原邮件 Message-ID，References 携带完整会话链。
    let (in_reply_to, references_line) = match reply {
        Some(headers) => (
            format!("In-Reply-To: {}\r\n", headers.in_reply_to),
            headers
                .references
                .as_deref()
                .map(fold_references)
                .unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    };
    format!(
        "{from}\r\nTo: {to}\r\n{cc}{in_reply_to}{references_line}{subject_line}Date: {}\r\nMessage-ID: {message_id}\r\nMIME-Version: 1.0\r\n{content_headers}\r\n{body}",
        Utc::now().to_rfc2822(),
    )
}

/// 登记发送：校验修订 / 能力 / 收件人 / 附件限额，生成一次 Message-ID 并全程复用。
pub(crate) fn enqueue(conn: &Connection, draft_id: &str, revision: u32) -> Result<Entry, AppError> {
    let draft: Option<(String, u32, String, String, String, String)> = conn
        .query_row(
            "SELECT id, revision, recipients_json, subject, body_text, attachments_json FROM drafts WHERE id = ?1",
            [draft_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, String>(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取草稿"))?;
    let Some((_, stored_revision, _, _, _, attachments_json)) = draft else {
        return Err(AppError::invalid("草稿不存在"));
    };
    if stored_revision != revision {
        return Err(AppError::conflict("草稿已被修改，请保存后重新发送"));
    }
    // 防御纵深：保存时已校验，登记时再核对元数据限额（不读内容）。
    let attachments: Vec<drafts::DraftAttachment> = serde_json::from_str(&attachments_json)
        .map_err(|_| AppError::storage("草稿附件数据损坏"))?;
    drafts::validate_attachments(&attachments)?;
    draft_recipients(conn, draft_id)?;
    let blocked: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = ?1)",
            [draft_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::storage("无法检查发送状态"))?;
    if blocked {
        return Err(AppError::conflict(
            "这封草稿已进入发送流程，请保留当前内容并另建草稿。",
        ));
    }
    let (account_id, profile): (String, String) = conn
        .query_row(
            "SELECT id, profile_json FROM accounts WHERE id = (SELECT account_id FROM drafts WHERE id = ?1)",
            [draft_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::storage("无法读取账号配置"))?;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接服务器，也不能发送邮件"));
    }
    let google_oauth = profile
        .get("oauth")
        .and_then(|oauth| oauth.get("provider"))
        .and_then(|provider| provider.as_str())
        == Some("google");
    // Google OAuth 发送走 smtp.gmail.com，不受历史「仅收信」标记限制（OAuth 曾默认只收信）。
    if !google_oauth && profile["receiveOnly"].as_bool().unwrap_or(false) {
        return Err(AppError::invalid(
            "仅收信账号不支持发送，请使用完整 SMTP 账号",
        ));
    }
    if profile["smtpHost"].as_str().unwrap_or("").trim().is_empty() && !google_oauth {
        return Err(AppError::invalid("账号未配置 SMTP 服务器，无法发送"));
    }
    if profile
        .get("oauth")
        .and_then(|value| value.get("provider"))
        .and_then(|provider| provider.as_str())
        == Some("microsoft")
    {
        return Err(AppError::invalid(
            "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用",
        ));
    }
    let message_id = format!(
        "<{}.{}@{SENT_DOMAIN}>",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let entry_id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO outbox (id, account_id, draft_id, state, idempotency_key, message_id, attempts, queued_at, updated_at)
         VALUES (?1, ?2, ?3, 'queued', ?4, ?4, 0, ?5, ?5)",
        params![entry_id, account_id, draft_id, message_id, now()],
    )
    .map_err(|_| AppError::storage("发送队列写入失败"))?;
    current(conn, &entry_id, "queued")
}

struct SendJob {
    entry_id: String,
    account_id: String,
    message_id: String,
    generation: String,
    from_name: String,
    from_email: String,
    smtp_host: String,
    smtp_port: u16,
    auth_user: String,
    oauth: Option<mail_oauth::Config>,
    subject: String,
    body_text: String,
    body_html: String,
    recipients: Recipients,
    /// 撰写附件内容（短锁内读出，锁外组装）。
    attachments: Vec<AttachmentPayload>,
    /// 回复草稿的会话关联头；原邮件缺失 / 无 Message-ID 时为 None（诚实降级）。
    reply: Option<ReplyHeaders>,
}

/// 短锁快照：取下一个 queued 任务 + 账号 / 草稿信息，并标记 sending。
fn snapshot_job(conn: &Connection, account_id: &str) -> Result<Option<SendJob>, AppError> {
    let Some(row) = conn
        .query_row(
            &format!(
                "SELECT {} FROM outbox WHERE account_id = ?1 AND state = 'queued' ORDER BY queued_at LIMIT 1",
                ENTRY_COLUMNS
            ),
            [account_id],
            entry,
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取发送任务"))?
    else {
        return Ok(None);
    };
    let (generation, display_name, from_email, profile): (String, String, String, String) = conn
        .query_row(
            "SELECT sync_generation, display_name, email, profile_json FROM accounts WHERE id = ?1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| AppError::storage("无法读取账号配置"))?;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).map_err(|_| AppError::storage("账号配置损坏"))?;
    if profile["status"] == "demo" {
        return Err(AppError::invalid("演示账号不会连接服务器，也不能发送邮件"));
    }
    /// (revision, recipients_json, subject, body_text, body_html, reply_to_id, attachments_json)
    type DraftSnapshot = (u32, String, String, String, String, Option<String>, String);
    let draft: Option<DraftSnapshot> = conn
        .query_row(
            "SELECT revision, recipients_json, subject, body_text, body_html, reply_to_id, attachments_json FROM drafts WHERE id = ?1",
            [&row.draft_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(|_| AppError::storage("无法读取草稿"))?;
    let Some((_, _, subject, body_text, body_html, reply_to_id, attachments_json)) = draft else {
        return Err(AppError::invalid("草稿已不存在，无法发送"));
    };
    // 附件内容：按元数据逐个从子表读取并核对 sha256；缺失即失败，不发送不完整的邮件。
    let metadata: Vec<drafts::DraftAttachment> = serde_json::from_str(&attachments_json)
        .map_err(|_| AppError::storage("草稿附件数据损坏"))?;
    let mut attachments = Vec::with_capacity(metadata.len());
    for item in metadata {
        let (sha, content): (String, Vec<u8>) = conn
            .query_row(
                "SELECT sha256, content FROM draft_attachment_blobs WHERE draft_id = ?1 AND attachment_id = ?2",
                params![row.draft_id, item.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::storage("无法读取附件内容"))?
            .ok_or_else(|| AppError::storage(format!("附件「{}」内容缺失，请重新添加后再发送。", item.name)))?;
        if sha != item.sha256 || content.len() != item.size {
            return Err(AppError::storage(format!(
                "附件「{}」与本机记录不一致，请重新添加后再发送。",
                item.name
            )));
        }
        attachments.push(AttachmentPayload {
            name: item.name,
            media_type: item.media_type,
            content,
        });
    }
    let recipients = draft_recipients(conn, &row.draft_id)?;
    let reply = reply_headers(conn, account_id, &reply_to_id);
    conn.execute(
        "UPDATE outbox SET state = 'sending', attempts = attempts + 1, updated_at = ?2 WHERE id = ?1 AND state = 'queued'",
        params![row.id, now()],
    )
    .map_err(|_| AppError::storage("无法更新发送状态"))?;
    Ok(Some(SendJob {
        entry_id: row.id,
        account_id: account_id.to_string(),
        message_id: row.message_id.unwrap_or_default(),
        generation,
        from_name: display_name,
        from_email,
        smtp_host: profile["smtpHost"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string(),
        smtp_port: profile["smtpPort"].as_u64().unwrap_or(465) as u16,
        auth_user: profile["authUser"].as_str().unwrap_or("").to_string(),
        oauth: profile.get("oauth").and_then(|value| {
            serde_json::from_value::<mail_oauth::Config>(value.clone())
                .ok()
                .map(mail_oauth::Config::with_builtin_client_id)
        }),
        subject,
        body_text,
        body_html,
        recipients,
        attachments,
        reply,
    }))
}

/// Google OAuth 空 SMTP 端口时默认 465 TLS。
fn smtp_port(job: &SendJob, smtp_host: &str) -> u16 {
    if job.smtp_port == 0
        || (job.smtp_host.is_empty() && smtp_host == "smtp.gmail.com" && job.smtp_port != 587)
    {
        465
    } else {
        job.smtp_port
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 短事务写终态；行已被并发改写（重启恢复等）时保持现状。
fn finish(
    conn: &Connection,
    entry_id: &str,
    result: &Result<(), AppError>,
) -> Result<(), AppError> {
    let (state, error) = match result {
        Ok(()) => ("sent", None),
        Err(error) if error.code == "unknown" => ("unknown", Some(error.message.clone())),
        Err(error) => ("failed", Some(error.message.clone())),
    };
    conn.execute(
        "UPDATE outbox SET state = ?2, last_error = ?3, updated_at = ?4 WHERE id = ?1 AND state = 'sending'",
        params![entry_id, state, error, now()],
    )
    .map_err(|_| AppError::storage("无法写入发送结果"))?;
    Ok(())
}

/// 锁外投递：凭据读取与 SMTP 会话都不持有 DB 锁。
fn deliver(db: &crate::mail_sync::SharedDb, job: &SendJob) -> Result<(), AppError> {
    let google_oauth =
        matches!(&job.oauth, Some(config) if config.provider == mail_oauth::Provider::Google);
    if job.smtp_host.is_empty() && !google_oauth {
        return Err(AppError::invalid("账号未配置 SMTP 服务器，无法发送"));
    }
    // Google OAuth 账号固定走 smtp.gmail.com（scope https://mail.google.com/ 覆盖 SMTP）。
    let smtp_host = if job.smtp_host.is_empty() {
        "smtp.gmail.com".to_string()
    } else {
        job.smtp_host.clone()
    };
    let security = if smtp_port(job, &smtp_host) == 587 {
        imap_transport::Security::Starttls
    } else {
        imap_transport::Security::Tls
    };
    let raw = assemble(&MimeInput {
        from_name: &job.from_name,
        from_email: &job.from_email,
        recipients: &job.recipients,
        subject: &job.subject,
        body_text: &job.body_text,
        body_html: Some(job.body_html.as_str()).filter(|html| !html.trim().is_empty()),
        message_id: &job.message_id,
        reply: job.reply.as_ref(),
        attachments: &job.attachments,
    });
    let stream = smtp_send::open(&smtp_host, job.smtp_port, security)?;
    enum PreparedAuth {
        Plain(String),
        Xoauth2(String),
    }
    let prepared = match &job.oauth {
        Some(config) => {
            if config.provider == mail_oauth::Provider::Microsoft {
                return Err(AppError::invalid(
                    "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用",
                ));
            }
            if config.provider == mail_oauth::Provider::Google
                && !job.smtp_host.is_empty()
                && !job.smtp_host.eq_ignore_ascii_case("smtp.gmail.com")
            {
                return Err(AppError::invalid(
                    "Google OAuth 账号只能使用 smtp.gmail.com 发送",
                ));
            }
            PreparedAuth::Xoauth2(mail_oauth::access(
                db,
                &job.account_id,
                &job.generation,
                &job.auth_user,
                config,
            )?)
        }
        None => PreparedAuth::Plain(credentials::with_account("imap", &job.account_id, || {
            credentials::secret_for("imap", &job.account_id)
        })?),
    };
    let auth = match &prepared {
        PreparedAuth::Plain(secret) => smtp_send::SendAuth::Plain {
            user: &job.auth_user,
            secret,
        },
        PreparedAuth::Xoauth2(token) => smtp_send::SendAuth::Xoauth2 {
            user: &job.auth_user,
            token,
        },
    };
    smtp_send::send_session(
        stream,
        &auth,
        &job.from_email,
        &job.recipients.envelope(),
        raw.as_bytes(),
    )
}

pub(crate) fn run(db: &crate::mail_sync::SharedDb, account_id: &str) -> Result<bool, AppError> {
    let mut processed = false;
    loop {
        // 入队后才发现的问题（草稿被删、收件人损坏等）不能让行卡在 queued：直接判 failed 继续。
        let snapshot = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            snapshot_job(&conn, account_id)
        };
        let job = match snapshot {
            Ok(job) => job,
            Err(error) => {
                let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
                if let Some(row) = conn
                    .query_row(
                        &format!("SELECT {} FROM outbox WHERE account_id = ?1 AND state = 'queued' ORDER BY queued_at LIMIT 1", ENTRY_COLUMNS),
                        [account_id],
                        entry,
                    )
                    .optional()
                    .map_err(|_| AppError::storage("无法读取发送任务"))?
                {
                    conn.execute(
                        "UPDATE outbox SET state = 'failed', last_error = ?3, updated_at = ?2 WHERE id = ?1",
                        params![row.id, now(), error.message],
                    )
                    .map_err(|_| AppError::storage("无法写入发送状态"))?;
                    processed = true;
                    continue;
                }
                return Err(error);
            }
        };
        let Some(job) = job else { break };
        let result = deliver(db, &job);
        {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            finish(&conn, &job.entry_id, &result)?;
        }
        processed = true;
    }
    Ok(processed)
}

pub(crate) fn recover(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "UPDATE outbox SET state = 'unknown', last_error = '上次发送中断，结果待核对；不会自动重发', updated_at = ?1 WHERE state = 'sending'",
        params![now()],
    )
    .map_err(|_| AppError::storage("无法恢复发送队列"))?;
    Ok(())
}

/// 同步提交后核对：Sent 目录出现相同 Message-ID → unknown 自动转 sent；
/// 未命中保持 unknown（服务器不保存 Sent 副本不构成未送达证据）。
pub(crate) fn resolve_unknown_after_sync(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "UPDATE outbox SET state = 'sent', resolved_by = 'sent-folder', last_error = NULL, updated_at = ?1
         WHERE state = 'unknown' AND message_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM messages m
           JOIN message_folders mf ON mf.account_id = m.account_id AND mf.message_id = m.id
           JOIN folders f ON f.account_id = mf.account_id AND f.id = mf.folder_id AND f.kind = 'sent'
           WHERE m.header_message_id = outbox.message_id AND m.account_id = outbox.account_id)",
        params![now()],
    )
    .map_err(|_| AppError::storage("无法核对已发送邮件"))?;
    Ok(())
}

/// 用户出口：unknown → 仅保留本机（failed，不自动重试）或确认重试（queued，Message-ID 不变）。
pub(crate) fn resolve(conn: &Connection, entry_id: &str, action: &str) -> Result<Entry, AppError> {
    let state = match action {
        "keep-local" => {
            conn.execute(
                "UPDATE outbox SET state = 'failed', resolved_by = 'keep-local', last_error = ?3, updated_at = ?2 WHERE id = ?1 AND state = 'unknown'",
                params![entry_id, now(), KEEP_LOCAL_NOTE],
            )
            .map_err(|_| AppError::storage("无法写入发送状态"))?;
            "failed"
        }
        "retry" => {
            conn.execute(
                "UPDATE outbox SET state = 'queued', last_error = NULL, updated_at = ?2 WHERE id = ?1 AND state = 'unknown'",
                params![entry_id, now()],
            )
            .map_err(|_| AppError::storage("无法写入发送状态"))?;
            "queued"
        }
        _ => return Err(AppError::invalid("未知的处理方式")),
    };
    current(conn, entry_id, state)
}

pub(crate) fn retry(conn: &Connection, entry_id: &str) -> Result<Entry, AppError> {
    let changed = conn
        .execute(
            "UPDATE outbox SET state = 'queued', last_error = NULL, updated_at = ?2
         WHERE id = ?1 AND state = 'failed' AND COALESCE(resolved_by, '') != 'keep-local'",
            params![entry_id, now()],
        )
        .map_err(|_| AppError::storage("无法重排发送任务"))?;
    if changed != 1 {
        return Err(AppError::conflict(
            "只能重试明确失败的任务；仅保留本机的任务不能再次发送。",
        ));
    }
    current(conn, entry_id, "queued")
}

/// 明确失败后退回草稿：只解除 outbox 占用，正文、附件、账号与 revision 均不改。
/// keep-local 来自 unknown，并非明确拒绝，不能通过此入口绕过防重复发送保护。
pub(crate) fn return_to_draft(conn: &Connection, entry_id: &str) -> Result<Entry, AppError> {
    conn.query_row(
        &format!("DELETE FROM outbox WHERE id = ?1 AND state = 'failed'
          AND COALESCE(resolved_by, '') != 'keep-local'
          AND EXISTS(SELECT 1 FROM drafts WHERE drafts.id = outbox.draft_id AND drafts.account_id = outbox.account_id)
          RETURNING {ENTRY_COLUMNS}"),
        [entry_id],
        entry,
    )
    .optional()
    .map_err(|_| AppError::storage("无法退回本机草稿"))?
    .ok_or_else(|| AppError::conflict("发送状态已变化；只有明确失败的任务可以返回草稿修改。"))
}

pub(crate) fn cancel(conn: &Connection, entry_id: &str) -> Result<Entry, AppError> {
    // 仅排队中可取消：取回条目后删除，草稿解除占用恢复可编辑。
    let entry = current(conn, entry_id, "queued")?;
    let deleted = conn
        .execute(
            "DELETE FROM outbox WHERE id = ?1 AND state = 'queued'",
            params![entry_id],
        )
        .map_err(|_| AppError::storage("无法取消发送任务"))?;
    if deleted == 0 {
        return Err(AppError::conflict("该任务已开始发送或已完成，不能取消"));
    }
    Ok(entry)
}

fn current(conn: &Connection, entry_id: &str, expected: &str) -> Result<Entry, AppError> {
    let row = conn
        .query_row(
            &format!("SELECT {ENTRY_COLUMNS} FROM outbox WHERE id = ?1"),
            [entry_id],
            entry,
        )
        .optional()
        .map_err(|e| AppError::storage(format!("无法读取发送任务: {e}")))?;
    let entry = row.ok_or_else(|| AppError::storage("发送任务已不存在"))?;
    if entry.state != expected {
        return Err(AppError::conflict("发送状态已变化，请刷新后查看"));
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn db() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db.execute_batch(
            "INSERT INTO accounts (id, provider, email, display_name, created_at, profile_json) VALUES
             ('real', 'imap', 'me@example.test', '测试发件人', '2026-11-14T00:00:00Z',
              '{\"status\":\"connected\",\"smtpHost\":\"smtp.example.test\",\"smtpPort\":465,\"authUser\":\"me@example.test\",\"receiveOnly\":false}');",
        )
        .unwrap();
        db.execute_batch("INSERT INTO drafts (id, account_id, recipients_json, subject, body_text, reply_to_id, updated_at, revision) VALUES
          ('draft-1', 'real', '[\"friend@example.test\"]', '你好', '正文', NULL, '2026-11-14T00:00:00Z', 3);")
            .unwrap();
        db
    }

    #[test]
    fn enqueue_records_message_id_and_blocks_duplicates() {
        let conn = db();
        let first = enqueue(&conn, "draft-1", 3).unwrap();
        assert_eq!(first.state, "queued");
        assert!(first.message_id.as_deref().unwrap().starts_with('<'));
        assert_eq!(first.queued_at.as_deref(), Some(first.updated_at.as_str()));
        let again = enqueue(&conn, "draft-1", 3).unwrap_err();
        assert_eq!(again.code, "conflict");
        // 修订不匹配拒绝；Message-ID 必须保持不变（幂等）。
        assert_eq!(enqueue(&conn, "draft-1", 9).unwrap_err().code, "conflict");
        assert_eq!(list(&conn).unwrap()[0].message_id, first.message_id);
    }

    #[test]
    fn receive_only_and_microsoft_and_bad_recipients_are_rejected_at_enqueue() {
        let conn = db();
        // Google OAuth 即使带历史「仅收信」标记也可发送（走 smtp.gmail.com）。
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('true')), '$.oauth', json('{\"provider\":\"google\",\"clientId\":\"id\",\"tenant\":\"common\"}')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap().state, "queued");
        conn.execute("DELETE FROM outbox", []).unwrap();
        // 非 OAuth 的仅收信账号仍然拒绝。
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('true')), '$.oauth', json('null')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.receiveOnly', json('false')), '$.oauth', json('{\"provider\":\"microsoft\",\"clientId\":\"id\",\"tenant\":\"common\"}')) WHERE id = 'real'",
            [],
        )
        .unwrap();
        assert_eq!(
            enqueue(&conn, "draft-1", 3).unwrap_err().message,
            "Microsoft 账号尚未授权 SMTP.Send，发送暂不可用"
        );
        conn.execute(
            "UPDATE drafts SET recipients_json = '[\"not-an-address\"]'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
    }

    #[test]
    fn draft_revision_mismatch_blocks_enqueue() {
        let conn = db();
        assert_eq!(enqueue(&conn, "draft-1", 99).unwrap_err().code, "conflict");
    }

    #[test]
    fn google_oauth_without_smtp_host_still_enqueues() {
        // OAuth 账号创建时只填 IMAP：发送默认 smtp.gmail.com，不能因空 SMTP 主机被拒。
        let conn = db();
        conn.execute(
            "UPDATE accounts SET profile_json = json_set(json_set(profile_json, '$.oauth', json('{\"provider\":\"google\",\"clientId\":\"id\",\"tenant\":\"common\"}')), '$.smtpHost', '') WHERE id = 'real'",
            [],
        )
        .unwrap();
        let entry = enqueue(&conn, "draft-1", 3).unwrap();
        assert_eq!(entry.state, "queued");
    }

    #[test]
    fn run_marks_credential_failure_failed_and_keeps_draft_locked() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let db = std::sync::Arc::new(std::sync::Mutex::new(conn));
        // 密码凭据缺失（测试环境没有 keyring）：任务转 failed，错误不含机密。
        let processed = run(&db, "real").unwrap();
        assert!(processed);
        let conn = db.lock().unwrap();
        let entries = list(&conn).unwrap();
        assert_eq!(entries[0].state, "failed");
        assert!(entries[0].last_error.is_some());
        assert_eq!(entries[0].attempts, 1);
        // 草稿仍被 outbox 占用（drafts::write 的守卫覆盖），不会被意外改写。
        let locked: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE draft_id = 'draft-1')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(locked);
    }

    #[test]
    fn finish_maps_result_codes_to_states() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        conn.execute("UPDATE outbox SET state = 'sending', attempts = 1", [])
            .unwrap();
        finish(&conn, &list(&conn).unwrap()[0].id, &Ok(())).unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "sent");
        conn.execute("UPDATE outbox SET state = 'sending'", [])
            .unwrap();
        finish(
            &conn,
            &list(&conn).unwrap()[0].id,
            &Err(AppError {
                code: "unknown",
                message: "未确认".into(),
                retryable: false,
            }),
        )
        .unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "unknown");
        assert_eq!(entry.last_error.as_deref(), Some("未确认"));
        conn.execute("UPDATE outbox SET state = 'sending'", [])
            .unwrap();
        finish(
            &conn,
            &list(&conn).unwrap()[0].id,
            &Err(AppError::invalid("拒绝")),
        )
        .unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "failed");
    }

    #[test]
    fn recover_turns_interrupted_sending_into_unknown() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        conn.execute("UPDATE outbox SET state = 'sending'", [])
            .unwrap();
        recover(&conn).unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "unknown");
        assert!(entry.last_error.as_deref().unwrap().contains("中断"));
        // queued 与终态不受影响。
        enqueue(&conn, "draft-1", 3).unwrap_err();
    }

    #[test]
    fn unknown_resolves_via_sent_folder_or_user_exits() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let message_id = list(&conn).unwrap()[0].message_id.clone().unwrap();
        conn.execute("UPDATE outbox SET state = 'unknown'", [])
            .unwrap();
        // 没有命中时保持 unknown。
        resolve_unknown_after_sync(&conn).unwrap();
        assert_eq!(list(&conn).unwrap()[0].state, "unknown");
        // 服务器 Sent 目录出现相同 Message-ID → 自动 resolved-sent。
        conn.execute(
            "INSERT INTO folders (id, account_id, remote_id, name, kind) VALUES ('f1', 'real', 'sent', 'Sent', 'sent')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, received_at, header_message_id)
             VALUES ('m-sent', 'real', 'r1', 't', '你好', '测试发件人', 'me@example.test', '2026-11-14T00:00:00Z', ?1)",
            params![message_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_folders (account_id, message_id, folder_id) VALUES ('real', 'm-sent', 'f1')",
            [],
        )
        .unwrap();
        resolve_unknown_after_sync(&conn).unwrap();
        let entry = &list(&conn).unwrap()[0];
        assert_eq!(entry.state, "sent");
        assert_eq!(entry.resolved_by.as_deref(), Some("sent-folder"));
        assert_eq!(entry.last_error, None);
    }

    #[test]
    fn assembly_encodes_cjk_headers_and_folds_body() {
        let raw = assemble(&MimeInput {
            from_name: "测试发件人",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "你好 · 收据",
            body_text: "第一行\n第二行",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[],
        });
        assert!(raw.starts_with("From: =?utf-8?B?"));
        assert!(raw.contains("\r\nTo: friend@example.test\r\n"));
        assert!(raw.contains("Subject: =?utf-8?B?"));
        assert!(raw.contains("Date: "));
        assert!(raw.contains("Message-ID: <abc@auumail.local>"));
        assert!(raw.contains(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"
        ));
        assert!(!raw.contains("multipart"));
        // base64 正文按 76 字符折行（取最后一段双 CRLF 之后的内容）。
        let body = raw.split("\r\n\r\n").last().unwrap();
        assert!(body.lines().all(|line| line.len() <= 76));
        let decoded = BASE64.decode(body.replace("\r\n", "")).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "第一行\n第二行");
    }

    #[test]
    fn html_body_becomes_multipart_alternative() {
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "rich",
            body_text: "plain text",
            body_html: Some("<p>rich <b>text</b></p>"),
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[],
        });
        assert!(raw.contains("Content-Type: multipart/alternative; boundary=\"=_auumail_"));
        assert!(raw.contains("Content-Type: text/plain; charset=utf-8"));
        assert!(raw.contains("Content-Type: text/html; charset=utf-8"));
        let boundary = raw
            .split("boundary=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        assert_eq!(
            raw.matches(&format!("--{boundary}")).count(),
            3,
            "开边界两次 + 闭边界一次"
        );
        assert!(raw.trim_end().ends_with("--"));
        // 两个部分都能独立解码。
        let encoded = raw
            .split("text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let decoded = String::from_utf8(BASE64.decode(encoded).unwrap()).unwrap();
        assert_eq!(decoded, "<p>rich <b>text</b></p>");
        // 纯文本部分保留。
        assert!(raw.contains("cGxhaW4gdGV4dA==") || raw.contains(&BASE64.encode(b"plain text")));
    }

    #[test]
    fn data_uri_images_become_related_inline_parts_with_content_dedupe() {
        use mail_parser::{MessageParser, MimeHeaders};
        let png: Vec<u8> = vec![137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3];
        let encoded = BASE64.encode(&png);
        let html = format!(
            "<p>张三</p><img src=\"data:image/png;base64,{encoded}\" alt=\"logo\"><img src=\"data:image/png;base64,{encoded}\">"
        );
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "签名",
            body_text: "签名文本",
            body_html: Some(&html),
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[],
        });
        assert!(raw.contains("Content-Type: multipart/alternative; boundary=\"=_auumail_"));
        assert!(raw.contains("Content-Type: multipart/related; boundary=\"=_auumail_"));
        assert!(raw.contains("Content-Disposition: inline; filename=\"inline.png\""));
        // mail_parser 回读（与收信渲染同一路径）：两处 src 都替换为 cid，同图去重只建一个内联件。
        let parsed = MessageParser::default().parse(raw.as_bytes()).unwrap();
        let html = parsed.body_html(0).unwrap().into_owned();
        let cids: Vec<String> = html
            .match_indices("cid:")
            .map(|(index, _)| {
                let start = index + 4;
                let end = html[start..].find('"').unwrap() + start;
                html[start..end].to_string()
            })
            .collect();
        assert_eq!(cids.len(), 2, "两处引用都替换为 cid");
        assert_eq!(cids[0], cids[1], "同图按内容去重");
        assert!(cids[0].ends_with("@auumail.local"));
        assert!(!html.contains("data:image/png"), "原文 data URI 不残留");
        let inline: Vec<_> = parsed
            .attachments()
            .filter(|part| part.content_id().is_some())
            .collect();
        assert_eq!(inline.len(), 1, "同图只建一个内联件");
        assert_eq!(
            inline[0].content_id().unwrap(),
            cids[0],
            "Content-ID 与 cid 引用对应"
        );
        assert_eq!(inline[0].contents(), png, "图片字节逐字一致");
    }

    #[test]
    fn undecodable_data_uri_is_kept_verbatim_without_related_parts() {
        use mail_parser::MessageParser;
        let broken = "data:image/png;base64,!not-base64!";
        let html = format!("<p><img src=\"{broken}\"></p>");
        let (converted, images) = extract_inline_images(&html);
        assert!(images.is_empty());
        assert!(converted.contains(broken), "无法解码的原样保留");
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "sig",
            body_text: "text",
            body_html: Some(&html),
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[],
        });
        assert!(
            !raw.contains("multipart/related"),
            "无有效内联图不建 related 结构"
        );
        let parsed = MessageParser::default().parse(raw.as_bytes()).unwrap();
        assert!(parsed.body_html(0).unwrap().contains(broken));
    }

    #[test]
    fn inline_images_nest_inside_mixed_with_regular_attachments() {
        let png = vec![9u8; 16];
        let encoded = BASE64.encode(&png);
        let html = format!("<p><img src=\"data:image/jpeg;base64,{encoded}\"></p>");
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "mixed",
            body_text: "text",
            body_html: Some(&html),
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[AttachmentPayload {
                name: "报告.pdf".into(),
                media_type: "application/pdf".into(),
                content: b"pdf".to_vec(),
            }],
        });
        let mixed_at = raw.find("multipart/mixed").unwrap();
        let alternative_at = raw.find("multipart/alternative").unwrap();
        let related_at = raw.find("multipart/related").unwrap();
        assert!(
            mixed_at < alternative_at && alternative_at < related_at,
            "嵌套顺序 mixed > alternative > related"
        );
        assert!(raw.contains("Content-Disposition: attachment"));
        assert!(raw.contains("Content-Disposition: inline; filename=\"inline.jpg\""));
    }

    #[test]
    fn header_injection_is_neutralized() {
        let raw = assemble(&MimeInput {
            from_name: "evil\r\nBcc: x@example.test",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["a@example.test".into()],
                ..Default::default()
            },
            subject: "subject\r\nBcc: y@example.test",
            body_text: "body",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[],
        });
        assert!(!raw.contains("Bcc:"), "CR/LF 必须被中和，头部不得注入");
        assert!(raw.matches("Message-ID:").count() == 1);
    }

    #[test]
    fn recipients_are_parsed_strictly() {
        assert_eq!(
            parse_recipients("a@example.test, b@example.test").unwrap(),
            vec!["a@example.test".to_string(), "b@example.test".to_string()]
        );
        assert!(parse_recipients("no-at-mark").is_err());
        assert!(parse_recipients("a@b@example.test").is_err());
        assert!(parse_recipients("a @example.test").is_err());
        assert!(parse_recipients("a@localhost").is_err());
        assert!(parse_recipients("").is_err());
        assert!(parse_recipients("a<b>@example.test").is_err());
        assert!(parse_recipients("a@example.test;evil").is_err());
    }

    #[test]
    fn cc_and_bcc_use_the_full_envelope_without_leaking_bcc_headers() {
        let conn = db();
        conn.execute("UPDATE drafts SET cc = 'copy@example.test', bcc = 'private@example.test, COPY@example.test' WHERE id = 'draft-1'", []).unwrap();
        enqueue(&conn, "draft-1", 3).unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        let envelope = job.recipients.envelope();
        assert!(envelope.contains(&"copy@example.test".into()));
        assert!(envelope.contains(&"private@example.test".into()));
        assert_eq!(
            envelope
                .iter()
                .filter(|address| address.eq_ignore_ascii_case("copy@example.test"))
                .count(),
            1
        );
        let raw = assemble(&MimeInput {
            from_name: &job.from_name,
            from_email: &job.from_email,
            recipients: &job.recipients,
            subject: "Recipients",
            body_text: "Body",
            body_html: None,
            message_id: &job.message_id,
            reply: None,
            attachments: &[],
        });
        assert!(raw.contains("\r\nCc: copy@example.test\r\n"));
        assert!(!raw.contains("private@example.test"));
        assert!(!raw.contains("Bcc:"));
        let parsed = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .unwrap();
        assert_eq!(parsed.body_text(0).unwrap(), "Body");
    }

    #[test]
    fn reply_drafts_carry_in_reply_to_and_references_headers() {
        let conn = db();
        conn.execute(
            "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, received_at, header_message_id, header_references)
             VALUES ('orig', 'real', 'r1', 't', '原始', '发件人', 'peer@example.test', '2026-11-14T00:00:00Z', 'orig-id@peer.test', '<a@chain.test> <b@chain.test>')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE drafts SET reply_to_id = 'orig' WHERE id = 'draft-1'",
            [],
        )
        .unwrap();
        enqueue(&conn, "draft-1", 3).unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        let reply = job.reply.as_ref().unwrap();
        assert_eq!(reply.in_reply_to, "<orig-id@peer.test>");
        assert_eq!(
            reply.references.as_deref(),
            Some("<a@chain.test> <b@chain.test> <orig-id@peer.test>")
        );
        let raw = assemble(&MimeInput {
            from_name: &job.from_name,
            from_email: &job.from_email,
            recipients: &job.recipients,
            subject: "Re: 原始",
            body_text: "Body",
            body_html: None,
            message_id: &job.message_id,
            reply: job.reply.as_ref(),
            attachments: &[],
        });
        assert!(raw.contains("\r\nIn-Reply-To: <orig-id@peer.test>\r\n"));
        // 短链单行输出；折行仅在超过 900 字节时发生。
        assert!(raw.contains("References: <a@chain.test> <b@chain.test> <orig-id@peer.test>\r\n"));
        // 头部顺序：To/Cc 之后、Subject 之前。
        let to_pos = raw.find("\r\nTo: ").unwrap();
        let references_pos = raw.find("References: ").unwrap();
        let subject_pos = raw.find("\r\nSubject: ").unwrap();
        assert!(to_pos < references_pos && references_pos < subject_pos);
    }

    #[test]
    fn reply_headers_degrade_without_original_or_message_id() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        // snapshot_job 会把行标记 sending；重置回 queued 以便复测后续场景。
        fn requeue(conn: &Connection) {
            conn.execute("UPDATE outbox SET state = 'queued'", [])
                .unwrap();
        }
        // 原邮件已被删除：不阻塞发送，也不写关联头。
        conn.execute(
            "UPDATE drafts SET reply_to_id = 'missing' WHERE id = 'draft-1'",
            [],
        )
        .unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        assert!(job.reply.is_none());
        // 原邮件没有 Message-ID：同样诚实降级。
        requeue(&conn);
        conn.execute(
            "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, sender_name, sender_email, received_at)
             VALUES ('orig-none', 'real', 'r2', 't', '原始', '发件人', 'peer@example.test', '2026-11-14T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE drafts SET reply_to_id = 'orig-none' WHERE id = 'draft-1'",
            [],
        )
        .unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        assert!(job.reply.is_none());
        // Message-ID 缺尖括号时补齐；CR/LF 注入被中和（单行内空格）。
        requeue(&conn);
        conn.execute(
            "UPDATE messages SET header_message_id = 'evil\r\nBcc: injected@example.test' WHERE id = 'orig-none'",
            [],
        )
        .unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        let reply = job.reply.as_ref().unwrap();
        assert_eq!(reply.in_reply_to, "<evil  Bcc: injected@example.test>");
        // 原邮件没有 References 时，会话链只含原 Message-ID 本身。
        assert_eq!(
            reply.references.as_deref(),
            Some("<evil  Bcc: injected@example.test>")
        );
        let raw = assemble(&MimeInput {
            from_name: &job.from_name,
            from_email: &job.from_email,
            recipients: &job.recipients,
            subject: "Re: 原始",
            body_text: "Body",
            body_html: None,
            message_id: &job.message_id,
            reply: job.reply.as_ref(),
            attachments: &[],
        });
        let in_reply_to_pos = raw.find("In-Reply-To: ").unwrap();
        assert!(
            !raw[in_reply_to_pos..].contains("Bcc:\r\n"),
            "CR/LF 必须被中和"
        );
        assert_eq!(raw.matches("In-Reply-To:").count(), 1);
    }

    #[test]
    fn bcc_only_is_supported_and_bad_cc_or_bcc_blocks_enqueue() {
        let conn = db();
        for field in ["cc", "bcc"] {
            conn.execute(&format!("UPDATE drafts SET {field} = 'bad-address'"), [])
                .unwrap();
            assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
            conn.execute(&format!("UPDATE drafts SET {field} = ''"), [])
                .unwrap();
        }
        conn.execute(
            "UPDATE drafts SET recipients_json = '[]', bcc = 'hidden@example.test'",
            [],
        )
        .unwrap();
        let recipients = draft_recipients(&conn, "draft-1").unwrap();
        assert_eq!(recipients.envelope(), vec!["hidden@example.test"]);
        let raw = assemble(&MimeInput {
            from_name: "Sender",
            from_email: "me@example.test",
            recipients: &recipients,
            subject: "",
            body_text: "正文",
            body_html: None,
            message_id: "<id@example.test>",
            reply: None,
            attachments: &[],
        });
        assert!(raw.contains("To: undisclosed-recipients:;\r\n"));
        assert!(!raw.contains("hidden@example.test"));
        enqueue(&conn, "draft-1", 3).unwrap();
    }

    #[test]
    fn user_exits_move_unknown_to_failed_or_back_to_queued() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        // 模拟一次真实投递尝试后的 unknown（attempts 已 +1）。
        conn.execute("UPDATE outbox SET state = 'unknown', attempts = 1", [])
            .unwrap();
        let entry = resolve(&conn, &list(&conn).unwrap()[0].id, "keep-local").unwrap();
        assert_eq!(entry.state, "failed");
        assert_eq!(entry.resolved_by.as_deref(), Some("keep-local"));
        assert!(entry.last_error.as_deref().unwrap().contains("仅保留本机"));
        assert_eq!(retry(&conn, &entry.id).unwrap_err().code, "conflict");
        assert_eq!(
            return_to_draft(&conn, &entry.id).unwrap_err().code,
            "conflict"
        );
        conn.execute(
            "UPDATE outbox SET state = 'unknown', resolved_by = NULL",
            [],
        )
        .unwrap();
        let entry = resolve(&conn, &entry.id, "retry").unwrap();
        assert_eq!(entry.state, "queued");
        assert_eq!(entry.attempts, 1, "重试不重置已尝试次数");
        assert_eq!(entry.last_error, None);
    }

    #[test]
    fn failed_tasks_retry_with_the_same_id_or_return_an_unchanged_draft() {
        let mut conn = db();
        let queued = enqueue(&conn, "draft-1", 3).unwrap();
        for state in ["queued", "sending", "unknown", "sent"] {
            conn.execute("UPDATE outbox SET state = ?1", [state])
                .unwrap();
            assert!(return_to_draft(&conn, &queued.id).is_err());
            assert!(retry(&conn, &queued.id).is_err());
        }
        conn.execute("UPDATE outbox SET state = 'failed', attempts = 2", [])
            .unwrap();
        let retried = retry(&conn, &queued.id).unwrap();
        assert_eq!(retried.message_id, queued.message_id);
        assert_eq!(retried.attempts, 2);
        conn.execute("UPDATE outbox SET state = 'failed'", [])
            .unwrap();
        let hash = crate::mail_recovery::hash(&[1, 2, 3]);
        conn.execute(
            "INSERT INTO draft_attachment_blobs VALUES ('draft-1', 'att-1', ?1, X'010203')",
            [&hash],
        )
        .unwrap();
        let attachments = serde_json::json!([{ "id": "att-1", "name": "保留.txt", "size": 3, "mediaType": "text/plain", "sha256": hash }]);
        conn.execute("UPDATE drafts SET attachments_json = ?1, cc = 'copy@example.test', bcc = 'private@example.test', body_html = '<p>正文</p>' WHERE id = 'draft-1'", [attachments.to_string()]).unwrap();
        let before = drafts::list(&conn).unwrap()[0].draft.clone().unwrap();
        return_to_draft(&conn, &queued.id).unwrap();
        assert!(return_to_draft(&conn, &queued.id).is_err());
        assert!(list(&conn).unwrap().is_empty());
        assert_eq!(
            drafts::list(&conn).unwrap()[0].draft.as_ref(),
            Some(&before)
        );
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT content FROM draft_attachment_blobs WHERE draft_id = 'draft-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bytes, [1, 2, 3]);
        let saved = drafts::save(
            &mut conn,
            &before,
            "real",
            &crate::compose_attach::ComposeStaging::default(),
        )
        .unwrap();
        assert_eq!(saved.draft.unwrap().revision, 4);
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT content FROM draft_attachment_blobs WHERE draft_id = 'draft-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bytes, [1, 2, 3]);
    }

    #[test]
    fn old_unresolved_tasks_remain_visible_after_more_than_128_successes() {
        let conn = db();
        let old = enqueue(&conn, "draft-1", 3).unwrap();
        conn.execute(
            "UPDATE outbox SET state='unknown', queued_at='2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
        for index in 0..140 {
            conn.execute(
                "INSERT INTO outbox(id,account_id,draft_id,state,idempotency_key,updated_at,queued_at)
              VALUES (?1,'real','draft-1','sent',?1,'2026-11-16T00:00:00Z','2026-11-16T00:00:00Z')",
                [format!("sent-{index:03}")],
            )
            .unwrap();
        }
        let entries = list(&conn).unwrap();
        assert_eq!(entries.len(), 129);
        assert_eq!(entries[0].id, old.id);
        assert_eq!(entries[0].state, "unknown");
    }

    #[test]
    fn cancel_only_removes_queued_entries() {
        let conn = db();
        enqueue(&conn, "draft-1", 3).unwrap();
        let entry = list(&conn).unwrap().remove(0);
        conn.execute("UPDATE outbox SET state = 'sending'", [])
            .unwrap();
        assert_eq!(cancel(&conn, &entry.id).unwrap_err().code, "conflict");
        conn.execute("UPDATE outbox SET state = 'queued'", [])
            .unwrap();
        let cancelled = cancel(&conn, &entry.id).unwrap();
        assert_eq!(cancelled.state, "queued");
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    // ── 撰写附件 MIME 组装 ──────────────────────────────────────

    fn attachment(name: &str, media_type: &str, content: &[u8]) -> AttachmentPayload {
        AttachmentPayload {
            name: name.into(),
            media_type: media_type.into(),
            content: content.to_vec(),
        }
    }

    /// 回读解析结果：text / html 正文与附件 (文件名, 字节) 列表。
    struct ParsedMail {
        text: Option<String>,
        attachments: Vec<(String, Vec<u8>)>,
    }

    /// 用 mail_parser 回读组装结果。
    fn parse_back(raw: &str) -> ParsedMail {
        use mail_parser::MimeHeaders;
        let parsed = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .expect("MIME 必须可解析");
        let attachments = (0..parsed.attachment_count())
            .map(|index| {
                let part = parsed.attachment(index).expect("附件存在");
                (
                    part.attachment_name().unwrap_or_default().to_string(),
                    part.contents().to_vec(),
                )
            })
            .collect();
        let text = parsed.body_text(0).map(|body| body.into_owned());
        ParsedMail { text, attachments }
    }

    #[test]
    fn text_body_with_attachments_becomes_multipart_mixed_and_roundtrips() {
        let content: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "带附件",
            body_text: "正文内容",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[
                attachment("季度报告.pdf", "application/pdf", &content),
                attachment("notes.txt", "text/plain", b"plain notes"),
            ],
        });
        assert!(raw.contains("Content-Type: multipart/mixed; boundary=\"=_auumail_"));
        assert!(!raw.contains("multipart/alternative"));
        assert!(raw.contains("Content-Disposition: attachment"));
        assert!(raw.contains("Content-Transfer-Encoding: base64"));
        // 组装结果按 CRLF 折行后每行内不得再含 CR/LF（头部注入防护）。
        for line in raw.split("\r\n") {
            assert!(!line.contains('\r') && !line.contains('\n'));
        }
        let parsed = parse_back(&raw);
        assert_eq!(parsed.text.as_deref(), Some("正文内容"));
        // 纯文本 + 附件：不存在真正的 HTML part（mail_parser 会为纯文本合成 html，故查头）。
        assert!(!raw.contains("Content-Type: text/html"));
        assert_eq!(parsed.attachments.len(), 2);
        assert_eq!(parsed.attachments[0].0, "季度报告.pdf");
        assert_eq!(parsed.attachments[0].1, content, "字节逐位一致");
        assert_eq!(parsed.attachments[1].0, "notes.txt");
        assert_eq!(parsed.attachments[1].1, b"plain notes");
    }

    #[test]
    fn html_body_with_attachments_keeps_alternative_inside_mixed() {
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "rich + files",
            body_text: "plain",
            body_html: Some("<p>rich <b>text</b></p>"),
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[attachment(
                "data.bin",
                "application/octet-stream",
                &[1, 2, 3],
            )],
        });
        assert!(raw.contains("Content-Type: multipart/mixed"));
        assert!(raw.contains("Content-Type: multipart/alternative"));
        let parsed = parse_back(&raw);
        assert_eq!(parsed.text.as_deref(), Some("plain"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].1, vec![1, 2, 3]);
    }

    #[test]
    fn attachment_names_are_neutralized_and_fallback_type_applies() {
        let raw = assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "names",
            body_text: "body",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[
                attachment("报告 最终版.txt", "text/plain", b"a"),
                attachment("bad\r\nname.bin", "not a valid type!", b"b"),
            ],
        });
        let parsed = parse_back(&raw);
        assert_eq!(parsed.attachments[0].0, "报告 最终版.txt");
        assert_eq!(parsed.attachments[1].0, "bad  name.bin");
        assert_eq!(parsed.attachments[1].1, b"b");
        // 非法 media_type 被中和为合法 token；头部无裸 CR/LF。
        assert!(!raw.contains("not a valid type!"));
    }

    #[test]
    fn ascii_attachment_filenames_keep_plain_quoted_params() {
        // ASCII 安全名与既有格式逐字节一致：不引入 RFC2231，也不改变引号形式。
        let headers = attachment_headers("notes.txt", "text/plain", b"x");
        assert!(
            headers.starts_with("Content-Type: text/plain; name=\"notes.txt\"\r\n"),
            "Content-Type 参数形式改变: {headers}"
        );
        assert!(
            headers.contains("Content-Disposition: attachment; filename=\"notes.txt\"\r\n"),
            "Content-Disposition 参数形式改变: {headers}"
        );
        assert!(!headers.contains("*="));
    }

    #[test]
    fn cjk_attachment_filenames_use_rfc2231_disposition_and_quoted_2047_type() {
        // 修复回归：非 ASCII 名此前是裸 RFC2047 参数值，Python email policy.default 丢失参数。
        let name = "中文附件.txt";
        let headers = attachment_headers(name, "text/plain", b"x");
        let expected_pct = percent_encode_value(name);
        // Content-Disposition：RFC 2231 扩展参数（行限内用单段；多段在 mail-parser
        // 0.10.2 接收侧会按段独立解码损坏文件名，见 rfc2231_parameters 文档与验收探针）。
        assert!(headers.contains(&format!("filename*=utf-8''{expected_pct}")));
        assert!(!headers.contains("filename*0*="));
        // Content-Type：引号内 RFC2047（Gmail 等宽松客户端兼容路径）。
        assert!(headers.contains("name=\"=?utf-8?B?"));
        // 同一头内不得再同时出现裸 filename 与 filename*（mail-parser 续行合并会误拼接），
        // 也不得出现裸（未加引号）encoded-word 参数值。
        assert!(!headers.contains("filename=\""));
        assert!(!headers.contains(" filename=?utf-8"));
        // Content-Type 不得使用 RFC2231 参数（按行核对：filename*= 是 name*= 的超串，不能整体 includes）。
        for line in headers.split(
            "
",
        ) {
            if line.starts_with("Content-Type:") {
                assert!(!line.contains("*="), "Content-Type 参数形式改变: {line}");
            }
        }
        let parsed = parse_back(&assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "params",
            body_text: "body",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[attachment(name, "text/plain", b"cjk-bytes")],
        }));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].0, name, "mail_parser 须取回中文原名");
        assert_eq!(parsed.attachments[0].1, b"cjk-bytes");
    }

    #[test]
    fn long_cjk_filenames_stay_on_one_2231_line_within_limits() {
        // >75 字节触发 RFC2047 折行；percent 值在 800 字符预算内保持单段 2231。
        let name = "这是一个相当长的中文文件名用来测试编码续行行为是否正确保存原名.txt";
        assert!(name.len() > 75);
        let headers = attachment_headers(name, "application/octet-stream", b"long");
        assert!(headers.contains("name=\"=?utf-8?B?"));
        assert!(headers.contains(&format!("filename*=utf-8''{}", percent_encode_value(name))));
        assert!(!headers.contains("filename*0*="));
        assert!(!headers.contains("filename=\""));
        // 折行后每一行都不得超过 RFC 5322 单行 998 字节上限。
        for line in headers.split("\r\n") {
            assert!(line.len() <= 998, "单行超限 {} 字节: {}", line.len(), line);
        }
        let parsed = parse_back(&assemble(&MimeInput {
            from_name: "me",
            from_email: "me@example.test",
            recipients: &Recipients {
                to: vec!["friend@example.test".into()],
                ..Default::default()
            },
            subject: "long",
            body_text: "body",
            body_html: None,
            message_id: "<abc@auumail.local>",
            reply: None,
            attachments: &[attachment(name, "application/octet-stream", b"long-bytes")],
        }));
        assert_eq!(parsed.attachments[0].0, name, "折行 + 分段后须逐字取回原名");
        assert_eq!(parsed.attachments[0].1, b"long-bytes");
    }

    #[test]
    fn extreme_filenames_fall_back_to_2231_continuations() {
        // 编码值超过单行预算（约 800 字符）才回退多段续行。多段在 mail-parser 0.10.2
        // 接收侧会损坏文件名（上游限制，probe_2231_single_vs_multi 实测），因此这里只
        // 断言参数结构与行限，不做接收端原名往返断言。
        let name = "长".repeat(300);
        let headers = attachment_headers(&name, "application/octet-stream", b"x");
        assert!(headers.contains("filename*0*=utf-8''"));
        assert!(headers.contains("filename*1*="));
        for line in headers.split(
            "
",
        ) {
            assert!(line.len() <= 998, "单行超限 {} 字节: {}", line.len(), line);
        }
    }

    #[test]
    fn empty_or_injected_filenames_fall_back_safely() {
        let headers = attachment_headers("   ", "text/plain", b"x");
        assert!(headers.contains("name=\"attachment.bin\""));
        assert!(headers.contains("filename=\"attachment.bin\""));
        // 含引号 / 分号的注入形名字走编码路径，不能破坏参数结构。
        let headers = attachment_headers("evil\";.txt", "text/plain", b"x");
        assert!(headers.contains("filename*=utf-8''evil%22%3B.txt"));
        assert!(!headers.contains("evil\";"));
    }

    #[test]
    fn enqueue_rejects_attachment_metadata_over_limits() {
        let conn = db();
        let over: Vec<String> = (0..21)
            .map(|index| {
                format!(
                    r#"{{"id":"att-{index:04}","name":"{index}.bin","size":1024,"mediaType":"application/octet-stream","sha256":"{}"}}"#,
                    "a".repeat(64)
                )
            })
            .collect();
        conn.execute(
            "UPDATE drafts SET attachments_json = ?1 WHERE id = 'draft-1'",
            [format!("[{}]", over.join(","))],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "invalid");
        // 损坏的 JSON 也不能登记。
        conn.execute(
            "UPDATE drafts SET attachments_json = '{broken' WHERE id = 'draft-1'",
            [],
        )
        .unwrap();
        assert_eq!(enqueue(&conn, "draft-1", 3).unwrap_err().code, "storage");
    }

    #[test]
    fn snapshot_loads_attachment_payloads_and_rejects_missing_or_mismatched_content() {
        let conn = db();
        let content = b"payload bytes".to_vec();
        let sha = format!("{:x}", Sha256::digest(&content));
        let metadata = format!(
            r#"[{{"id":"att-1","name":"file.txt","size":{},"mediaType":"text/plain","sha256":"{sha}"}}]"#,
            content.len()
        );
        conn.execute(
            "UPDATE drafts SET attachments_json = ?1 WHERE id = 'draft-1'",
            [&metadata],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO draft_attachment_blobs (draft_id, attachment_id, sha256, content) VALUES ('draft-1', 'att-1', ?1, ?2)",
            params![sha, content],
        )
        .unwrap();
        enqueue(&conn, "draft-1", 3).unwrap();
        let job = snapshot_job(&conn, "real").unwrap().unwrap();
        assert_eq!(job.attachments.len(), 1);
        assert_eq!(job.attachments[0].name, "file.txt");
        assert_eq!(job.attachments[0].content, content);
        // 内容缺失：快照失败（发送按 failed 处理，不发送不完整邮件）。
        conn.execute("DELETE FROM outbox", []).unwrap();
        conn.execute("DELETE FROM draft_attachment_blobs", [])
            .unwrap();
        enqueue(&conn, "draft-1", 3).unwrap();
        let error = match snapshot_job(&conn, "real") {
            Err(error) => error,
            Ok(_) => panic!("附件缺失时快照必须失败"),
        };
        assert!(error.message.contains("内容缺失"));
    }
}
