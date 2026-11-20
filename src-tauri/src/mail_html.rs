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
pub(crate) fn inline_data_uris(
    parsed: &mail_parser::Message<'_>,
) -> Result<Vec<(String, String)>, AppError> {
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
/// 保留格式标签、style 属性与 `data:image/*` 内联图片（签名 / 未来的正文插图走此通道，
/// 发送时由 outbox 转 cid: 内联件）；剥 <style> 块 / 其他 img（远程与 svg）/ 相对地址 /
/// 脚本与事件处理器。`data:` 只对 img src 放行是经 attribute_filter 精确收紧的——ammonia
/// 的 url_schemes 是全局的，否则 data: 也会进入 a href。撰写内容会发送给别人，
/// 消毒发生在草稿写入时而非发送时。
pub(crate) fn sanitize_compose(html: &str) -> String {
    sanitize_compose_inner(html, true)
}

/// 引用管线变体：剥除全部图片——引用不携带原文图片（接收侧引用本就剥图），
/// 也避免原文的远程追踪图进入撰写正文。
pub(crate) fn sanitize_quote_fragment(html: &str) -> String {
    sanitize_compose_inner(html, false)
}

fn sanitize_compose_inner(html: &str, keep_images: bool) -> String {
    let mut builder = ammonia::Builder::default();
    let tags: &[&str] = if keep_images {
        &[
            "p",
            "div",
            "span",
            "br",
            "hr",
            "b",
            "strong",
            "i",
            "em",
            "u",
            "s",
            "strike",
            "a",
            "img",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "ul",
            "ol",
            "li",
            "blockquote",
            "pre",
            "code",
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
            "font",
            "center",
            "small",
            "sub",
            "sup",
        ]
    } else {
        &[
            "p",
            "div",
            "span",
            "br",
            "hr",
            "b",
            "strong",
            "i",
            "em",
            "u",
            "s",
            "strike",
            "a",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "ul",
            "ol",
            "li",
            "blockquote",
            "pre",
            "code",
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
            "font",
            "center",
            "small",
            "sub",
            "sup",
        ]
    };
    builder.tags(HashSet::from_iter(tags.iter().copied()));
    builder.generic_attributes(HashSet::from(["style", "data-auumail-quote"]));
    builder.filter_style_properties(HashSet::from([
        "color",
        "background-color",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "text-decoration",
        "text-align",
        "text-indent",
        "text-transform",
        "line-height",
        "letter-spacing",
        "word-spacing",
        "white-space",
        "vertical-align",
        "display",
        "width",
        "min-width",
        "max-width",
        "height",
        "min-height",
        "max-height",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-width",
        "border-style",
        "border-color",
        "border-radius",
        "border-collapse",
        "border-spacing",
        "table-layout",
        "list-style-type",
        "list-style-position",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
    ]));
    builder.tag_attributes(std::collections::HashMap::from([
        ("a", HashSet::from(["href", "title"])),
        (
            "img",
            HashSet::from(["src", "alt", "width", "height", "align", "border"]),
        ),
        ("blockquote", HashSet::from(["data-auumail-quote"])),
        (
            "tr",
            HashSet::from(["align", "valign", "bgcolor", "height"]),
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
    builder.attribute_filter(|element, attribute, value| {
        // img src 仅放行位图 data:image/*（svg 因收件端普遍拦截面排除），远程 src 剥除
        // （撰写不引用外部资源，离线签名图片不破图）；其余任何属性上的 data:（如 a
        // href）一律剥除。attribute_filter 在 url_schemes 清洗之后执行，是精确收紧点。
        let inline_image = element == "img"
            && attribute == "src"
            && value.starts_with("data:image/")
            && !value.starts_with("data:image/svg");
        if value.starts_with("data:") || (element == "img" && attribute == "src") {
            return inline_image.then(|| value.into());
        }
        Some(value.into())
    });
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

/// 阅读字号由前端传入（可调整字号档位）；固定 13px 常量拆出，字号动态拼进 CSS。
const WRAPPER_CSS_BASE: &str = "html,body{margin:0;padding:12px;font-family:system-ui,-apple-system,'Segoe UI','Microsoft YaHei',sans-serif;line-height:1.65;overflow-wrap:break-word}img{max-width:100%;height:auto}table{max-width:100%}";
const DEFAULT_FONT_PX: u32 = 13;

/// 阅读正文字号：白名单外的值回落 13px（防 CSS 注入与异常值）。
fn sanitize_font_px(font_px: Option<u32>) -> u32 {
    match font_px {
        Some(value @ 11..=24) => value,
        _ => DEFAULT_FONT_PX,
    }
}

fn wrapper_font_css(font_px: u32) -> String {
    format!("{WRAPPER_CSS_BASE}html{{font-size:{font_px}px}}body{{font-size:{font_px}px}}")
}
const WRAPPER_CSS_LIGHT: &str =
    "html{color-scheme:light}body{background:#ffffff;color:#1c2430}a{color:#0a7ea4}";
const WRAPPER_CSS_DARK: &str =
    "html{color-scheme:dark}body{background:#1b2129;color:#d9e1ea}a{color:#7ab8e8}blockquote{border-left:2px solid #465060;padding-left:12px;margin-left:8px}";

/// 包装 CSS 按当前主题模式生成：深色主题下阅读区不再固定白底。
/// 发件人自带显式颜色的邮件仍按原样渲染（与 Gmail / Outlook 行为一致）。
fn wrapper_css(dark: bool) -> &'static str {
    if dark {
        WRAPPER_CSS_DARK
    } else {
        WRAPPER_CSS_LIGHT
    }
}

const BRIDGE_SCRIPT: &str = "(function () { 'use strict'; function postHeight() { try { parent.postMessage({ type: 'auumail-frame-height', height: document.documentElement.scrollHeight + 24 }, '*'); } catch (error) {} } document.addEventListener('click', function (event) { var node = event.target; while (node && node.nodeType === 1 && node.tagName !== 'A') node = node.parentNode; if (!node || node.nodeType !== 1) return; event.preventDefault(); parent.postMessage({ type: 'auumail-mail-link', href: node.getAttribute('href') || '' }, '*'); }, true); window.addEventListener('load', postHeight); if (window.ResizeObserver) { new ResizeObserver(postHeight).observe(document.documentElement); } postHeight(); })();";

pub(crate) fn render(
    conn: &rusqlite::Connection,
    account: &str,
    id: &str,
    allow_remote_images: bool,
    dark: bool,
    font_px: Option<u32>,
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
    Ok(srcdoc(
        &sanitize(&prepared),
        allow_remote_images,
        dark,
        font_px,
    ))
}

fn srcdoc(body: &str, allow_remote_images: bool, dark: bool, font_px: Option<u32>) -> String {
    let mut doc = String::with_capacity(body.len() + 2048);
    doc.push_str("<!DOCTYPE html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"");
    doc.push_str(&csp(allow_remote_images));
    doc.push_str("\"><style>");
    doc.push_str(&wrapper_font_css(sanitize_font_px(font_px)));
    doc.push_str(wrapper_css(dark));
    doc.push_str("</style></head><body>");
    doc.push_str(body);
    doc.push_str("<script>");
    doc.push_str(BRIDGE_SCRIPT);
    doc.push_str("</script></body></html>");
    doc
}

/// Short DB snapshot; cache I/O, MIME parsing and CSS inlining run after releasing the lock.
pub(crate) fn fragment_source(
    conn: &Connection,
    account: &str,
    id: &str,
) -> Result<(PathBuf, String), AppError> {
    load_cache_key(conn, account, id)
}

/// Preserve newsletter layout locally, then sanitize before it enters the app DOM.
fn prepare_quote(html: &str) -> Result<String, AppError> {
    const MAX_QUOTE: usize = 200_000;
    if html.len() > MAX_HTML {
        return Err(AppError::invalid("原邮件排版过大，改用纯文本引用"));
    }
    let document = scraper::Html::parse_document(html);
    let elements = scraper::Selector::parse("*").expect("static selector");
    let styles = scraper::Selector::parse("style").expect("static selector");
    if document.select(&elements).take(4001).count() > 4000
        || document
            .select(&styles)
            .map(|style| style.inner_html().len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err(AppError::invalid("原邮件排版过于复杂，改用纯文本引用"));
    }
    struct LimitedOutput(Vec<u8>);
    impl std::io::Write for LimitedOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + bytes.len() > MAX_QUOTE {
                return Err(std::io::Error::other("quote limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // No HTTP/file features; explicitly disable all external stylesheet resolution as well.
    let inliner = css_inline::CSSInliner::options()
        .load_remote_stylesheets(false)
        .build();
    let mut output = LimitedOutput(Vec::new());
    inliner
        .inline_to(html, &mut output)
        .map_err(|_| AppError::invalid("原邮件样式无法完整转换，改用纯文本引用"))?;
    let inlined = String::from_utf8(output.0).map_err(|_| AppError::invalid("原邮件编码无效"))?;
    let document = scraper::Html::parse_document(&inlined);
    let body_selector = scraper::Selector::parse("body").expect("static selector");
    let body = document
        .select(&body_selector)
        .next()
        .ok_or_else(|| AppError::invalid("原邮件正文无效"))?;
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    };
    // A received HTML document uses a light canvas unless it explicitly styles its body.
    // Carry body defaults into the fragment instead of inheriting the editor's dark text color.
    let background = body
        .value()
        .attr("bgcolor")
        .filter(|value| value.chars().all(|c| c.is_ascii_alphanumeric() || c == '#'))
        .unwrap_or("#ffffff");
    let foreground = body
        .value()
        .attr("text")
        .filter(|value| value.chars().all(|c| c.is_ascii_alphanumeric() || c == '#'))
        .unwrap_or("#000000");
    let wrapper = format!("<div style=\"color:{foreground};background-color:{background};font-family:Arial,sans-serif;font-size:14px;line-height:normal;text-align:start;{}\">{}</div>", escape(body.value().attr("style").unwrap_or("")), body.inner_html());
    let safe = sanitize_quote_fragment(&wrapper);
    if safe.len() > MAX_QUOTE {
        return Err(AppError::invalid("原邮件排版过大，改用纯文本引用"));
    }
    Ok(safe)
}

pub(crate) fn render_fragment(root: &std::path::Path, key: &str) -> Result<String, AppError> {
    let raw = mail_cache::read(root, key)?;
    let parsed = MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| AppError::invalid("邮件 MIME 无效，无法引用原文"))?;
    let html = parsed
        .body_html(0)
        .filter(|html| html.len() <= MAX_HTML)
        .ok_or_else(|| AppError::invalid("这封邮件没有可引用的正文"))?;
    prepare_quote(&html)
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
        let doc = srcdoc(&sanitized, false, false, None);
        assert!(doc.contains("img-src 'none'"));
        assert!(doc.contains(&format!("script-src 'sha256-{}'", bridge_hash())));
        assert!(doc.starts_with("<!DOCTYPE html>"));
        let allow = srcdoc(&sanitized, true, false, None);
        assert!(allow.contains("img-src https: data:"));
    }

    #[test]
    fn wrapper_css_follows_theme_mode() {
        let light = srcdoc("<p>x</p>", false, false, None);
        assert!(light.contains("color-scheme:light") && light.contains("#ffffff"));
        let dark = srcdoc("<p>x</p>", false, true, None);
        assert!(dark.contains("color-scheme:dark") && dark.contains("#1b2129"));
        assert!(!dark.contains("#ffffff"));
    }

    #[test]
    fn reader_font_size_is_injected_and_out_of_range_values_fall_back() {
        let large = srcdoc("<p>x</p>", false, false, Some(17));
        assert!(
            large.contains("html{font-size:17px}body{font-size:17px}"),
            "可调整字号应写入包装 CSS"
        );
        assert!(
            !large.contains("font-size:13px"),
            "指定字号时不再包含默认值"
        );
        let default = srcdoc("<p>x</p>", false, false, None);
        assert!(default.contains("font-size:13px"));
        // 白名单外（过小 / 过大）回落 13px，不做任意值注入。
        assert!(srcdoc("<p>x</p>", false, false, Some(10)).contains("font-size:13px"));
        assert!(srcdoc("<p>x</p>", false, false, Some(25)).contains("font-size:13px"));
        assert!(!srcdoc("<p>x</p>", false, false, Some(25)).contains("font-size:25px"));
    }

    #[test]
    fn quote_inlines_local_css_and_keeps_body_and_table_layout_without_external_resources() {
        let html = r##"<!doctype html><html><head>
          <link rel="stylesheet" href="https://example.test/unavailable.css">
          <link rel="stylesheet" href="file:///not-readable.css">
          <style>body { color:#262626; background-color:#fafafa; line-height:20px }
          .card { width:620px; border-collapse:collapse }
          .card td { padding:20px; vertical-align:top }
          .headline { font-size:24px; margin:0; text-align:center }
          .unsafe { position:fixed; z-index:999; background-image:url(https://example.test/track) }</style>
          </head><body><table class="card" border="0" cellpadding="0" cellspacing="0" align="center"><tr><td bgcolor="#ffffff">
          <h1 class="headline">Newsletter</h1><p class="unsafe" onclick="evil()">Content</p>
          <img src="https://example.test/track"><script>bad()</script></td></tr></table></body></html>"##;
        let safe = prepare_quote(html).unwrap();
        for wanted in [
            "width:620px",
            "padding:20px",
            "font-size:24px",
            "color:#262626",
            "line-height:20px",
            "background-color:#fafafa",
            "border=\"0\"",
            "cellpadding=\"0\"",
            "align=\"center\"",
        ] {
            assert!(safe.contains(wanted), "missing {wanted}: {safe}");
        }
        for unwanted in [
            "<style",
            "<link",
            "<script",
            "<img",
            "onclick",
            "position:",
            "z-index",
            "background-image",
            "example.test/track",
            "class=",
        ] {
            assert!(!safe.contains(unwanted), "unexpected {unwanted}");
        }
        assert_eq!(
            sanitize_compose(&safe),
            safe,
            "saving a quote keeps its safe inline layout"
        );
    }

    #[test]
    fn quote_limits_fall_back_instead_of_creating_an_uneditable_oversized_draft() {
        assert!(prepare_quote(&format!("<p>{}</p>", "x".repeat(200_001))).is_err());
        assert!(prepare_quote(&"<span>x</span>".repeat(4001)).is_err());
        let safe = sanitize_compose("<blockquote data-auumail-quote=\"true\"><table><tr><td style=\"border-top-width:0px;padding-left:20px;width:200px;position:fixed\">x</td></tr></table></blockquote>");
        assert!(safe.contains("data-auumail-quote=\"true\""));
        assert!(safe.contains("border-top-width:0px"));
        assert!(!safe.contains("position"));
    }

    #[test]
    fn compose_sanitize_allows_only_bitmap_data_image_on_img_src() {
        // 签名图片通道：img src 的位图 data:image/* 保留，alt/尺寸属性随行。
        let keep = sanitize_compose(
            "<p><img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"logo\" width=\"120\"></p>",
        );
        assert!(keep.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""));
        assert!(keep.contains("alt=\"logo\""));
        assert!(keep.contains("width=\"120\""));
        // svg（收件端普遍拦截）与其他 data: 一律剥除；远程 src 仍禁止。
        let strip = sanitize_compose(
            "<img src=\"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=\">\
             <img src=\"data:text/html;base64,PGI+PGI+\">\
             <img src=\"https://example.test/x.png\">",
        );
        assert!(!strip.contains("data:image/svg"));
        assert!(!strip.contains("data:text/html"));
        assert!(!strip.contains("example.test"));
        // a href 上的 data: 剥除，https 保留。
        let link = sanitize_compose(
            "<a href=\"data:image/png;base64,AAAA\">bad</a><a href=\"https://ok.test/\">ok</a>",
        );
        assert!(!link.contains("href=\"data:"));
        assert!(link.contains("href=\"https://ok.test/\""));
        // img 不复活事件处理器与脚本注入。
        let hostile = sanitize_compose(
            "<img src=\"data:image/png;base64,AAAA\" onerror=\"evil()\"><script>bad()</script>",
        );
        assert!(!hostile.contains("onerror"));
        assert!(!hostile.contains("<script"));
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
