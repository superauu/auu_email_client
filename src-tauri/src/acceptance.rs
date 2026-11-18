//! 真实服务器验收（自建 GreenMail 全栈：IMAP / POP3 / SMTP，非演示 harness、非 MockStream）。
//!
//! 前置（详见 docs/STATUS.md 验收节）：
//! 1. `cd artifacts/greenmail && java -Xmx2g -Dgreenmail.smtp.hostname=127.0.0.1 -Dgreenmail.smtp.port=13025 -Dgreenmail.imap.hostname=127.0.0.1 -Dgreenmail.imap.port=13143 -Dgreenmail.pop3.hostname=127.0.0.1 -Dgreenmail.pop3.port=13110 "-Dgreenmail.users=auu:auupass@mail.example.com,cc:ccpass@mail.example.com,bcc:bccpass@mail.example.com,bulk:bulkpass@mail.example.com" -jar greenmail.jar`；
//!    注意：不要启用 `-Dgreenmail.setup.api`——API 服务器的 preloadDirectory 会把注入的邮件持久化并在重启后自动回灌，导致用例状态污染（2026-11-18 实测）。
//! 2. `python tls_proxy.py`（TLS 9301→IMAP、9302→POP3、9303→SMTP，终止自签 CA 证书）；
//! 3. `certutil -user -addstore Root ca.cer`（把验收 CA 装入用户信任库，供 Schannel 信任）。
//! 运行：`AUUMAIL_ACCEPTANCE=1 cargo test --manifest-path src-tauri/Cargo.toml --lib acceptance::tests -- --nocapture --test-threads=1`
//! 未设置 AUUMAIL_ACCEPTANCE 时全部跳过，常规 `check:native` 不受影响。
//!
//! 边界：这验证真实协议路径对接真实第三方服务器实现（GreenMail），
//! 不等于 Gmail / QQ / 163 等公网邮箱的怪癖验收（IMAP 扩展差异、限速、垃圾过滤）。
#[cfg(test)]
mod tests {
    use crate::compose_attach::ComposeStaging;
    use crate::mail_sync::SharedDb;
    use crate::mail_verify::Endpoints;
    use crate::{
        account_service, credentials, drafts, imap_jobs, mail_store, mail_verify, message_changes,
        outbox, storage,
    };
    use base64::Engine;
    use mail_parser::MimeHeaders;
    use std::net::TcpStream;
    use std::sync::{Arc, Mutex};

    const IMAP_PORT: u16 = 9301;
    const POP3_PORT: u16 = 9302;
    const SMTP_PORT: u16 = 9303;
    const HOST: &str = "127.0.0.1";
    const DOMAIN: &str = "mail.example.com";
    const TLS_NAME: &str = "localhost";

    fn enabled() -> bool {
        std::env::var("AUUMAIL_ACCEPTANCE").is_ok()
    }

    fn marker() -> String {
        format!("acc-{}", uuid::Uuid::new_v4())
    }

    fn fresh_db(tag: &str) -> (SharedDb, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("auumail-acceptance-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = storage::initialize(&dir.join("db.sqlite")).unwrap();
        (Arc::new(Mutex::new(conn)), dir)
    }

    /// 真实账号保存路径：mail_verify 探针（真实 TLS IMAP / SMTP 登录）→ account_service::persist（keyring）。
    fn save_account(
        db: &SharedDb,
        provider: &str,
        user: &str,
        pass: &str,
        receive_only: bool,
    ) -> String {
        let receive_port = if provider == "pop3" {
            POP3_PORT
        } else {
            IMAP_PORT
        };
        let account = mail_store::RealAccount {
            receive_only,
            oauth: None,
            provider: provider.into(),
            id: None,
            name: format!("验收 {user}"),
            email: format!("{user}@{DOMAIN}"),
            imap_host: HOST.into(),
            imap_port: receive_port,
            imap_security: crate::imap_transport::Security::Tls,
            smtp_host: HOST.into(),
            smtp_port: SMTP_PORT,
            auth_user: user.into(),
            avatar: "验".into(),
            color: "teal".into(),
        };
        let expected = {
            let conn = db.lock().unwrap();
            account_service::existing_generation(&conn, &account).unwrap()
        };
        let endpoints = Endpoints {
            provider: provider.into(),
            receive_only,
            imap_host: HOST.into(),
            imap_port: receive_port,
            imap_security: crate::imap_transport::Security::Tls,
            smtp_host: HOST.into(),
            smtp_port: SMTP_PORT,
            auth_user: user.into(),
        };
        let report = mail_verify::verify(&endpoints, pass);
        mail_verify::ensure_verified(&report).unwrap();
        let id = account_service::persist(db, account, expected, pass.into()).unwrap();
        assert!(credentials::exists("imap", &id).unwrap());
        id
    }

    /// 清理本测试落下的账号与凭据；尽力而为（keyring 条目不残留）。
    fn cleanup(db: &SharedDb, ids: &[String]) {
        for id in ids {
            let _ = credentials::delete("imap", id);
            if let Ok(mut conn) = db.lock() {
                let _ = mail_store::delete_real_account(&mut conn, id);
            }
        }
    }

    /// 全量同步入队（真实执行器：imap_jobs::run 含回写 flush）。
    fn sync(db: &SharedDb, id: &str) -> crate::mail_sync::SyncOutcome {
        {
            let mut conn = db.lock().unwrap();
            let ids = [id.to_string()];
            imap_jobs::request(&mut conn, &ids).unwrap();
        }
        imap_jobs::run(db, id).unwrap()
    }

    /// 原始 SMTP 注入（独立于被测代码的对照通道）：一条 TLS 连接上发多封原始 MIME。
    fn raw_send(user: &str, pass: &str, messages: &[String]) {
        let stream = TcpStream::connect((HOST, SMTP_PORT)).unwrap();
        let mut tls = native_tls::TlsConnector::new()
            .unwrap()
            .connect(TLS_NAME, stream)
            .unwrap();
        let read = |tls: &mut native_tls::TlsStream<TcpStream>| {
            crate::mail_verify::read_response(tls).unwrap()
        };
        let write = |tls: &mut native_tls::TlsStream<TcpStream>, cmd: &str| {
            crate::mail_verify::write_command(tls, cmd).unwrap()
        };
        let (code, _) = read(&mut tls);
        assert_eq!(code, 220);
        write(&mut tls, &format!("EHLO {TLS_NAME}"));
        let (_, caps) = read(&mut tls);
        assert!(caps.to_ascii_lowercase().contains("auth"));
        let plain =
            base64::engine::general_purpose::STANDARD.encode(format!("\u{0}{user}\u{0}{pass}"));
        write(&mut tls, &format!("AUTH PLAIN {plain}"));
        let (code, _) = read(&mut tls);
        assert_eq!(code, 235, "AUTH PLAIN 应成功");
        for message in messages {
            let from = message
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("from:"))
                .map(|line| line["from:".len()..].trim())
                .expect("注入消息应有 From 头");
            // SMTP 投递按信封（RCPT TO）而非头字段：To / Cc / Bcc 头里的每个地址都要逐一 RCPT。
            let mut recipients: Vec<String> = Vec::new();
            for header in ["to:", "cc:", "bcc:"] {
                if let Some(line) = message
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with(header))
                {
                    let value = line[header.len()..].trim();
                    for address in value.split(',') {
                        let address = address.trim().trim_start_matches('<').trim_end_matches('>');
                        if !address.is_empty() {
                            recipients.push(address.to_string());
                        }
                    }
                }
            }
            assert!(!recipients.is_empty());
            write(&mut tls, &format!("MAIL FROM:<{from}>"));
            let (code, _) = read(&mut tls);
            assert_eq!(code, 250);
            for to in &recipients {
                write(&mut tls, &format!("RCPT TO:<{to}>"));
                let (code, _) = read(&mut tls);
                assert_eq!(code, 250);
            }
            write(&mut tls, "DATA");
            let (code, _) = read(&mut tls);
            assert_eq!(code, 354);
            // dot-stuffing + CRLF.CRLF 终止；原始 MIME 是 CRLF 行尾，载荷必须以 CRLF
            // 结束，终止点才能独占一行（否则服务器一直等待，30s 后断开——实测踩坑）。
            let stuffed = message.replace("\r\n.", "\r\n..");
            let payload = if stuffed.ends_with("\r\n") {
                stuffed
            } else {
                format!("{stuffed}\r\n")
            };
            use std::io::Write;
            tls.write_all(payload.as_bytes()).unwrap();
            write(&mut tls, ".");
            let (code, _) = read(&mut tls);
            assert_eq!(code, 250, "消息应被服务器接受");
        }
        write(&mut tls, "QUIT");
    }

    /// 独立核对通道：不经被测代码，直接用 imap 库 + TLS 打开服务器会话。
    fn raw_imap(user: &str, pass: &str) -> imap::Session<native_tls::TlsStream<TcpStream>> {
        let stream = TcpStream::connect((HOST, IMAP_PORT)).unwrap();
        let tls = native_tls::TlsConnector::new()
            .unwrap()
            .connect(TLS_NAME, stream)
            .unwrap();
        imap::Client::new(tls)
            .login(user, pass)
            .map_err(|(_, err)| err)
            .unwrap()
    }

    /// 清空服务器邮箱（测试重跑时不依赖服务器初始状态；IMAP 会话独立于被测代码）。
    fn purge_mailbox(user: &str, pass: &str) {
        let mut session = raw_imap(user, pass);
        session.select("INBOX").unwrap();
        let uids: Vec<String> = session
            .uid_search("ALL")
            .unwrap()
            .into_iter()
            .map(|uid| uid.to_string())
            .collect();
        if !uids.is_empty() {
            let set = uids.join(",");
            let _ = session.uid_store(&set, "+FLAGS (\\Deleted)");
            session.expunge().unwrap();
        }
        session.logout().unwrap();
    }

    /// 服务器侧搜索用 Message-ID（ASCII 头）：中文主题在服务器上是 RFC2047 编码，
    /// 原始 SEARCH SUBJECT 匹配不到编码后的头文本。
    fn uid_search(
        session: &mut imap::Session<native_tls::TlsStream<TcpStream>>,
        mailbox: &str,
        message_id: &str,
    ) -> std::collections::HashSet<u32> {
        session.select(mailbox).unwrap();
        session
            .uid_search(&format!("HEADER MESSAGE-ID \"{message_id}\""))
            .unwrap()
    }

    /// 收件箱全部邮件（分页取全：page_messages 单页有上限）。
    fn inbox_subjects(db: &SharedDb, account: &str) -> Vec<(String, String)> {
        let conn = db.lock().unwrap();
        let mut all: Vec<(String, String)> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = crate::mail_store::page_messages(
                &conn,
                &mail_store::MessageQuery {
                    account_id: Some(account.into()),
                    mailbox: Some("inbox".into()),
                    cursor: cursor.clone(),
                    limit: Some(500),
                    ..mail_store::MessageQuery::default()
                },
            )
            .unwrap();
            all.extend(page.items.into_iter().map(|m| (m.id, m.subject)));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        all
    }

    /// 回归记录：mail-parser 0.10.2 对 RFC 2231 的单段与多段行为（2026-11-18 验收实测）。
    /// 单段 `filename*=utf-8''…` 正确还原；多段续行按段独立解码后拼接，段边界落在
    /// UTF-8 多字节字符中间时损坏文件名——发送端因此用单段（见 outbox rfc2231_parameters）。
    #[test]
    fn probe_2231_single_vs_multi() {
        use mail_parser::{MessageParser, MimeHeaders};
        let encoded = "%E5%AD%A3%E5%BA%A6%E6%8A%A5%E5%91%8A-%E9%AA%8C%E8%AF%81%E7%89%88.txt";
        let single = format!("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*=utf-8''{encoded}\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\r\n");
        let msg_single = format!("From: a@b\r\nSubject: t\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=BB\r\n\r\n--BB\r\n{single}--BB--\r\n");
        let parsed = MessageParser::default()
            .parse(msg_single.as_bytes())
            .unwrap();
        let part = parsed.attachment(0).expect("attachment 0");
        assert_eq!(
            part.attachment_name().map(|n| n.to_string()).as_deref(),
            Some("季度报告-验证版.txt"),
            "单段 RFC2231 应正确还原"
        );

        let (seg0, seg1) = encoded.split_at(63);
        let multi = format!("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*0*=utf-8''{seg0}; filename*1*=utf-8''{seg1}\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\r\n");
        let msg_multi = format!("From: a@b\r\nSubject: t\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=BB\r\n\r\n--BB\r\n{multi}--BB--\r\n");
        let parsed = MessageParser::default()
            .parse(msg_multi.as_bytes())
            .unwrap();
        let part = parsed.attachment(0).expect("attachment 0");
        let merged = part.attachment_name().map(|n| n.to_string());
        assert_ne!(
            merged.as_deref(),
            Some("季度报告-验证版.txt"),
            "多段续行在 mail-parser 0.10.2 接收侧损坏文件名（上游限制，勿依赖）"
        );
    }

    /// ── 验收①：中文附件名修复的端到端复验 ──────────────────────────
    /// 真实 outbox（RFC 2231 + RFC2047 双形式）→ GreenMail SMTP 投递 → 真实 IMAP 收信
    /// → requestBody 原文下载 → mail-parser 解析 → 附件名与字节逐字一致。
    #[test]
    fn chinese_attachment_name_survives_real_smtp_to_imap_roundtrip() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        let (db, _dir) = fresh_db("attachment");
        let account = save_account(&db, "imap", "auu", "auupass", false);
        let tag = marker();

        let content = format!("中文附件内容验收 {}\r\n第二行以保持多行结构", tag).into_bytes();
        let attachment = crate::drafts::DraftAttachment {
            id: "att-1".into(),
            name: "季度报告-验收版.txt".into(),
            size: content.len(),
            media_type: "text/plain".into(),
            sha256: crate::mail_recovery::hash(&content),
        };
        let draft = drafts::Draft {
            id: format!("draft-acc-{}", uuid::Uuid::new_v4()),
            account_id: account.clone(),
            to: format!("auu@{DOMAIN}"),
            cc: String::new(),
            bcc: String::new(),
            subject: format!("{tag} 附件名复验"),
            body_text: format!("{tag} 正文与附件同发。"),
            body_html: None,
            reply_to_id: None,
            attachments: vec![attachment.clone()],
            updated_at: crate::now_utc(),
            revision: 0,
        };
        let staging = ComposeStaging::default();
        staging
            .stage(&draft.id, &attachment, content.clone())
            .unwrap();
        let saved = {
            let mut conn = db.lock().unwrap();
            drafts::save(&mut conn, &draft, &account, &staging).unwrap()
        };
        {
            let mut conn = db.lock().unwrap();
            let saved_revision = saved.draft.as_ref().map(|d| d.revision).unwrap_or(0);
            outbox::enqueue(&mut conn, &saved.id, saved_revision).unwrap();
        }
        let sends = outbox::run(&db, &account).unwrap();
        assert!(sends, "发送队列应有任务被处理");
        {
            let conn = db.lock().unwrap();
            let entry = outbox::list(&conn)
                .unwrap()
                .into_iter()
                .find(|e| e.draft_id == saved.id)
                .unwrap();
            assert_eq!(entry.state, "sent", "真实 SMTP 投递应确认 sent");
            assert_eq!(entry.attempts, 1);
            assert_eq!(entry.last_error, None);
        }

        let outcome = sync(&db, &account);
        assert!(outcome.fetched_messages >= 1);
        let messages = inbox_subjects(&db, &account);
        let hit = messages
            .iter()
            .find(|(_, subject)| subject.contains(&tag))
            .unwrap();
        let message_id = hit.0.clone();

        // 生产收信路径：状态核对 + 原文下载 + 附件元数据解析。
        let row = {
            let conn = db.lock().unwrap();
            mail_store::message_by_id(&conn, &account, &message_id).unwrap()
        };
        let downloaded = crate::imap_body::request(
            &mut db.lock().unwrap(),
            &account,
            &message_id,
            row.state_revision,
        )
        .unwrap();
        let got = downloaded
            .attachments
            .iter()
            .find(|a| a.index == 0)
            .unwrap();
        assert_eq!(
            got.name, "季度报告-验收版.txt",
            "收件端解析的附件名必须还原"
        );
        assert_eq!(got.size, content.len());

        // 生产缓存 + 解析链的字节级核对（与导出共享同一条 load / read / parse 路径）。
        let (root, key, _meta) = {
            let conn = db.lock().unwrap();
            crate::mail_attachments::load(&conn, &account, &message_id, 0).unwrap()
        };
        let raw = crate::mail_cache::read(&root, &key).unwrap();
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| panic!("缓存原文 MIME 无效"))
            .unwrap();
        let part = parsed
            .attachment(0)
            .filter(|part| part.contents().len() == attachment.size)
            .expect("缓存附件索引应与元数据一致");
        assert_eq!(part.contents(), content, "附件字节必须与原文件一致");
        assert_eq!(
            part.attachment_name()
                .map(|name| name.to_string())
                .as_deref(),
            Some("季度报告-验收版.txt"),
            "mail-parser 还原的附件名必须与原名一致"
        );
        cleanup(&db, &[account]);
    }

    /// ── 验收②：Cc / Bcc 独立收件与 Bcc 头隐私 ──────────────────────
    /// 生产 outbox 按信封 RCPT TO 分别投递到每个收件人邮箱；已投递副本不含 Bcc 头。
    #[test]
    fn cc_and_bcc_recipients_each_receive_independent_delivery() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        // 走生产发送路径（outbox DATA 装配 + 真实 SMTP 会话），Cc / Bcc 是草稿真实字段。
        let (db, _dir) = fresh_db("ccb");
        let account = save_account(&db, "imap", "auu", "auupass", false);
        let tag = marker();
        let draft = drafts::Draft {
            id: format!("draft-acc-{}", uuid::Uuid::new_v4()),
            account_id: account.clone(),
            to: format!("auu@{DOMAIN}"),
            cc: format!("cc@{DOMAIN}"),
            bcc: format!("bcc@{DOMAIN}"),
            subject: format!("{tag} 抄送密送验收"),
            body_text: format!("{tag} 抄送与密送独立投递正文。"),
            body_html: None,
            reply_to_id: None,
            attachments: Vec::new(),
            updated_at: crate::now_utc(),
            revision: 0,
        };
        let saved = {
            let mut conn = db.lock().unwrap();
            drafts::save(&mut conn, &draft, &account, &ComposeStaging::default()).unwrap()
        };
        {
            let mut conn = db.lock().unwrap();
            let saved_revision = saved.draft.as_ref().map(|d| d.revision).unwrap_or(0);
            outbox::enqueue(&mut conn, &saved.id, saved_revision).unwrap();
        }
        assert!(
            outbox::run(&db, &account).unwrap(),
            "发送队列应有任务被处理"
        );

        // auu（To）、cc（Cc）、bcc（Bcc）各自的账号分别真实收信。
        let ids: Vec<String> = ["auu", "cc", "bcc"]
            .iter()
            .map(|user| match user.as_ref() {
                "auu" => account.clone(),
                other => save_account(&db, "imap", other, &format!("{other}pass"), false),
            })
            .collect();
        for id in &ids {
            sync(&db, id);
        }
        for id in &ids {
            let subjects = inbox_subjects(&db, id);
            assert!(
                subjects.iter().any(|(_, subject)| subject.contains(&tag)),
                "每个收件人邮箱都应独立收到该邮件（信封 RCPT 逐一投递）"
            );
        }
        // Bcc 隐私与 Cc 保留：核对 auu 收到的原始副本（独立 IMAP 通道，按 Message-ID 搜索）。
        let sent_message_id = {
            let conn = db.lock().unwrap();
            outbox::list(&conn)
                .unwrap()
                .into_iter()
                .find(|e| e.draft_id == saved.id)
                .and_then(|e| e.message_id)
                .expect("发送条目应带 Message-ID")
        };
        let mut session = raw_imap("auu", "auupass");
        let uids = uid_search(&mut session, "INBOX", &sent_message_id);
        assert_eq!(uids.len(), 1);
        let uid = uids
            .iter()
            .next()
            .expect("服务器应能搜索到该邮件")
            .to_string();
        let fetched = session.uid_fetch(&uid, "(BODY.PEEK[])").unwrap();
        let raw = fetched.iter().next().unwrap();
        let bytes = raw.body().expect("RFC822 body");
        let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        assert!(!text.contains("\r\nbcc:"), "投递副本不得泄漏 Bcc 头");
        assert!(text.contains(&format!("cc: cc@{DOMAIN}")), "Cc 头保留");
        session.logout().unwrap();
        // To 投影保留。
        let subjects = inbox_subjects(&db, &account);
        let (message_id, _) = subjects
            .iter()
            .find(|(_, subject)| subject.contains(&tag))
            .unwrap();
        let row = {
            let conn = db.lock().unwrap();
            mail_store::message_by_id(&conn, &account, message_id).unwrap()
        };
        assert!(row.to.iter().any(|t| t.contains("auu@")), "To 投影保留");
        cleanup(&db, &ids);
    }

    /// ── 验收③：真实 POP3 收信路径 ─────────────────────────────────
    /// POP3 账号经 TLS 到真实服务器完成 UIDL 增量同步；消息入本机库。
    #[test]
    fn pop3_account_receives_mail_from_real_server() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        let (db, _dir) = fresh_db("pop3");
        // 测试重跑自包含：清空 auu 邮箱（避免历史轮次消息占满 MAX_DOWNLOAD=50 窗口）。
        purge_mailbox("auu", "auupass");
        // 先经 SMTP 放一封新邮件到 auu 的 INBOX，POP3 再收。
        let tag = marker();
        raw_send(
            "auu",
            "auupass",
            &[format!(
                "From: sender@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: {tag} POP3 验收\r\nMessage-ID: <{tag}@{DOMAIN}>\r\nDate: Sat, 7 Feb 2026 11:00:00 +0000\r\n\r\n{tag} POP3 收信正文。"
            )],
        );
        let account = save_account(&db, "pop3", "auu", "auupass", false);
        // POP3 每轮最多下载 50 封（MAX_DOWNLOAD），循环到拉净或上限。
        let mut rounds = 0usize;
        let mut fetched_first = 0usize;
        let mut reached_marker = false;
        for _ in 0..5 {
            let outcome = sync(&db, &account);
            rounds += 1;
            if rounds == 1 {
                fetched_first = outcome.fetched_messages;
            }
            if outcome.fetched_messages == 0 {
                break;
            }
            reached_marker = inbox_subjects(&db, &account)
                .iter()
                .any(|(_, subject)| subject.contains(&tag));
            if reached_marker {
                break;
            }
        }
        assert!(fetched_first >= 1, "POP3 首次同步应拉到消息");
        assert!(reached_marker, "POP3 收到的邮件应进入本机库");
        // 增量：第二次同步不再重复拉取（UIDL 增量）。
        let second = sync(&db, &account);
        assert_eq!(second.fetched_messages, 0, "无新邮件时增量同步应为 0");
        cleanup(&db, &[account]);
    }

    /// ── 验收④：真实 IMAP 删除 / 移动（UID MOVE 落盘核对） ──────────
    /// 意图队列 → 真实 UID MOVE（服务器支持 MOVE）→ 独立 IMAP 会话核对物理归属。
    #[test]
    fn delete_and_move_intents_really_move_messages_on_server() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        let (db, _dir) = fresh_db("move");
        // 服务器上确保 Archive / Trash 目录（名称启发式映射 kind）。
        {
            let mut session = raw_imap("auu", "auupass");
            for folder in ["Archive", "Trash"] {
                if session.list(Some(""), Some(folder)).unwrap().is_empty() {
                    session.create(folder).unwrap();
                }
            }
            session.logout().unwrap();
        }
        let account = save_account(&db, "imap", "auu", "auupass", false);
        let tag_move = marker();
        let tag_delete = marker();
        raw_send(
            "auu",
            "auupass",
            &[
                format!(
                    "From: sender@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: {tag_move} 移动验收\r\nMessage-ID: <{tag_move}@{DOMAIN}>\r\nDate: Sat, 7 Feb 2026 12:00:00 +0000\r\n\r\n{tag_move} 正文。"
                ),
                format!(
                    "From: sender@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: {tag_delete} 删除验收\r\nMessage-ID: <{tag_delete}@{DOMAIN}>\r\nDate: Sat, 7 Feb 2026 12:01:00 +0000\r\n\r\n{tag_delete} 正文。"
                ),
            ],
        );
        sync(&db, &account);
        let find = |db: &SharedDb, tag: &str| {
            inbox_subjects(db, &account)
                .into_iter()
                .find(|(_, subject)| subject.contains(tag))
                .map(|(id, _)| id)
                .expect("被测邮件应已同步")
        };
        let move_id = find(&db, &tag_move);
        let delete_id = find(&db, &tag_delete);
        // MOVE 到 Archive（真实 UID MOVE）。
        let archive_folder = format!("{account}:folder:Archive");
        message_changes::update(
            &mut db.lock().unwrap(),
            &account,
            &move_id,
            &message_changes::Patch {
                move_to: Some(archive_folder.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        // DELETE 到废纸篓（唯一 trash 目录）。
        message_changes::update(
            &mut db.lock().unwrap(),
            &account,
            &delete_id,
            &message_changes::Patch {
                delete: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        sync(&db, &account); // 回写在同步会话内 flush。

        // 本机投影：移动 → archive，删除 → trash。
        let conn = db.lock().unwrap();
        let moved = mail_store::message_by_id(&conn, &account, &move_id).unwrap();
        assert_eq!(moved.mailbox, "archive");
        let deleted = mail_store::message_by_id(&conn, &account, &delete_id).unwrap();
        assert_eq!(deleted.mailbox, "trash");
        drop(conn);

        // 独立会话核对服务器物理归属：INBOX 不再有；Archive / Trash 各一。
        let mut session = raw_imap("auu", "auupass");
        assert!(
            uid_search(&mut session, "INBOX", &format!("<{tag_move}@{DOMAIN}>")).is_empty(),
            "移动后 INBOX 不应再有该邮件"
        );
        assert!(
            uid_search(&mut session, "INBOX", &format!("<{tag_delete}@{DOMAIN}>")).is_empty(),
            "删除后 INBOX 不应再有该邮件"
        );
        let archived = uid_search(&mut session, "Archive", &format!("<{tag_move}@{DOMAIN}>"));
        assert_eq!(archived.len(), 1, "Archive 目录应有被移动邮件");
        let trashed = uid_search(&mut session, "Trash", &format!("<{tag_delete}@{DOMAIN}>"));
        assert_eq!(trashed.len(), 1, "Trash 目录应有被删除邮件");
        session.logout().unwrap();
        cleanup(&db, &[account]);
    }

    /// ── 验收⑤：Outlook 风格消息的头证据与同标题归组输入 ────────────
    /// Outlook 风格邮件（RE / FW 前缀 + Thread-Index / References）经真实链路后证据完整。
    #[test]
    fn outlook_style_headers_survive_real_roundtrip_for_subject_grouping() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        // Outlook 风格邮件：RE / FW 主题前缀 + In-Reply-To / References / Thread-Index。
        // AuuMail 的会话归组由「同标题折叠」承担（前缀归一化在 tests/threading.test.ts 与
        // e2e 覆盖）；这里验收真实链路不损坏归组所需的头证据。
        let (db, _dir) = fresh_db("outlook");
        let account = save_account(&db, "imap", "auu", "auupass", false);
        let tag = marker();
        let root_id = format!("{tag}-1@{DOMAIN}");
        raw_send(
            "auu",
            "auupass",
            &[
                format!(
                    "From: colleague@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: {tag} Project kickoff\r\nMessage-ID: <{root_id}>\r\nThread-Index: Ac{tag}\r\nDate: Sat, 7 Feb 2026 09:00:00 +0000\r\n\r\n{tag} 根邮件。"
                ),
                format!(
                    "From: colleague@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: RE: {tag} Project kickoff\r\nMessage-ID: <{tag}-2@{DOMAIN}>\r\nIn-Reply-To: <{root_id}>\r\nReferences: <{root_id}>\r\nThread-Index: Ac{tag}#\r\nDate: Sat, 7 Feb 2026 09:30:00 +0000\r\n\r\n{tag} Outlook 回复。"
                ),
                format!(
                    "From: colleague@{DOMAIN}\r\nTo: auu@{DOMAIN}\r\nSubject: FW: {tag} Project kickoff\r\nMessage-ID: <{tag}-3@{DOMAIN}>\r\nReferences: <{root_id}> <{tag}-2@{DOMAIN}>\r\nThread-Index: Ac{tag}#\r\nDate: Sat, 7 Feb 2026 10:30:00 +0000\r\n\r\n{tag} Outlook 转发。"
                ),
            ],
        );
        sync(&db, &account);
        let conn = db.lock().unwrap();
        let rows: Vec<(String, String, Option<String>)> = conn
            .prepare("SELECT subject, header_message_id, header_references FROM messages WHERE account_id=?1 AND subject LIKE ?2 ORDER BY received_at")
            .unwrap()
            .query_map(rusqlite::params![account, format!("%{tag}%")], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), 3, "三封 Outlook 风格邮件都应入库");
        // 主题逐字保留（RE / FW 前缀完好，归一化交给 UI 层）；同一秒注入，顺序不保证。
        let root = rows
            .iter()
            .find(|(subject, _, _)| subject.starts_with(&tag))
            .expect("根邮件");
        let reply = rows
            .iter()
            .find(|(subject, _, _)| subject.starts_with("RE: "))
            .expect("回复");
        let forward = rows
            .iter()
            .find(|(subject, _, _)| subject.starts_with("FW: "))
            .expect("转发");
        // Message-ID 与 References 证据链完整（恢复 / 未来 threadId 分组依赖）。
        // header_message_id 按约定存裸 id（mail-parser parse_id 剥尖括号，迁移 013 / outbox 组装时补 <>）。
        assert_eq!(root.1, root_id);
        assert!(reply.1.contains(&format!("{tag}-2@{DOMAIN}")));
        assert!(
            forward.2.as_deref().is_some_and(
                |refs| refs.contains(&root_id) && refs.contains(&format!("{tag}-2@{DOMAIN}"))
            ),
            "转发的 References 证据链应完整保留"
        );
        drop(conn);
        cleanup(&db, &[account]);
    }

    /// ── 验收⑥：大邮箱真实同步吞吐 ─────────────────────────────────
    /// 经真实 SMTP 注入 2000 封 → IMAP 全量同步计时 → 二次增量归零。
    /// 规模边界：GreenMail 内存实现在万封级 FETCH 上会挂起（探针实测），因此本验收
    /// 定在 2000 封；数据只代表本机协议链路吞吐，不外推为 Gmail / Dovecot 等真实
    /// 服务器在 5 万封级的表现（网络 / 分页 / 限速差异另计）。
    #[test]
    fn large_mailbox_syncs_and_measures_real_throughput() {
        if !enabled() {
            eprintln!("skip: AUUMAIL_ACCEPTANCE 未设置");
            return;
        }
        let (db, _dir) = fresh_db("huge");
        // 专用 bulk 账号：不污染同服务器上其他验收用例的 auu 邮箱。
        let account = save_account(&db, "imap", "bulk", "bulkpass", false);
        // 初始同步只拉最近 50 封（INITIAL_WINDOW）；开启历史回填设置后按轮回拉全部。
        crate::imap_settings::save(
            &db.lock().unwrap(),
            &account,
            &crate::imap_settings::Settings {
                sync_all_folders: false,
                history_enabled: true,
                auto_sync_seconds: 0,
            },
        )
        .unwrap();
        // 测试重跑自包含：先清空上一轮注入的邮件（IMAP 会话独立核对通道）。
        purge_mailbox("bulk", "bulkpass");
        let tag = marker();
        let total = 2_000usize;
        let mut messages = Vec::with_capacity(total);
        for index in 0..total {
            messages.push(format!(
                "From: bulk@{DOMAIN}\r\nTo: bulk@{DOMAIN}\r\nSubject: {tag} 大邮箱 {index:05}\r\nMessage-ID: <{tag}-{index}@{DOMAIN}>\r\nDate: Sat, 7 Feb 2026 13:00:00 +0000\r\n\r\n{tag} 第 {index} 封正文，带少量文本保证真实解析开销。"
            ));
        }
        let inject_started = std::time::Instant::now();
        // 分批会话注入，避免单条连接过载。
        for chunk in messages.chunks(250) {
            raw_send("bulk", "bulkpass", chunk);
        }
        let injected = inject_started.elapsed();
        let started = std::time::Instant::now();
        // IMAP 同步按游标分轮推进，循环直到拉完。
        let mut fetched_total = 0usize;
        for _ in 0..200 {
            let outcome = sync(&db, &account);
            fetched_total += outcome.fetched_messages;
            if fetched_total >= total {
                break;
            }
        }
        let elapsed = started.elapsed();
        assert!(
            fetched_total >= total,
            "应拉取全部注入邮件：{fetched_total}"
        );
        let counted = inbox_subjects(&db, &account)
            .iter()
            .filter(|(_, subject)| subject.contains(&tag))
            .count();
        assert_eq!(counted, total, "注入邮件应全部入库");
        let second = sync(&db, &account);
        assert_eq!(second.fetched_messages, 0, "增量同步应为 0");
        eprintln!("[吞吐] 注入 {total} 封：{injected:?}；全量同步：{elapsed:?}；二次增量：0 封");
        cleanup(&db, &[account]);
    }
}
