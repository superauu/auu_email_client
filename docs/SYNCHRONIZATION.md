# 同步与多账号调度

状态：2026-11-11，A3.8。代码与脚本回归已接入；真实服务器与大邮箱性能未验收。完整范围见 [IMAP_COMPLETION](IMAP_COMPLETION.md)。

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

## 目录、历史和远端变化

新账号和 v8 升级的真实账号默认同步全部可访问目录；可关闭后仅同步基础目录。Noselect 排除；特殊用途优先识别唯一 Archive，歧义不移动。原始目录名用于协议和定位，modified UTF-7 只解码显示，改名 / 不可访问目录保留本机并标不可用，不猜别名。

INBOX / 原 Sent / Archive 保持兼容作用域，其他目录使用原始名 SHA-256 的 folder-* 作用域。UID 只在 account + generation + folder + UIDVALIDITY 内有意义。UI mailbox 是单分类投影；folderIds 保留具体目录关联。Gmail 各标签目录的副本不按 Message-ID 自动合并。

首次下载最近 50 数值 UID 范围，增量每范围最多 100 UID、每目录每任务最多 20 范围。历史默认关闭，开启后每任务每目录向前补一个 100 UID 范围；imap_history 独立保存 before_uid，不倒退增量游标。历史提交后检查点失败最多重复读取，不跳过邮件。

已知 UID 的 FLAGS 每组最多 100；缺失响应必须再 UID SEARCH 精确集合并复核 UIDVALIDITY，确认不存在才标 remoteMissing。保留正文、本地意图及引用；编辑竞争使旧结果失效。外部移动通过用户登记的可信指纹跨目录核对恢复，普通同步不自动重绑。

连接 / OAuth 重新配置每次更换 authEpoch，旋转 generation 并重建游标。相同数值 UID 的旧缓存另存内部 retired ID，original_remote_id 保留旧定位；旧 ID、正文与意图仍在，待回写冲突。新邮件独立建行，不能继承旧账号修改。恢复中的缓存先保护、不退休、不覆盖。

## 有界下载与协议优化

下载先取 SIZE、分组不超过 8 MiB；大于 8 MiB 原文使用精确 256 KiB BODY.PEEK 偏移、临时文件和完整 SHA-256，最大 64 MiB，完整后解析 / 提交。命令输入限额、512 MiB 缓存及附件约定见 [IMAP_LIMITS](IMAP_LIMITS.md)。明确原文重取也进入账号 worker，CAS 保存缓存，不能升级弱历史指纹来源。

CAPABILITY 支持 IDLE 时，自动同步使用最多 5 秒通知片段 + DONE，再释放 worker 公平轮转；连接池最多 64 账号、按 generation 隔离，复用时重新绑定当前任务取消令牌。无 IDLE 或被 NO / BAD 拒绝时按设置轮询；网络 / 不完整响应丢弃连接。通知只监测 INBOX，其他目录按定时完整轮次核对；不是所有目录持续推送。纯通知轮次不重新加载工作区，最近完整轮次之后的只读续页不重复全箱 FLAGS。

CONDSTORE 可用时保存每目录 generation / UIDVALIDITY / HIGHESTMODSEQ，使用 CHANGEDSINCE；空增量仍 SEARCH 核对 UID 存在性。能力拒绝、缺水位 / NOMODSEQ / 水位倒退回全量 FLAGS。最终 EXAMINE 复核后仅保存开始水位，本机编辑或恢复保护抑制标记应用时删除水位，下次完整补读。没有 QRESYNC、跨目录事务快照或基于 MODSEQ 放宽绑定证据。

## IPC 与 UI

mail_enqueue_sync / mail_sync_jobs / mail_cancel_sync 返回带 revision 的快照；mail_sync_account 兼容入口也经同一队列。Job 包含 cancelRequested、起止时间、outcome / error。outcome 的 hasMore、moreMail、listening、retryNeeded、refreshNeeded 区分续跑、通知、部分失败和是否需要刷新。

syncStore 在 App 层活动每 750ms、空闲每 5 秒读进度，发现后台任务；旧 revision 不覆盖新状态。每个需要刷新的终态只刷新一次，失败 / 运行中停止也刷新已提交批次，排队取消和纯 IDLE 不刷新。后台 reload 合并草稿 revision / 邮件 stateRevision，不丢期间编辑；刷新失败可重读本机结果，不重新发网络动作。

mail_imap_settings / mail_save_imap_settings 读写账号选项；mail_request_body 只登记原文请求；mail_export_attachment 调系统保存窗口。OAuth 生命周期与连接重配见 [OAUTH_SETUP](OAUTH_SETUP.md)。

## 测试与维护

运行 npm run check、npm run check:native、npm run test:ui。脚本覆盖 DB try_lock、两账号队列、取消 / 重排竞争、generation / revision、事务回滚、v8 升级 / 重开、密集分段、跨目录重复、IDLE DONE / 不完整行、CONDSTORE 回退、17 MiB 分块、配额与 keyring 补偿。浏览器仅 test-only IPC harness，不等同原生 WebView / 真实服务端成功。当前前端 gateway 仍拼接全部分页结果，大邮箱列表虚拟化和按视图加载属于后续 UI 性能工程。
