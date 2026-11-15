//! 从已验证的完整原文缓存按需提取并消毒 HTML 正文。邮件 HTML 是不可信内容：
//! Rust 层 ammonia 消毒 + 唯一源沙箱 iframe + 仅放行桥接脚本的 CSP，三层防御缺一不可。
//! 渲染返回完整 srcdoc 文档，前端不得对正文 HTML 追加任何转义或直插 DOM。
use crate::{error::AppError, mail_cache};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::PathBuf, sync::OnceLock};

const MAX_HTML: usize = 2 * 1024 * 1024;
const MAX_INLINE_IMAGE: usize = 2 * 1024 * 1024;
const MAX_INLINE_TOTAL: usize = 8 * 1024 * 1024;

/// 仅当 raw_cache_key 与 content_hash 一致（可审计的完整原文）时才允许渲染。
fn load_cache_key(
    conn: &rusqlite::Connection,
    account: &str,
    id: &str,
) -> Result<(PathBuf, String), AppError> {
    let (key, digest): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT raw_cache_key,content_hash FROM messages WHERE account_id=?1 AND id=?2",
            params![account, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::invalid("邮件不存在或不属于此账号"))?;
    let key = key
        .filter(|key| Some(key) == digest.as_ref())
        .ok_or_else(|| AppError::invalid("这封邮件没有已验证的完整原文，无法显示富文本"))?;
    Ok((mail_cache::root(conn)?, key))
}

/// 内联图片（multipart/related 的 cid: 引用）转 data: URI；超大图片跳过，不猜正文。
fn inline_data_uris(parsed: &mail_parser::Message<'_>) -> Result<Vec<(String, String)>, AppError> {
    let mut map = Vec::new();
    let mut total = 0usize;
    for part in parsed.attachments() {
        let Some(raw_id) = part.content_id() else {
            continue;
        };
        if !part
            .content_type()
            .is_some_and(|ct| ct.c_type.eq_ignore_ascii_case("image"))
        {
            continue;
        }
        let contents = part.contents();
        if contents.len() > MAX_INLINE_IMAGE || total + contents.len() > MAX_INLINE_TOTAL {
            continue;
        }
        let mime = match part.content_type().and_then(|ct| ct.c_subtype.as_deref()) {
            Some(sub) => format!("image/{}", sub.to_ascii_lowercase()),
            None => "image/octet-stream".into(),
        };
        let id = raw_id.trim_matches(|c| c == '<' || c == '>');
        map.push((
            id.to_string(),
            format!("data:{mime};base64,{}", BASE64.encode(contents)),
        ));
        total += contents.len();
    }
    Ok(map)
}

/// 撰写草稿的 HTML 消毒（compose 专用，比收件消毒更严格）：
/// 保留格式标签与 style 属性；剥 <style> 块 / img / data: URI / 相对地址 / 脚本与事件处理器。
/// 撰写内容会发送给别人，消毒发生在草稿写入时而非发送时。
pub(crate) fn sanitize_compose(html: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder.tags(HashSet::from([
        "p", "div", "span", "br", "hr", "b", "strong", "i", "em", "u", "s", "strike", "a", "h1",
        "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "code", "table",
        "thead", "tbody", "tr", "td", "th", "font", "center", "small", "sub", "sup",
    ]));
    builder.generic_attributes(HashSet::from(["style"]));
    builder.tag_attributes(std::collections::HashMap::from([
        ("a", HashSet::from(["href", "title"])),
        ("table", HashSet::from(["width", "border", "cellpadding", "cellspacing", "align", "bgcolor"])),
        ("td", HashSet::from(["colspan", "rowspan", "align", "valign", "width", "height", "bgcolor"])),
        ("th", HashSet::from(["colspan", "rowspan", "align", "valign", "width", "height", "bgcolor"])),
        ("font", HashSet::from(["color", "face", "size"])),
        ("hr", HashSet::from(["size", "width", "align"])),
    ]));
    builder.url_schemes(HashSet::from(["http", "https", "mailto"]));
    builder.url_relative(ammonia::UrlRelative::Deny);
    builder.link_rel(Some("noopener noreferrer"));
    builder.clean(html).to_string()
}

fn sanitize(html: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder.tags(HashSet::from([
        "p",
        "div",
        "span",
        "br",
        "hr",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "caption",
        "colgroup",
        "col",
        "tr",
        "td",
        "th",
        "ul",
        "ol",
        "li",
        "dl",
        "dt",
        "dd",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "s",
        "strike",
        "a",
        "img",
        "blockquote",
        "pre",
        "code",
        "font",
        "center",
        "small",
        "sub",
        "sup",
        "style",
        "figure",
        "figcaption",
        "main",
        "article",
        "section",
        "header",
        "footer",
    ]));
    // 默认 clean_content_tags 含 style，会剥掉邮件排版赖以生存的 <style> 块；这里只清 script。
    builder.clean_content_tags(HashSet::from(["script"]));
    builder.generic_attributes(HashSet::from(["style"]));
    builder.tag_attributes(std::collections::HashMap::from([
        ("a", HashSet::from(["href", "name", "target"])),
        (
            "img",
            HashSet::from(["src", "alt", "width", "height", "align", "border"]),
        ),
        (
            "table",
            HashSet::from([
                "width",
                "border",
                "cellpadding",
                "cellspacing",
                "align",
                "bgcolor",
            ]),
        ),
        (
            "td",
            HashSet::from([
                "colspan", "rowspan", "align", "valign", "width", "height", "bgcolor",
            ]),
        ),
        (
            "th",
            HashSet::from([
                "colspan", "rowspan", "align", "valign", "width", "height", "bgcolor",
            ]),
        ),
        ("font", HashSet::from(["color", "face", "size"])),
        ("hr", HashSet::from(["size", "width", "align"])),
    ]));
    builder.url_schemes(HashSet::from(["http", "https", "mailto", "data"]));
    builder.url_relative(ammonia::UrlRelative::Deny);
    builder.link_rel(Some("noopener noreferrer"));
    builder.clean(html).to_string()
}

fn bridge_hash() -> &'static String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| BASE64.encode(Sha256::digest(BRIDGE_SCRIPT.as_bytes())))
}

/// CSP 哈希只放行桥接脚本：消毒后漏网的邮件脚本无法匹配哈希，一律被浏览器拒绝。
fn csp(allow_remote_images: bool) -> String {
    let images = if allow_remote_images {
        "https: data:"
    } else {
        "'none'"
    };
    format!(
        "default-src 'none'; script-src 'sha256-{}'; style-src 'unsafe-inline' https: data:; img-src {}; font-src https: data:; frame-src 'none'; form-action 'none'; base-uri 'none'",
        bridge_hash(),
        images
    )
}

const WRAPPER_CSS_FONT: &str = "html,body{margin:0;padding:12px;font-family:system-ui,-apple-system,'Segoe UI','Microsoft YaHei',sans-serif;font-size:13px;line-height:1.65;overflow-wrap:break-word}img{max-width:100%;height:auto}table{max-width:100%}";
const WRAPPER_CSS_LIGHT: &str =
    "html{color-scheme:light}body{background:#ffffff;color:#1c2430}a{color:#0a7ea4}";
const WRAPPER_CSS_DARK: &str =
    "html{color-scheme:dark}body{background:#1b2129;color:#d9e1ea}a{color:#7ab8e8}blockquote{border-left:2px solid #465060;padding-left:12px;margin-left:8px}";

/// 包装 CSS 按当前主题模式生成：深色主题下阅读区不再固定白底。
/// 发件人自带显式颜色的邮件仍按原样渲染（与 Gmail / Outlook 行为一致）。
fn wrapper_css(dark: bool) -> &'static str {
    if dark { WRAPPER_CSS_DARK } else { WRAPPER_CSS_LIGHT }
}

const BRIDGE_SCRIPT: &str = "(function () { 'use strict'; function postHeight() { try { parent.postMessage({ type: 'auumail-frame-height', height: document.documentElement.scrollHeight + 24 }, '*'); } catch (error) {} } document.addEventListener('click', function (event) { var node = event.target; while (node && node.nodeType === 1 && node.tagName !== 'A') node = node.parentNode; if (!node || node.nodeType !== 1) return; event.preventDefault(); parent.postMessage({ type: 'auumail-mail-link', href: node.getAttribute('href') || '' }, '*'); }, true); window.addEventListener('load', postHeight); if (window.ResizeObserver) { new ResizeObserver(postHeight).observe(document.documentElement); } postHeight(); })();";

pub(crate) fn render(
    conn: &rusqlite::Connection,
    account: &str,
    id: &str,
    allow_remote_images: bool,
    dark: bool,
) -> Result<String, AppError> {
    let (root, key) = load_cache_key(conn, account, id)?;
    let raw = mail_cache::read(&root, &key)?;
    let parsed = MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| AppError::invalid("邮件 MIME 无效，无法显示富文本"))?;
    let html = parsed
        .body_html(0)
        .filter(|html| html.len() <= MAX_HTML)
        .ok_or_else(|| AppError::invalid("这封邮件没有可显示的 HTML 正文"))?;
    let inline = inline_data_uris(&parsed)?;
    let mut prepared = html.into_owned();
    for (id, uri) in inline {
        prepared = prepared.replace(&format!("cid:{id}"), &uri);
    }
    Ok(srcdoc(&sanitize(&prepared), allow_remote_images, dark))
}

fn srcdoc(body: &str, allow_remote_images: bool, dark: bool) -> String {
    let mut doc = String::with_capacity(body.len() + 2048);
    doc.push_str("<!DOCTYPE html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"");
    doc.push_str(&csp(allow_remote_images));
    doc.push_str("\"><style>");
    doc.push_str(WRAPPER_CSS_FONT);
    doc.push_str(wrapper_css(dark));
    doc.push_str("</style></head><body>");
    doc.push_str(body);
    doc.push_str("<script>");
    doc.push_str(BRIDGE_SCRIPT);
    doc.push_str("</script></body></html>");
    doc
}

/// 回复 / 转发引用：消毒后的正文 HTML 片段（无 srcdoc 包装，进编辑器 contentEditable）。
/// 用 compose 严格消毒（剥 <style> 块 / img / data: URI），避免片段影响编辑器全局样式。
pub(crate) fn render_fragment(conn: &Connection, account: &str, id: &str) -> Result<String, AppError> {
    let (root, key) = load_cache_key(conn, account, id)?;
    let raw = mail_cache::read(&root, &key)?;
    let parsed = MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| AppError::invalid("邮件 MIME 无效，无法引用原文"))?;
    let html = parsed
        .body_html(0)
        .filter(|html| html.len() <= MAX_HTML)
        .ok_or_else(|| AppError::invalid("这封邮件没有可引用的正文"))?;
    let inline = inline_data_uris(&parsed)?;
    let mut prepared = html.into_owned();
    for (cid, uri) in inline {
        prepared = prepared.replace(&format!("cid:{cid}"), &uri);
    }
    Ok(sanitize_compose(&prepared))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RICH_MAIL: &str = "From: sender@example.test\r\nTo: user@example.test\r\nSubject: rich\r\nContent-Type: multipart/related; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n--b2\r\nContent-Type: text/plain\r\n\r\nplain fallback\r\n--b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><style>p.t{color:red}</style><p class=\"t\" style=\"font-weight:bold\">Hi <a href=\"https://example.test/x\">link</a> <a href=\"javascript:alert(1)\">bad</a></p><img src=\"cid:logo1\" alt=\"l\"><img src=\"https://tracker.example/pixel.png\"><script>alert(1)</script><div onmouseover=\"evil()\">x</div><a href=\"/relative\">rel</a></body></html>\r\n--b2--\r\n--b\r\nContent-Type: image/png; name=\"l.png\"\r\nContent-ID: <logo1>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\r\n--b--\r\n";

    #[test]
    fn sanitization_removes_active_content_and_rewrites_cid() {
        let parsed = MessageParser::default()
            .parse(RICH_MAIL.as_bytes())
            .unwrap();
        assert!(parsed.body_html(0).is_some());
        let inline = inline_data_uris(&parsed).unwrap();
        assert_eq!(inline.len(), 1);
        assert!(inline[0].0 == "logo1" && inline[0].1.starts_with("data:image/png;base64,"));
        let mut prepared = parsed.body_html(0).unwrap().into_owned();
        for (id, uri) in &inline {
            prepared = prepared.replace(&format!("cid:{id}"), uri);
        }
        let sanitized = sanitize(&prepared);
        assert!(!sanitized.to_ascii_lowercase().contains("<script"));
        assert!(!sanitized.contains("onmouseover"));
        assert!(!sanitized.contains("javascript:"));
        assert!(sanitized.contains("<style>p.t{color:red}</style>"));
        assert!(sanitized.contains("href=\"https://example.test/x\""));
        assert!(sanitized.contains("rel=\"noopener noreferrer\""));
        assert!(
            !sanitized.contains("href=\"/relative\""),
            "相对地址必须被拒绝"
        );
        assert!(
            sanitized.contains("src=\"data:image/png;base64,"),
            "cid 必须转成 data URI"
        );
        let doc = srcdoc(&sanitized, false, false);
        assert!(doc.contains("img-src 'none'"));
        assert!(doc.contains(&format!("script-src 'sha256-{}'", bridge_hash())));
        assert!(doc.starts_with("<!DOCTYPE html>"));
        let allow = srcdoc(&sanitized, true, false);
        assert!(allow.contains("img-src https: data:"));
    }

    #[test]
    fn wrapper_css_follows_theme_mode() {
        let light = srcdoc("<p>x</p>", false, false);
        assert!(light.contains("color-scheme:light") && light.contains("#ffffff"));
        let dark = srcdoc("<p>x</p>", false, true);
        assert!(dark.contains("color-scheme:dark") && dark.contains("#1b2129"));
        assert!(!dark.contains("#ffffff"));
    }

    #[test]
    fn oversize_inline_images_are_skipped_not_fatal() {
        let parsed = MessageParser::default()
            .parse(RICH_MAIL.as_bytes())
            .unwrap();
        let inline = inline_data_uris(&parsed).unwrap();
        assert!(!inline.is_empty());
        // 超大内联图被跳过后，正文仍可渲染（仅失去该图），不因尺寸上限整体失败。
        let html = parsed.body_html(0).unwrap();
        assert!(!html.is_empty());
    }

    #[test]
    fn bridge_hash_is_deterministic_base64() {
        assert_eq!(bridge_hash().len(), 44);
        assert_eq!(bridge_hash(), bridge_hash());
    }
}
