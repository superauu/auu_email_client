# 发送与 outbox 设计（SMTP）

状态：2026-11-16 已实现；真实 Gmail 自发自收附件下载与回复会话归组已验收，抄送 / 密送独立收件与其他客户端兼容仍待验收，中文附件名严格 MIME 参数解析存在待修问题（见下）。核心铁律继承 AGENTS / STATUS：**发送结果不明时不重复发送，不把任何中间状态伪装成成功**。

## 目标

完成「收信 / 回复 / 发信 / 重启恢复 / 离线阅读」的闭环（ROADMAP 阶段 A 验收）：本地草稿可经真实 SMTP 发出；发送全程持久化，重启续跑；结果不明有明确语义与出口。

## 范围与提供商能力

- 密码账号：SMTP TLS 直连（465）与 STARTTLS（587）。SMTP STARTTLS 路径此前仅做过连接验证，实现时补全与测试。
- Gmail OAuth：已申请 `https://mail.google.com/`，XOAUTH2 可用于 smtp.gmail.com:465 / 587，无需新增 scope。
- Microsoft OAuth：当初刻意未申请 `SMTP.Send`（OAUTH_SETUP 约定），Open 后发送按钮禁用并说明原因；注册应用补 scope 前不得伪装可用。
- 撰写支持纯文本 / HTML 与 To / Cc / Bcc；附件发送已实现（2026-11-16，见下），图片发送（正文内联）待后续，详见 COMPOSING。

## 收件人隔离（2026-11-15）

草稿 v12 保存独立 cc / bcc 字段，登记与 worker 快照均检查三栏地址。SMTP RCPT 使用合并去重的完整信封；MIME 只写 To / Cc，密送地址不写入头部，仅密送时 To 为 undisclosed-recipients。空主题在 UI 明确确认，服务端仍允许。字段长度、地址编辑与回归见 COMPOSING。纯文本输出明确 base64 传输编码且只含一次 Content-Type；HTML 使用 multipart/alternative。

## 撰写附件发送（2026-11-16）

- 登记（enqueue）读取 `drafts.attachments_json` 并重复校验限额（≤20 个、合计 16 MiB，与保存侧同一 `validate_attachments`）；元数据损坏按 storage 错误拒绝登记。
- worker 快照按元数据逐个从 `draft_attachment_blobs` 读取字节并核对 sha256 与大小；缺失或不一致报错（发送按 failed 语义落为明确失败），不发送不完整邮件。内容在短锁快照内读出，MIME 组装在锁外完成。
- 组装：有附件时在正文（text 或 multipart/alternative）外再包一层 `multipart/mixed`；附件为 base64 part，`Content-Disposition: attachment`，文件名 CR/LF 中和后 ASCII 安全字符用引号串、其余 RFC2047-B 编码，`name=` 与 `filename=` 同值；非法 media_type 的 token 字符白名单过滤后回退 `application/octet-stream`。无附件输出与既有格式逐字节一致。
- 总大小无客户端 SMTP SIZE 预检：服务器因限额拒绝按明确 failed 展示服务器错误，不伪装；本地保存侧的 16 MiB 限额先于发送拦截大部分超限。
- 铁律不变：Message-ID 复用、unknown 语义、不自动重试、同账号队列均与无附件发送一致。

## 回复会话关联（2026-11-15）

- 回复目标规则见 COMPOSING；发送侧同步补全 RFC 5322 会话头：worker 快照读取草稿 `reply_to_id`（本机邮件 ID），在同账号 messages 查原邮件的 `header_message_id` 与 `header_references`（迁移 013，IMAP / POP3 解析时落库）。
- 组装时 `In-Reply-To` = 原邮件 Message-ID；`References` = 原 References 链逐条规范化（mail-parser 存裸 id，统一补尖括号）后追加原 Message-ID，原链已含该 ID 不重复；超过 900 字节按空格折行（RFC 5322 单行上限 998）。
- Message-ID 写入前中和 CR / LF（防头部注入）；缺尖括号补齐。
- 诚实降级：原邮件已删除、查询失败或原邮件没有 Message-ID 时不写两个关联头，不伪造会话关联，也不阻塞发送。
- 未回复草稿（新建 / 转发）不写 In-Reply-To / References；发件人自身 Reply-To 头暂不提供（账号无别名场景）。2026-11-16 原生回复的关联头及 Gmail 网页实际归组通过自发自收验收；Outlook、回复全部与 Reply-To 列表地址仍待验收。

## Outbox 状态机

本地草稿（drafts）仍是唯一编辑入口；「发送」把草稿登记进 outbox 队列，草稿本体保留为已发送副本关联。

```
composing(草稿) --发送--> queued --> sending --> sent
                                    |---> unknown      (网络中断 / 超时 / 无法判定)
                                    |---> failed       (明确拒绝：5xx、认证失败、收件人拒绝)
queued --[取消发送]--> 本机草稿      (删除队列占用；sending 不可中断取消)
unknown --[仅保留本机]--> failed + keep-local  (投递仍未确认，禁止普通重试 / 返回草稿)
unknown --[明确确认未送达]--> queued            (Message-ID 不变)
unknown --[下次同步核对]--> sent + sent-folder  (Sent 目录出现相同 Message-ID)
failed --[手动重试]--> queued                  (仅明确失败，不自动重试)
failed --[返回草稿修改]--> 本机草稿             (原子解除占用，不修改草稿)
```

规则：

1. **queued / sending 持久化**：v10 迁移新增 `outbox` 表（id、account_id、draft_id、message_id、state、attempts、last_error、queued_at、updated_at、resolved_by）。重启后 queued / unknown / failed 恢复；sending 中断按 unknown 处理（见下）。
2. **Message-ID 幂等**：登记 outbox 时生成一次 `Message-ID`，此后所有尝试复用同一值；服务器端去重与 Sent 核对都依赖它。发送前不改动草稿正文与收件人。
3. **unknown 的判定**：仅当「DATA 阶段已提交点号但未收到 / 未读到最终 2xx」或连接在发送中途丢失、超时且无法区分是否已投递时。命令阶段（EHLO / AUTH / MAIL FROM / RCPT TO）失败或被拒是明确 failed，不是 unknown。
4. **unknown 不自动重试、不自动标失败**：UI 展示「结果不明」与证据（尝试次数、最后错误码）；用户可明确选择「保留本机（不再发送）」或「确认未送达，重试」。同步时若 Sent 目录出现相同 Message-ID，自动转 resolved-sent（Sent 缺失不构成未送达证据，不自动回滚）。
5. **明确拒绝**（5xx、认证拒绝、收件人不存在）：failed，不自动重试；界面给错误码与「修改后重试」入口。
6. **队列复用账号调度**：发送与同步共用同账号互斥与全局两 worker；网络 / SMTP 交互不占 DB 锁（沿用短锁快照 → 锁外网络 → 短事务提交）。账号 worker 先处理已登记的 outbox，再进入本轮同步；不会绕过队列并发连接同一账号。
7. **已发送副本**：发送成功后不本地伪造 Sent 邮件；依赖服务器保存的 Sent 目录经同步进入本地（UID 游标已覆盖）。服务器未保存 Sent 副本时，本地仅以 outbox 的 resolved-sent 记录展示，不合成假邮件。
8. **凭据**：密码走系统凭据库（现有 keyring 通道）；OAuth access token 仅内存、按账号 / generation 隔离，与 IMAP 复用同一刷新逻辑。

## 发送队列处理入口（2026-11-16）

设置 → 邮箱账号 → 发送队列，可按账号及状态筛选。显示全部未发送任务（包括旧 failed / unknown 与仅保留本机记录），加最近 128 条已发送记录；不再只展示每账号最近两条，也不让大量成功记录挤掉旧的待处理任务。主题与账号来自本机快照，错误长内容换行。

- queued：取消发送，删除队列占用，原草稿可继续编辑。
- sending：等待服务器结果，不提供中断取消。
- 明确 failed：手动重试复用原 Message-ID；或「返回草稿修改」，以单条条件 DELETE 原子核对状态并解除占用，草稿原账号 / revision / 收件人 / HTML / 附件字节保持不变。退回后定位本机草稿，关闭设置再点「编辑草稿」；不替换当前仍打开的撰写内容。后续显式发送属于新提交，生成新的 Message-ID。
- unknown：仅提供「仅保留本机」与「确认未送达并重试」，不能当作普通 failed 编辑或重试。
- 仅保留本机：底层沿用 failed + resolved_by=keep-local，但不是已证明发送失败；界面独立说明，后端也拒绝普通重试 / 返回草稿，防止绕过 unknown 保护。
- 操作期间按任务禁用重复动作；失败保留记录并显示错误，过期轮询不能恢复已取消 / 已退回的任务。未开放 SMTP 的账号禁用重试并说明。

## IPC

| 命令 | 输入 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mail_send_draft` | draftId, revision | outbox 条目 | 校验草稿 revision 与账号；生成 Message-ID；入队 |
| `mail_outbox_list` | - | 条目数组 | 各账号发送状态汇总 |
| `mail_outbox_cancel` | id | 条目 | 仅 queued |
| `mail_outbox_return_draft` | id | 原条目 | 仅明确 failed；原子解除发送占用，不修改草稿 |
| `mail_outbox_resolve` | id, action | 条目 | unknown → 明确认定（仅保留本机 / 确认重试） |
| `mail_retry_outbox` | id | 条目 | failed → queued |

跨 IPC 结构变更同步 Rust 与 TypeScript；outbox 条目字段以 domain 层类型为准。

## UI

- 撰写页脚：桌面版且账号支持 SMTP 时显示「发送」；否则按钮禁用并说明（浏览器演示不可发送、Microsoft 未授权 SMTP.Send）。
- 设置账号页：独立发送队列区显示排队中 / 发送中 / 已发送 / 结果不明 / 发送失败 / 仅保留本机，支持筛选、刷新与对应处理入口。
- 发送失败 / 结果不明不删除草稿；「已发送」草稿保留在草稿箱并标记，直到服务器 Sent 同步进入本地。

## 测试策略

- 逐命令 SMTP 脚本会话（沿用 IMAP MockStream 模式）：正常投递、命令阶段拒绝、DATA 后断线（unknown）、超时（unknown）、AUTH 失败（failed）、STARTTLS 升级、TLS 465、XOAUTH2 挑战。
- 网络步骤中验证 DB 未持锁；outbox 落盘重开恢复 queued / unknown；Message-ID 复用断言；Sent 核对解析 unknown 的单测（命中 / 不命中两种）。
- 附件组装回归（2026-11-16）：multipart/mixed 经 mail_parser 回读（中文名 / 注入字符 / 字节逐位一致 / text+html+附件 / 无附件不变）、登记限额与损坏元数据拒绝、快照缺失附件报错。
- e2e：发送按钮状态（演示账号禁用 + 说明）、排队 / 发送中徽标、unknown 的两个出口；IPC harness 扩展 outbox 命令。
- 队列处理回归：明确失败重试 / 返回草稿、queued 取消、unknown / keep-local 禁止绕过、旧待处理任务不被 128 条成功历史截掉；返回草稿保留账号 / revision / HTML / 抄送密送 / 附件字节。UI 覆盖账号筛选、失败保留任务、返回后继续编辑，store 覆盖重复操作与过期轮询。验证计数与真实验收边界见 STATUS。
- 2026-11-16 原生 Tauri / WebView2 独立资料手工通过：queued 取消、明确 failed 返回且可打开原草稿与附件、unknown 仅保留本机、重试复用 Message-ID 并按本机端口拒绝显示明确失败。真实工作区队列显示及待处理筛选通过；未对真实邮件点击发送或重试，不把本机拒绝路径当成成功投递验收。

## 真实验收（实现后）

1. 授权测试账号真实发送一封 → 收到；Sent 同步后本地可见。
2. 发送中重启进程 → 状态落盘为 unknown 或 queued，不重复投递。
3. 断网发送 → 结果不明 → 恢复网络后经 Sent 核对自动 resolved-sent。
4. 收件人拒绝 → failed，修改后手动重试成功。
5. Gmail OAuth 账号经 XOAUTH2 SMTP 发送成功；Microsoft 账号发送按钮保持禁用。
6. Gmail 附件投递（2026-11-16 自发自收通过）：170 字节中文名文本与 4096 字节二进制附件，在收件原文及 Gmail 网页实际下载后均与原文件逐字节 / SHA-256 一致；两封原生发送（原邮件、回复）各 attempts=1，均 sent，Inbox / Sent / All Mail 同步可见。

本轮 Gmail 网页原会话内实际显示原邮件与回复两封，下载的回复原文中 In-Reply-To / References 正确引用原邮件。Cc / Bcc 留空，因此不覆盖独立抄送密送收件或隐私；接近 16 MiB 的附件、其他提供商及上列故障场景不因本轮通过而视为已验收。详细记录见 STATUS。

**已发现待修的兼容性问题**：非 ASCII 文件名当前以未加引号的 RFC2047 编码参数输出，Gmail / mail_parser 可识别，但 Python `email` 的 `policy.default` 丢失该原文中的中文 filename / name 参数。需修正标准参数编码并补严格解析器回归；此次通过只证明 Gmail 中所测文件的显示与字节往返。

## 明确不做

- 图片发送（正文内联 / 粘贴图片）、群发调度、定时发送、S/MIME、EHLO SIZE 解析与 MAIL FROM SIZE= 参数。
- 自动重试 unknown；本地合成 Sent 邮件；未配置签名发布的对外分发。
