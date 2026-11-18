# 同步与多账号调度

状态：2026-11-16。已完成现有 Gmail 的 25 个目录实际同步、约 25 MiB 邮件下载、大小差异与断点续跑验收；五万封以上邮箱吞吐与其他服务商仍未验收。完整范围见 [IMAP_COMPLETION](IMAP_COMPLETION.md)。

## 锁与提交

所有账号同步、状态回写、恢复页、原文重取统一进入 sync_scheduler：FIFO、两 worker、同账号互斥、最多 128 排队、保留 64 终态。队列锁只管理队列，DB Mutex 只读取快照、版本核对和短事务；禁止带锁等待网络、凭据、MIME、磁盘缓存或任务结果。

凭据按账号另设锁，串行刷新令牌、重配和删除；可在该锁内访问 keyring / 授权服务，不能持 DB 锁。会话池锁只取出 / 放回连接，socket 操作在锁外。缓存配额锁不跨网络请求。

每次提交核对账号 generation、expected cursor；字段与恢复还核对 revision。批次邮件、目录关联、游标和 UIDVALIDITY 清理同事务，SQL 失败整批回滚；后批失败保留先前已提交批次。MOVE 未知结果不自动重发。

## 持久任务与停止

v9 的 imap_sync_requests 保存账号代次、请求版本、状态 queued / running / failed / paused、来源 manual / auto / continuation、次数、due_at 和错误，不保存密码或网络连接。1 秒派发器只把到期请求交给同一 FIFO。启动时 running 回 queued，游标从最后成功批次继续；已停止 / 认证失败的 paused 不自动恢复。

- 手动同步登记 durable request 后入队；重复运行合并，新的手动请求递增 request_version。旧任务完成不能删除 / 暂停较新的请求，取消后立即重排也保留新请求。
- 网络失败从 5 秒开始指数退避，上限 900 秒，最多 8 次后暂停；认证、数据冲突、无效参数、用户停止不自动重试。次要目录失败也登记重试，不能仅用 warnings 假装全部完成。
- 尚有增量范围、历史范围、queued 恢复页或原文请求时，1 秒后公平续跑。首次恢复检查完成后停在 located，必须用户确认；确认授权后仍完整重扫。
- 自动同步默认关闭，设置保存到 imap_settings，开启后仅在应用运行期间生效。手动停止会 pause 整个账号，需手动同步重新开始；关闭自动同步不删除明确登记的恢复 / 下载任务。
- queued 可直接取消；running 标记 cancelRequested，实际 worker 在边界与输入读写时停止，待释放后进入 cancelled。已提交批次保留，不能把请求已接受当成连接立即关闭。
- worker 从实际开始计 300 秒预算，单 IMAP 命令 60 秒，socket 10 秒，STARTTLS 明文握手 30 秒、连接地址最多 8 个。系统 DNS / 平台 TLS 调用不能强制即时中断；停止可能等待当前阻塞返回。
- runner 异常被隔离，durable request 暂停并允许手动重试；工作线程继续服务其他账号。

设置页关闭不停止同步；退出进程不保留 socket，重启恢复 durable request。未知归档恢复仍由 message_changes::recover 保持 unknown，不会因请求恢复重发 MOVE。

## 全目录轮次与有界续跑（2026-11-16）

v14 新增 imap_round_progress / imap_flag_progress，只记录同步工作，不改既有邮件、草稿和原游标。轮次以 accountId / generation / request_version 保护，目标键包含目录作用域及原始目录名摘要。完整目录成功后从待处理队列移除，仍有增量的目录排到队尾；新目录加入、不可访问目录移出，重启后续跑不再反复处理前面的已完成目录。

每个 worker 至多处理 8 个目录；在协议操作的安全边界预留 75 秒给下一条命令和提交，主动让出后交给原有队列继续。300 秒任务 / 60 秒命令硬上限、取消、账号队列和网络限额保持不变。网络异常后停止使用该连接，失败目录保留并交给退避；未完整提交的批次不推进游标。

新邮件索引仍最多查询 100 数值 UID，但正文按约 4 MiB / 最多 10 封划分提交批次，单封较大邮件独立下载且仍遵守 64 MiB 上限。返回明确的 through_uid，游标只到第一个延期 UID 之前；范围末端、UIDVALIDITY 和 UID 集合核对仍保留。历史回填保持完整固定范围语义，不把不完整前缀当成历史范围完成。

FLAGS 以每 100 个缓存 UID 的短事务保存进度，与已读 / 星标观察同事务，复用时核对账号代次、请求版本与 UIDVALIDITY；续查不重扫已完成前缀。跨 worker 的标记核对保留较旧 MODSEQ，不把部分观察升级为完整水位。新手动请求或重配使旧轮次不能覆盖、清理较新进度。

可选开发诊断环境变量 AUUMAIL_SYNC_TRACE=1 仅输出目录摘要、阶段及计数 / 字节量，不输出账号、目录显示名、邮件正文或凭据；正式 release 不输出。

## 目录、历史和远端变化

新账号和 v8 升级的真实账号默认同步全部可访问目录；可关闭后仅同步基础目录。Noselect 排除；特殊用途优先识别唯一 Archive，歧义不移动。原始目录名用于协议和定位，modified UTF-7 只解码显示，改名 / 不可访问目录保留本机并标不可用，不猜别名。

INBOX / 原 Sent / Archive 保持兼容作用域，其他目录使用原始名 SHA-256 的 folder-* 作用域。UID 只在 account + generation + folder + UIDVALIDITY 内有意义。UI mailbox 是单分类投影；folderIds 保留具体目录关联。Gmail 各标签目录的副本不按 Message-ID 自动合并。

首次下载最近 50 数值 UID 范围，增量每个索引范围最多 100 UID、正文按约 4 MiB / 最多 10 封提交，每目录每任务最多 20 批。历史默认关闭，开启后每任务每目录向前补一个 100 UID 范围；imap_history 独立保存 before_uid，不倒退增量游标。历史提交后检查点失败最多重复读取，不跳过邮件。

已知 UID 的 FLAGS 每组最多 100；缺失响应必须再 UID SEARCH 精确集合并复核 UIDVALIDITY，确认不存在才标 remoteMissing。保留正文、本地意图及引用；编辑竞争使旧结果失效。外部移动通过用户登记的可信指纹跨目录核对恢复，普通同步不自动重绑。

连接 / OAuth 重新配置每次更换 authEpoch，旋转 generation 并重建游标。相同数值 UID 的旧缓存另存内部 retired ID，original_remote_id 保留旧定位；旧 ID、正文与意图仍在，待回写冲突。新邮件独立建行，不能继承旧账号修改。恢复中的缓存先保护、不退休、不覆盖。

## 有界下载与协议优化

下载先取 SIZE、分组不超过 8 MiB；大于 8 MiB 原文使用精确 256 KiB BODY.PEEK 偏移、临时文件和完整 SHA-256，最大 64 MiB，完整后解析 / 提交。命令输入限额、512 MiB 缓存及附件约定见 [IMAP_LIMITS](IMAP_LIMITS.md)。明确原文重取也进入账号 worker，CAS 保存缓存，不能升级弱历史指纹来源。

CAPABILITY 支持 IDLE 时，自动同步使用最多 5 秒通知片段 + DONE，再释放 worker 公平轮转；连接池最多 64 账号、按 generation 隔离，复用时重新绑定当前任务取消令牌。无 IDLE 或被 NO / BAD 拒绝时按设置轮询；网络 / 不完整响应丢弃连接。通知只监测 INBOX，其他目录按定时完整轮次核对；不是所有目录持续推送。纯通知轮次不重新加载工作区，最近完整轮次之后的只读续页不重复全箱 FLAGS。

CONDSTORE 可用时保存每目录 generation / UIDVALIDITY / HIGHESTMODSEQ，使用 CHANGEDSINCE；空增量仍 SEARCH 核对 UID 存在性。能力拒绝、缺水位 / NOMODSEQ / 水位倒退回全量 FLAGS。最终 EXAMINE 复核后仅保存开始水位，本机编辑或恢复保护抑制标记应用时删除水位，下次完整补读。没有 QRESYNC、跨目录事务快照或基于 MODSEQ 放宽绑定证据。

## IPC 与 UI

2026-11-16：POP3 与 IMAP 共用设置页单账号 / 同步全部入口、持久账号队列和自动同步间隔。POP3 设置只开放自动间隔，后端允许真实 POP3 保存并拒绝多目录 / IMAP 历史选项；读取时投影这两项为 false，兼容旧默认值，无迁移。POP3 按 UIDL 下载尚未见过的服务器邮件到单一收件箱，保留服务器副本；不提供服务器标记 / 文件夹同步。连接重配从 Rust 配置返回到向导均保留 provider=pop3，仅提供现有 POP3S TLS 能力，不把账号变成 IMAP。真实 POP3 服务端验收仍未执行。

原生验收补充：手动 IPC 的 `validate_accounts` 必须按数据库 provider 分流。IMAP 使用 `prepare`，POP3 与 worker 共享纯配置校验；混合批次中的 POP3 不读取 IMAP 游标，不把 IMAP `prepare` 当成统一账号门控。只读校验不做网络 / keyring I/O，全部校验通过后才登记 durable request。原生独立资料已验证自动间隔保存 / 重开、POP3 单账号与混合同步全部确实进入后端；合成无凭据失败不等于真实服务器成功。

mail_enqueue_sync / mail_sync_jobs / mail_cancel_sync 返回带 revision 的快照；mail_sync_account 兼容入口也经同一队列。Job 包含 cancelRequested、起止时间、outcome / error。outcome 的 hasMore、moreMail、listening、retryNeeded、refreshNeeded 区分续跑、通知、部分失败和是否需要刷新。

syncStore 在 App 层活动每 750ms、空闲每 5 秒读进度，发现后台任务；旧 revision 不覆盖新状态。每个需要刷新的终态只刷新一次，失败 / 运行中停止也刷新已提交批次，排队取消和纯 IDLE 不刷新。后台 reload 合并草稿 revision / 邮件 stateRevision，不丢期间编辑；刷新失败可重读本机结果，不重新发网络动作。

## 静默同步（2026-11-19）

自动同步 / IDLE 的空轮次对界面完全不可见：`SyncOutcome.refresh_needed` 为诚实语义——只有**拉取到新邮件**（fetched_messages > 0）或**确认了本机回写**（flush 确认数 > 0，含 EXPUNGE / MOVE / 标记 ack 与冲突 / unknown 状态写入）才为 true；`imap_writeback::flush` 返回 (warnings, 确认数)。恢复保护暂停、并发更新抢输、applying 转移失败的跳过不计入。前端 syncStore 另按账号比对 outcome.folders 与上一轮值——远端目录结构变化（0 拉取轮次）同样触发刷新；本机目录 CRUD 自带即时刷新，不经此路径。运行中的任务不再按固定间隔重建工作区（移除原 2 秒节流渐进刷新）：新邮件在轮次终态进入列表（长回填按 has_more 分轮，每轮终态即刷新），阅读界面在无真实变化时零重渲染。失败任务的 outcome 为空，仍按可刷新处理（部分提交的批次需要上屏）。

**监听不是"同步中"（2026-11-19）**：IDLE 账号的任务按 5 秒监听片段 + 1 秒续跑无限循环（新邮件到达时 dirty 立即转真实同步轮）——队列快照为这类运行标记 `listening`（live 会话注册表 `IDLING` 置位，真实同步轮清除，`forget` 注销）。前端转圈（顶栏 / 列表工具栏）与"同步中"摘要只对真实同步阶段生效；监听阶段摘要显示"已连接 · 实时监听中…"，同步按钮的禁用逻辑不变（队列语义）。

mail_imap_settings / mail_save_imap_settings 读写账号选项；mail_request_body 只登记原文请求；mail_export_attachment 调系统保存窗口。OAuth 生命周期与连接重配见 [OAUTH_SETUP](OAUTH_SETUP.md)。

## 测试与维护

运行 npm run check、npm run check:native、npm run test:ui。脚本覆盖 DB try_lock、两账号队列、取消 / 重排竞争、generation / revision、事务回滚、v8 升级 / 重开、密集分段、跨目录重复、IDLE DONE / 不完整行、CONDSTORE 回退、17 MiB 分块、配额与 keyring 补偿。浏览器仅 test-only IPC harness，不等同原生 WebView / 真实服务端成功。当前前端 gateway 仍拼接全部分页结果，大邮箱列表虚拟化和按视图加载属于后续 UI 性能工程。
