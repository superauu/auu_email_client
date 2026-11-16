use super::*;

pub(super) fn key(kind: &str, remote: &str) -> String {
    format!("{kind}:{}", crate::mail_recovery::hash(remote.as_bytes()))
}

pub(super) struct Round {
    targets: Vec<String>,
    pub pending: Vec<String>,
}

fn guard(conn: &Connection, plan: &SyncPlan) -> Result<(), AppError> {
    ensure_current(conn, plan)?;
    let version: Option<i64> = conn
        .query_row(
            "SELECT request_version FROM imap_sync_requests WHERE account_id=?1",
            [&plan.account_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::storage("无法核对目录轮次"))?;
    if version != plan.request_version {
        return Err(AppError::conflict("同步请求已更新，旧目录轮次未覆盖新请求"));
    }
    Ok(())
}

pub(super) fn flag_position(
    db: &SharedDb,
    plan: &SyncPlan,
    key: &str,
    validity: u32,
) -> Result<u32, AppError> {
    let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
    guard(&conn, plan)?;
    conn.query_row("SELECT last_uid FROM imap_flag_progress WHERE account_id=?1 AND folder_key=?2 AND generation=?3 AND request_version=?4 AND uid_validity=?5",params![plan.account_id,key,plan.generation,plan.request_version.unwrap_or(0),validity],|r|r.get(0)).optional().map(|v|v.unwrap_or(0)).map_err(|_|AppError::storage("无法读取标记核对进度"))
}

pub(super) fn save_flags(
    conn: &Connection,
    plan: &SyncPlan,
    key: &str,
    validity: u32,
    uid: u32,
) -> Result<(), AppError> {
    guard(conn, plan)?;
    conn.execute("INSERT INTO imap_flag_progress VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(account_id,folder_key) DO UPDATE SET generation=excluded.generation,request_version=excluded.request_version,uid_validity=excluded.uid_validity,last_uid=excluded.last_uid",params![plan.account_id,key,plan.generation,plan.request_version.unwrap_or(0),validity,uid]).map_err(|_|AppError::storage("无法保存标记核对进度"))?;
    Ok(())
}

impl Round {
    pub fn load(
        db: &SharedDb,
        plan: &SyncPlan,
        targets: &[(String, String)],
    ) -> Result<Self, AppError> {
        let saved: Option<(String, String)> = {
            let conn = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
            guard(&conn, plan)?;
            conn.query_row("SELECT targets_json,pending_json FROM imap_round_progress WHERE account_id=?1 AND generation=?2 AND request_version=?3", params![plan.account_id,plan.generation,plan.request_version.unwrap_or(0)], |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| AppError::storage("无法读取目录轮次"))?
        };
        let current: Vec<_> = targets
            .iter()
            .map(|(kind, remote)| key(kind, remote))
            .collect();
        let mut round = if let Some((all, pending)) = saved {
            let parse = |raw: &str| {
                serde_json::from_str::<Vec<String>>(raw)
                    .map_err(|_| AppError::storage("目录轮次数据损坏，未重置邮件游标"))
            };
            Self {
                targets: parse(&all)?,
                pending: parse(&pending)?,
            }
        } else {
            Self {
                targets: current.clone(),
                pending: current.clone(),
            }
        };
        round.pending.retain(|key| current.contains(key));
        for key in &current {
            if !round.targets.contains(key) {
                round.pending.push(key.clone());
            }
        }
        round.targets = current;
        round.save(db, plan, None)?;
        Ok(round)
    }

    pub fn finished(
        &mut self,
        db: &SharedDb,
        plan: &SyncPlan,
        key: &str,
        more: bool,
    ) -> Result<(), AppError> {
        self.pending.retain(|pending| pending != key);
        if more {
            self.pending.push(key.to_owned());
        }
        self.save(db, plan, (!more).then_some(key))
    }

    fn save(
        &self,
        db: &SharedDb,
        plan: &SyncPlan,
        completed: Option<&str>,
    ) -> Result<(), AppError> {
        // Serialize before taking the database lock; no protocol I/O while locked.
        let all = serde_json::to_string(&self.targets)
            .map_err(|_| AppError::storage("目录轮次无法保存"))?;
        let pending = serde_json::to_string(&self.pending)
            .map_err(|_| AppError::storage("目录轮次无法保存"))?;
        let mut connection = db.lock().map_err(|_| AppError::storage("数据库忙"))?;
        let conn = connection
            .transaction()
            .map_err(|_| AppError::storage("目录轮次事务不可用"))?;
        guard(&conn, plan)?;
        if let Some(key) = completed {
            conn.execute("DELETE FROM imap_flag_progress WHERE account_id=?1 AND folder_key=?2 AND generation=?3 AND request_version=?4",params![plan.account_id,key,plan.generation,plan.request_version.unwrap_or(0)]).map_err(|_|AppError::storage("无法完成标记进度"))?;
        }
        if self.pending.is_empty() {
            conn.execute("DELETE FROM imap_flag_progress WHERE account_id=?1 AND generation=?2 AND request_version=?3",params![plan.account_id,plan.generation,plan.request_version.unwrap_or(0)]).map_err(|_|AppError::storage("无法完成标记进度"))?;
            conn.execute("DELETE FROM imap_round_progress WHERE account_id=?1 AND generation=?2 AND request_version=?3", params![plan.account_id,plan.generation,plan.request_version.unwrap_or(0)]).map_err(|_| AppError::storage("目录轮次无法完成"))?;
        } else {
            conn.execute("INSERT INTO imap_round_progress VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account_id) DO UPDATE SET generation=excluded.generation,request_version=excluded.request_version,targets_json=excluded.targets_json,pending_json=excluded.pending_json",params![plan.account_id,plan.generation,plan.request_version.unwrap_or(0),all,pending]).map_err(|_| AppError::storage("目录轮次无法保存"))?;
        }
        conn.commit()
            .map_err(|_| AppError::storage("目录轮次未提交"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{db, plan};
    use super::*;

    fn targets() -> Vec<(String, String)> {
        ["A", "B", "C"]
            .iter()
            .map(|name| (crate::imap_settings::scope(name), name.to_string()))
            .collect()
    }

    #[test]
    fn unfinished_folders_and_flag_position_survive_database_reopen() {
        let db = db();
        let plan = plan(&db);
        let targets = targets();
        let mut progress = Round::load(&db, &plan, &targets).unwrap();
        progress
            .finished(&db, &plan, &key(&targets[0].0, "A"), false)
            .unwrap();
        let flag_key = key(&targets[1].0, "B");
        save_flags(&db.lock().unwrap(), &plan, &flag_key, 5, 100).unwrap();
        progress.finished(&db, &plan, &flag_key, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("restart.db");
        db.lock()
            .unwrap()
            .execute("VACUUM INTO ?1", [file.to_str().unwrap()])
            .unwrap();
        let reopened = Arc::new(Mutex::new(crate::storage::initialize(&file).unwrap()));
        let resumed = Round::load(&reopened, &plan, &targets).unwrap();
        assert_eq!(
            resumed.pending,
            vec![key(&targets[2].0, "C"), flag_key.clone()]
        );
        assert_eq!(flag_position(&reopened, &plan, &flag_key, 5).unwrap(), 100);
        assert_eq!(flag_position(&reopened, &plan, &flag_key, 6).unwrap(), 0);
    }

    #[test]
    fn changed_directory_list_adds_new_targets_without_repeating_completed_ones() {
        let db = db();
        let plan = plan(&db);
        let mut targets = targets();
        let mut progress = Round::load(&db, &plan, &targets).unwrap();
        progress
            .finished(&db, &plan, &key(&targets[0].0, "A"), false)
            .unwrap();
        targets.remove(1);
        targets.push((crate::imap_settings::scope("D"), "D".into()));
        let mut resumed = Round::load(&db, &plan, &targets).unwrap();
        assert_eq!(
            resumed.pending,
            vec![key(&targets[1].0, "C"), key(&targets[2].0, "D")]
        );
        for target in &targets[1..] {
            resumed
                .finished(&db, &plan, &key(&target.0, &target.1), false)
                .unwrap();
        }
        assert!(!super::super::tests::plan(&db).resume_round);
    }

    #[test]
    fn old_round_cannot_clear_a_new_request_or_a_reconfigured_accounts_progress() {
        let db = db();
        crate::imap_jobs::request(&mut db.lock().unwrap(), &["acc".into()]).unwrap();
        let old = plan(&db);
        let targets = targets();
        let mut progress = Round::load(&db, &old, &targets).unwrap();
        crate::imap_jobs::request(&mut db.lock().unwrap(), &["acc".into()]).unwrap();
        let current = plan(&db);
        let next = Round::load(&db, &current, &targets).unwrap();
        assert!(progress
            .finished(&db, &old, &key(&targets[0].0, "A"), false)
            .is_err());
        assert_eq!(
            Round::load(&db, &current, &targets).unwrap().pending,
            next.pending
        );
        db.lock()
            .unwrap()
            .execute(
                "UPDATE accounts SET email='new@example.test' WHERE id='acc'",
                [],
            )
            .unwrap();
        assert!(Round::load(&db, &current, &targets).is_err());
    }
}
