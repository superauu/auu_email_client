//! ICS（RFC 5545 text/calendar）邀请解析：从邮件附件提取 VEVENT 入库事件视图。
//! 只读解析——不解析 VALARM / 不参与 RSVP 回复；解析失败或字段缺失的 VEVENT 跳过
//! （不伪造事件时间），不影响邮件本体与附件另存。
use crate::error::AppError;
use chrono::{NaiveDate, TimeZone, Utc};

/// 单条解析结果：starts_at 为 UTC ISO-8601（明确时区 / UTC 时）或无时区后缀的
/// 本地墙上时间（floating：全天事件与未知时区按「查看者本地时区」语义展示）。
pub(crate) struct IcsEvent {
    pub title: String,
    pub starts_at: String,
    pub time_zone: String,
}

/// ICS 附件字节上限：超限附件跳过解析（事件不产生），防恶意输入。
pub(crate) const MAX_ICS_BYTES: usize = 256 * 1024;
/// 每封邮件最多解析的日历附件数。
pub(crate) const MAX_CALENDAR_ATTACHMENTS: usize = 8;
/// 单封邮件最多产生的事件数（一封含大量 VEVENT 的日历不灌库）。
pub(crate) const MAX_EVENTS_PER_MESSAGE: usize = 20;

/// 解析全部 VEVENT：行折叠展开、SUMMARY 反转义、DTSTART 支持
/// UTC（Z）/ TZID（IANA，经 chrono-tz 转换）/ 全天 DATE / floating。
pub(crate) fn parse_events(raw: &[u8]) -> Vec<IcsEvent> {
    if raw.len() > MAX_ICS_BYTES {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(raw);
    let mut events = Vec::new();
    let mut in_event = false;
    let mut summary = String::new();
    let mut dtstart_params = String::new();
    let mut dtstart_value = String::new();
    let mut has_dtstart = false;
    for line in unfold(&text).lines() {
        match line.trim().to_ascii_uppercase().as_str() {
            "BEGIN:VEVENT" => {
                in_event = true;
                summary.clear();
                dtstart_params.clear();
                dtstart_value.clear();
                has_dtstart = false;
            }
            "END:VEVENT" => {
                if in_event && has_dtstart {
                    if let Some(event) =
                        build_event(summary.trim(), &dtstart_params, &dtstart_value)
                    {
                        if events.len() < MAX_EVENTS_PER_MESSAGE {
                            events.push(event);
                        }
                    }
                }
                in_event = false;
            }
            _ if in_event => {
                if let Some((name, params, value)) = split_property(line) {
                    match name.to_ascii_uppercase().as_str() {
                        "SUMMARY" => summary = unescape_text(&value),
                        "DTSTART" => {
                            dtstart_params = params.to_string();
                            dtstart_value = value.clone();
                            has_dtstart = true;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    events
}

/// RFC 5545 行折叠：换行后紧跟空格 / 制表符的续行拼接回单行。
fn unfold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(current) = chars.next() {
        if current == '\n' && matches!(chars.peek(), Some(' ') | Some('\t')) {
            chars.next();
            continue;
        }
        if current != '\r' {
            out.push(current);
        }
    }
    out
}

/// 拆分属性行为 (名称, 参数串含前导分号, 值)；无冒号的行不是属性。
fn split_property(line: &str) -> Option<(&str, &str, String)> {
    let colon = line.find(':')?;
    let (head, value) = (line[..colon].trim(), line[colon + 1..].trim().to_string());
    let name_end = head.find(';').unwrap_or(head.len());
    Some((&head[..name_end], &head[name_end..], value))
}

/// SUMMARY 等文本值的转义还原（\\, \\; \\n \\\\）；其余序列原样保留。
fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(current) = chars.next() {
        if current != '\\' {
            out.push(current);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(escaped) => out.push(escaped),
            None => out.push('\\'),
        }
    }
    out
}

fn build_event(title: &str, params: &str, value: &str) -> Option<IcsEvent> {
    let (starts_at, time_zone) = parse_dtstart(params, value)?;
    if title.is_empty() {
        return None;
    }
    Some(IcsEvent {
        title: title.chars().take(200).collect(),
        starts_at,
        time_zone,
    })
}

/// 解析 DTSTART 值；返回 (starts_at, time_zone)。
/// DATE 全天 / floating / 未知 TZID → 无时区后缀的墙上时间 + "floating"；
/// UTC → ...Z + "UTC"；已知 TZID → 转 UTC ISO + TZID 原文。
fn parse_dtstart(params: &str, value: &str) -> Option<(String, String)> {
    let value = value.trim();
    // 全天（VALUE=DATE 或纯 8 位日期）：本地零点 floating。
    if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((format!("{date}T00:00:00"), "floating".into()));
    }
    if value.len() != 15 && value.len() != 16 {
        return None;
    }
    let (date_part, time_part) = value.split_at(8);
    let time_part = time_part.strip_prefix('T')?;
    let (time_digits, utc) = match time_part.as_bytes().last() {
        Some(b'Z') if time_part.len() == 7 => (&time_part[..6], true),
        _ if time_part.len() == 6 => (time_part, false),
        _ => return None,
    };
    let date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    let time = chrono::NaiveTime::parse_from_str(time_digits, "%H%M%S").ok()?;
    let naive = date.and_time(time);
    if utc {
        return Some((
            format!("{}Z", naive.format("%Y-%m-%dT%H:%M:%S")),
            "UTC".into(),
        ));
    }
    let tzid = params
        .split(';')
        .find_map(|param| param.trim().strip_prefix("TZID=").map(str::to_owned))
        .filter(|tzid| !tzid.is_empty());
    if let Some(tzid) = tzid {
        if let Ok(tz) = tzid.parse::<chrono_tz::Tz>() {
            if let Some(utc_datetime) = tz.from_local_datetime(&naive).earliest() {
                let iso = utc_datetime
                    .with_timezone(&Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string();
                return Some((iso, tzid));
            }
        }
        // 未知时区或歧义本地时间：按 floating 展示，不做错误换算。
        return Some((
            format!("{}", naive.format("%Y-%m-%dT%H:%M:%S")),
            "floating".into(),
        ));
    }
    Some((
        format!("{}", naive.format("%Y-%m-%dT%H:%M:%S")),
        "floating".into(),
    ))
}

/// 提交邮件附带的事件（事务内调用；幂等 upsert，completed 不被覆盖）。
/// id 含邮件行 ID 与附件内序号，跨同步轮次稳定。
pub(crate) fn commit_events(
    tx: &rusqlite::Transaction<'_>,
    account_id: &str,
    message_id: &str,
    events: &[IcsEvent],
) -> Result<(), AppError> {
    for (index, event) in events.iter().enumerate() {
        let id = format!("ics:{message_id}:{index}");
        tx.execute(
            "INSERT INTO mail_events (id,account_id,message_id,title,starts_at,time_zone,completed,source) VALUES (?1,?2,?3,?4,?5,?6,0,'ics')
             ON CONFLICT(id) DO UPDATE SET title=excluded.title,starts_at=excluded.starts_at,time_zone=excluded.time_zone",
            rusqlite::params![id, account_id, message_id, event.title, event.starts_at, event.time_zone],
        )
        .map_err(|_| AppError::storage("日历事件未能保存"))?;
    }
    Ok(())
}

/// 从解析后的附件元数据判断是否为日历附件（media_type text/calendar，或
/// 无类型但文件名 .ics 的宽松回退）；返回是否值得取出字节解析。
pub(crate) fn is_calendar_attachment(media_type: &str, name: &str) -> bool {
    media_type.eq_ignore_ascii_case("text/calendar")
        || ((media_type.is_empty() || media_type == "application/octet-stream")
            && name.to_ascii_lowercase().ends_with(".ics"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(raw: &str) -> IcsEvent {
        let events = parse_events(raw.as_bytes());
        assert_eq!(events.len(), 1);
        events.into_iter().next().unwrap()
    }

    #[test]
    fn parses_utc_tzid_floating_and_all_day_events() {
        let utc = parse_one("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:季度评审\r\nDTSTART:20260301T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        assert_eq!(utc.starts_at, "2026-03-01T09:00:00Z");
        assert_eq!(utc.time_zone, "UTC");
        // TZID 带 DST 边界：上海 2026-03-01 无 DST，固定 +08:00。
        let shanghai = parse_one("BEGIN:VEVENT\r\nSUMMARY:发布检查\r\nDTSTART;TZID=Asia/Shanghai:20260301T090000\r\nEND:VEVENT\r\n");
        assert_eq!(shanghai.starts_at, "2026-03-01T01:00:00Z");
        assert_eq!(shanghai.time_zone, "Asia/Shanghai");
        let floating = parse_one(
            "BEGIN:VEVENT\r\nSUMMARY:本地时间事件\r\nDTSTART:20260301T090000\r\nEND:VEVENT\r\n",
        );
        assert_eq!(floating.starts_at, "2026-03-01T09:00:00");
        assert_eq!(floating.time_zone, "floating");
        let all_day = parse_one(
            "BEGIN:VEVENT\r\nSUMMARY:全天训练日\r\nDTSTART;VALUE=DATE:20260315\r\nEND:VEVENT\r\n",
        );
        assert_eq!(all_day.starts_at, "2026-03-15T00:00:00");
        assert_eq!(all_day.time_zone, "floating");
    }

    #[test]
    fn unfolds_lines_and_unescapes_summary() {
        let event = parse_one("BEGIN:VEVENT\r\nSUMMARY:季度评审：第二部分；续\r\n  +补充\r\nDTSTART:2026\r\n 0301T090000Z\r\nEND:VEVENT\r\n");
        assert_eq!(event.title, "季度评审：第二部分；续 +补充");
        assert_eq!(event.starts_at, "2026-03-01T09:00:00Z");
        let escaped = parse_one("BEGIN:VEVENT\r\nSUMMARY:A\\,B\\;C\\\\D\\nE\r\nDTSTART:20260301T090000Z\r\nEND:VEVENT\r\n");
        assert_eq!(escaped.title, "A,B;C\\D E");
    }

    #[test]
    fn skips_incomplete_events_and_caps_output() {
        // 无 DTSTART / 无 SUMMARY / 空标题 / 坏时间都跳过。
        let events = parse_events("BEGIN:VEVENT\r\nSUMMARY:没有时间\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20260301T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:坏时间\r\nDTSTART:not-a-date\r\nEND:VEVENT\r\n".as_bytes());
        assert!(events.is_empty());
        let mut raw = String::new();
        for index in 0..30 {
            raw.push_str(&format!("BEGIN:VEVENT\r\nSUMMARY:事件 {index}\r\nDTSTART:20260301T090000Z\r\nEND:VEVENT\r\n"));
        }
        assert_eq!(parse_events(raw.as_bytes()).len(), MAX_EVENTS_PER_MESSAGE);
        // 超限附件不解析。
        assert!(parse_events(&vec![b'x'; MAX_ICS_BYTES + 1]).is_empty());
    }

    #[test]
    fn unknown_tzid_stays_floating_and_daylight_saving_converts() {
        let unknown = parse_one("BEGIN:VEVENT\r\nSUMMARY:未知时区\r\nDTSTART;TZID=Mars/Phobos:20260301T090000\r\nEND:VEVENT\r\n");
        assert_eq!(unknown.starts_at, "2026-03-01T09:00:00");
        assert_eq!(unknown.time_zone, "floating");
        // 悉尼 2026-04-01 处于 AEDT（+11:00）。
        let sydney = parse_one("BEGIN:VEVENT\r\nSUMMARY:悉尼站会\r\nDTSTART;TZID=Australia/Sydney:20260401T100000\r\nEND:VEVENT\r\n");
        assert_eq!(sydney.starts_at, "2026-03-31T23:00:00Z");
    }

    #[test]
    fn commit_events_upserts_idempotently_and_preserves_completion() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        conn.execute_batch("INSERT INTO accounts(id,provider,email,display_name,profile_json,created_at) VALUES ('acc','imap','a@x.test','A','{\"status\":\"offline\"}','2026-11-20T00:00:00Z');
          INSERT INTO messages(id,account_id,remote_id,thread_id,subject,sender_name,sender_email,body_text,received_at,remote_folder,remote_uid_validity,remote_uid,remote_generation) VALUES ('m','acc','imap:inbox:1:1','m','邀请','Sender','s@x.test','正文','2026-11-20T00:00:00Z','INBOX',1,1,'gen');").unwrap();
        let events = vec![IcsEvent {
            title: "评审".into(),
            starts_at: "2026-03-01T09:00:00Z".into(),
            time_zone: "UTC".into(),
        }];
        let tx = conn.transaction().unwrap();
        commit_events(&tx, "acc", "m", &events).unwrap();
        tx.commit().unwrap();
        // 用户勾选完成 → 重新同步（同一 id）→ completed 保留，标题可更新。
        conn.execute("UPDATE mail_events SET completed=1 WHERE id='ics:m:0'", [])
            .unwrap();
        let updated = vec![IcsEvent {
            title: "评审（改期）".into(),
            starts_at: "2026-03-02T09:00:00Z".into(),
            time_zone: "UTC".into(),
        }];
        let tx = conn.transaction().unwrap();
        commit_events(&tx, "acc", "m", &updated).unwrap();
        tx.commit().unwrap();
        let (title, completed, count): (String, i64, i64) = conn.query_row("SELECT title,completed,(SELECT COUNT(*) FROM mail_events) FROM mail_events WHERE id='ics:m:0'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(title, "评审（改期）");
        assert_eq!(completed, 1);
        assert_eq!(count, 1, "同一附件重复提交不产生重复事件");
        // 邮件行删除 → 触发器解绑（事件保留但 message_id 置空）。
        conn.execute("DELETE FROM messages WHERE id='m'", [])
            .unwrap();
        let detached: Option<String> = conn
            .query_row(
                "SELECT message_id FROM mail_events WHERE id='ics:m:0'",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert_eq!(detached, None);
    }

    #[test]
    fn parse_raw_and_pop3_commit_end_to_end_stores_events() {
        use std::sync::{Arc, Mutex};
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO accounts (id,provider,email,display_name,profile_json,created_at) VALUES ('p','pop3','p@x.test','P','{\"status\":\"offline\"}','2026-11-20T00:00:00Z')", []).unwrap();
        let generation: String = conn
            .query_row(
                "SELECT sync_generation FROM accounts WHERE id='p'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let db: crate::mail_sync::SharedDb = Arc::new(Mutex::new(conn));
        let raw = "From: Sender <s@x.test>\r\nTo: p@x.test\r\nSubject: 邀请\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\n请查看邀请。\r\n--B\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\nContent-Disposition: attachment; filename=\"invite.ics\"\r\n\r\nBEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nSUMMARY:季度评审会\r\nDTSTART;TZID=Asia/Shanghai:20260301T090000\r\nUID:evt-1@x.test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--B--\r\n".as_bytes();
        let parse = || crate::mail_sync::parse_raw(1, raw, crate::now_utc(), true, false).unwrap();
        let parsed = parse();
        assert_eq!(
            parsed.ics_events.len(),
            1,
            "合成 MIME 的 text/calendar 附件应解析出事件"
        );
        crate::mail_sync::commit_pop3(
            &db,
            "p",
            &generation,
            &["uidl-1".to_owned()],
            &[("uidl-1".to_owned(), parsed)],
        )
        .unwrap();
        {
            let conn = db.lock().unwrap();
            let (count, title, zone): (i64, String, String) = conn.query_row("SELECT COUNT(*),(SELECT title FROM mail_events WHERE source='ics' LIMIT 1),(SELECT time_zone FROM mail_events WHERE source='ics' LIMIT 1) FROM mail_events WHERE source='ics'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
            assert_eq!(count, 1);
            assert_eq!(zone, "Asia/Shanghai");
            assert!(title.contains("评审"), "SUMMARY 应反转为可读标题");
        }
        // 二轮重复提交同一 UIDL（同步幂等路径）→ 事件不重复。
        crate::mail_sync::commit_pop3(
            &db,
            "p",
            &generation,
            &["uidl-1".to_owned()],
            &[("uidl-1".to_owned(), parse())],
        )
        .unwrap();
        let count: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM mail_events WHERE source='ics'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "重复同步 upsert 同一事件 id，不产生重复行");
    }

    #[test]
    fn calendar_attachment_detection() {
        assert!(is_calendar_attachment("text/calendar", "invite.ics"));
        assert!(is_calendar_attachment(
            "application/octet-stream",
            "邀请.ics"
        ));
        assert!(!is_calendar_attachment("application/pdf", "invite.ics"));
        assert!(is_calendar_attachment("text/calendar", ""));
    }
}
