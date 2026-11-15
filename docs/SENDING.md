# 发送与 outbox 设计（SMTP）

状态：2026-11-15 已实现（既有真实 Gmail 发送已验收；本轮新增抄送 / 密送已自动验证，真实投递待验收）。核心铁律继承 AGENTS / STATUS：**发送结果不明时不重复发送，不把任何中间状态伪装成成功**。

## 目标

完成「收信 / 回复 / 发信 / 重启恢复 / 离线阅读」的闭环（ROADMAP 阶段 A 验收）：本地草稿可经真实 SMTP 发出；发送全程持久化，重启续跑；结果不明有明确语义与出口。

## 范围与提供商能力

- 密码账号：SMTP TLS 直连（465）与 STARTTLS（587）。SMTP STARTTLS 路径此前仅做过连接验证，实现时补全与测试。
- Gmail OAuth：已申请 `https://mail.google.com/`，XOAUTH2 可用于 smtp.gmail.com:465 / 587，无需新增 scope。
- Microsoft OAuth：当初刻意未申请 `SMTP.Send`（OAUTH_SETUP 约定），Open 后发送按钮禁用并说明原因；注册应用补 scope 前不得伪装可用。
- 撰写支持纯文本 / HTML 与 To / Cc / Bcc；附件 / 图片发送待后续，详见 COMPOSING。

## 收件人隔离（2026-11-15）

草稿 v12 保存独立 cc / bcc 字段，登记与 worker 快照均检查三栏地址。SMTP RCPT 使用合并去重的完整信封；MIME 只写 To / Cc，密送地址不写入头部，仅密送时 To 为 undisclosed-recipients。空主题在 UI 明确确认，服务端仍允许。字段长度、地址编辑与回归见 COMPOSING。纯文本输出明确 base64 传输编码且只含一次 Content-Type；HTML 使用 multipart/alternative。

## 回复会话关联（2026-11-15）

- 回复目标规则见 COMPOSING；发送侧同步补全 RFC 5322 会话头：worker 快照读取草稿 `reply_to_id`（本机邮件 ID），在同账号 messages 查原邮件的 `header_message_id` 与 `header_references`（迁移 013，IMAP / POP3 解析时落库）。
- 组装时 `In-Reply-To` = 原邮件 Message-ID；`References` = 原 References 链逐条规范化（mail-parser 存裸 id，统一补尖括号）后追加原 Message-ID，原链已含该 ID 不重复；超过 900 字节按空格折行（RFC 5322 单行上限 998）。
- Message-ID 写入前中和 CR / LF（防头部注入）；缺尖括号补齐。
- 诚实降级：原邮件已删除、查询失败或原邮件没有 Message-ID 时不写两个关联头，不伪造会话关联，也不阻塞发送。
- 未回复草稿（新建 / 转发）不写 In-Reply-To / References；发件人自身 Reply-To 头暂不提供（账号无别名场景）。真实收件端（Gmail / Outlook）的会话归组验收待执行。

## Outbox 状态机

本地草稿（drafts）仍是唯一编辑入口；「发送」把草稿登记进 outbox 队列，草稿本体保留为已发送副本关联。

```
composing(草稿) --发送--> queued --> sending --> sent
                                    |---> unknown      (网络中断 / 超时 / 无法判定)
                                    |---> failed       (明确拒绝：5xx、认证失败、收件人拒绝)
queued/sending --> cancelled        (仅排队中可取消；sending 不可中断取消)
unknown --[用户明确确认]--> sent | failed      （确认保留本机 / 确认未发出可重试）
unknown --[下次同步核对]--> resolved-sent      (Sent 目录出现相同 Message-ID)
failed --[手动重试]--> queued                  (明确拒绝错误不自动重试；网络类错误按持久退避)
```

规则：

1. **queued / sending 持久化**：v10 迁移新增 `outbox` 表（id、account_id、draft_id、message_id、state、attempts、last_error、queued_at、updated_at、resolved_by）。重启后 queued / unknown / failed 恢复；sending 中断按 unknown 处理（见下）。
2. **Message-ID 幂等**：登记 outbox 时生成一次 `Message-ID`，此后所有尝试复用同一值；服务器端去重与 Sent 核对都依赖它。发送前不改动草稿正文与收件人。
3. **unknown 的判定**：仅当「DATA 阶段已提交点号但未收到 / 未读到最终 2xx」或连接在发送中途丢失、超时且无法区分是否已投递时。命令阶段（EHLO / AUTH / MAIL FROM / RCPT TO）失败或被拒是明确 failed，不是 unknown。
4. **unknown 不自动重试、不自动标失败**：UI 展示「结果不明」与证据（尝试次数、最后错误码）；用户可明确选择「保留本机（不再发送）」或「确认未送达，重试」。同步时若 Sent 目录出现相同 Message-ID，自动转 resolved-sent（Sent 缺失不构成未送达证据，不自动回滚）。
5. **明确拒绝**（5xx、认证拒绝、收件人不存在）：failed，不自动重试；界面给错误码与「修改后重试」入口。
6. **队列复用账号调度**：发送是账号队列的一种任务，与同步共用同账号互斥与全局两 worker；网络 / SMTP 交互不占 DB 锁（沿用短锁快照 → 锁外网络 → 短事务提交）。发送任务与同步任务按登记顺序执行，同账号先同步后发送需等待，不插队。
7. **已发送副本**：发送成功后不本地伪造 Sent 邮件；依赖服务器保存的 Sent 目录经同步进入本地（UID 游标已覆盖）。服务器未保存 Sent 副本时，本地仅以 outbox 的 resolved-sent 记录展示，不合成假邮件。
8. **凭据**：密码走系统凭据库（现有 keyring 通道）；OAuth access token 仅内存、按账号 / generation 隔离，与 IMAP 复用同一刷新逻辑。

## IPC（草案）

| 命令 | 输入 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mail_send_draft` | draftId, revision | outbox 条目 | 校验草稿 revision 与账号；生成 Message-ID；入队 |
| `mail_outbox_list` | - | 条目数组 | 各账号发送状态汇总 |
| `mail_outbox_cancel` | id | 条目 | 仅 queued |
| `mail_outbox_resolve` | id, action | 条目 | unknown → 明确认定（仅保留本机 / 确认重试） |
| `mail_retry_outbox` | id | 条目 | failed → queued |

跨 IPC 结构变更同步 Rust 与 TypeScript；outbox 条目字段以 domain 层类型为准。

## UI（草案）

- 撰写页脚：桌面版且账号支持 SMTP 时显示「发送」；否则按钮禁用并说明（浏览器演示不可发送、Microsoft 未授权 SMTP.Send）。
- 阅读区 / 账号页：outbox 状态徽标（排队中 / 发送中 / 已发送 / 结果不明 / 发送失败），结果不明提供两个明确出口按钮。
- 发送失败 / 结果不明不删除草稿；「已发送」草稿保留在草稿箱并标记，直到服务器 Sent 同步进入本地。

## 测试策略

- 逐命令 SMTP 脚本会话（沿用 IMAP MockStream 模式）：正常投递、命令阶段拒绝、DATA 后断线（unknown）、超时（unknown）、AUTH 失败（failed）、STARTTLS 升级、TLS 465、XOAUTH2 挑战。
- 网络步骤中验证 DB 未持锁；outbox 落盘重开恢复 queued / unknown；Message-ID 复用断言；Sent 核对解析 unknown 的单测（命中 / 不命中两种）。
- e2e：发送按钮状态（演示账号禁用 + 说明）、排队 / 发送中徽标、unknown 的两个出口；IPC harness 扩展 outbox 命令。

## 真实验收（实现后）

1. 授权测试账号真实发送一封 → 收到；Sent 同步后本地可见。
2. 发送中重启进程 → 状态落盘为 unknown 或 queued，不重复投递。
3. 断网发送 → 结果不明 → 恢复网络后经 Sent 核对自动 resolved-sent。
4. 收件人拒绝 → failed，修改后手动重试成功。
5. Gmail OAuth 账号经 XOAUTH2 SMTP 发送成功；Microsoft 账号发送按钮保持禁用。

## 明确不做

- 附件 / 图片发送、群发调度、定时发送、S/MIME。
- 自动重试 unknown；本地合成 Sent 邮件；未配置签名发布的对外分发。
