# 已读、星标与归档

状态：2026-11-11，A3.3 邮件状态基础与 A3.4 主动恢复均已实现并用脚本会话验证；恢复细节见 REMOTE_RECOVERY。尚未用真实服务器验收。浏览器演示只保存演示数据，不连接 IMAP。

## 用户行为与保存边界

- 点击邮件标为已读；阅读区可切换已读 / 未读、星标及归档。本地草稿不提供这些操作；服务器草稿副本仍按邮件标记处理。
- 桌面先在 SQLite 单事务保存状态和回写意图，成功后更新界面。离线无需网络；磁盘失败保留原状态并显示错误，不显示保存成功。
- 每封邮件的界面操作串行，连续切换从最后成功值计算；后台 reload 不覆盖期间完成的本机修改。账号之间独立。
- 真实 IMAP 邮件显示“本机已保存 · 等待同步此账号回写”。点击阅读区“同步此账号”或设置中的同步按钮，进入原有同账号互斥 / 全局两个 worker 队列。保存编辑不自动联网；可明确开启账号自动同步，之后后台处理。
- 演示账号的修改仅在本机：桌面存 SQLite，浏览器以 `auumail.demo.message-changes.v1` 保存已知演示邮件 ID 和布尔状态。桌面重新初始化演示种子不删除邮件或清空状态意图。
- 事件完成状态仍只在当前会话；此增量没有实现标签、删除、服务器草稿或日历回写。

## 数据与 IPC

追加迁移 `005_message_changes.sql`，不改写 v1–v4：

| 数据 | 用途 |
| --- | --- |
| messages.state_revision | 本机状态 / 回写状态 / 服务器标记更新的单调版本 |
| messages.remote_folder / remote_uid_validity / remote_uid / remote_generation | 经真实 FETCH 确认的 IMAP 定位，绑定账号连接代次 |
| message_changes | 每账号、邮件、字段一条持久意图，含 value、revision、state、源定位、可选目标与可公开错误 |

本机 ID 不因确认 MOVE 改变，事件引用保持有效；远端 ID 和定位更新为目标 UID。当前远端 ID 仍用 `imap:{kind}:{validity}:{uid}`，只同步 INBOX、首个 Sent 和唯一 Archive；完整任意目录 / Gmail 多标签映射仍待独立设计。不要将其扩展为仅 UID 唯一或用 Message-ID 猜测邮件身份。

`MailMessage` 增加 `stateRevision` 和 `changes[{field,state,error}]`；现有草稿投影为版本 0、空 changes。`MailGateway` 增加 updateMessage / keepMessageLocal。Tauri API 仅在 platform 中调用：

| 命令 | 输入 | 返回 |
| --- | --- | --- |
| mail_update_message | accountId、messageId、patch `{unread?,starred?,archive?:true}` | 保存后的 MailMessage |
| mail_keep_message_local | accountId、messageId | 停止这些修改的远端回写，返回本机投影 |

patch 必须非空，archive 只接受 true，未知字段拒绝；核对账号所有权，不操作草稿。归档通过 intent 覆盖 UI 的 mailbox 与分页文件夹筛选，确认 MOVE 后才改变真实 folder membership；保留数据库多文件夹结构。

## 状态与冲突规则

| 状态 | 含义 / 后续 |
| --- | --- |
| queued | 本机已保存；下次手动同步尝试 |
| applying | 已持久标记即将发送；本次任务处理结果 |
| failed | 标记或尚未发出 MOVE 的前置步骤失败；下次手动同步可重试 |
| conflict | 定位过期、账号改变、UID 消失、服务器拒绝预期标记、MOVE 能力 / 唯一目标不可确认；停止自动回写 |
| unknown | MOVE 已可能执行但结果 / 目标 UID 或本机确认不完整；禁止自动重试 |
| local | 仅本机覆盖；服务器观察值不会覆盖该字段，也不再回写 |

本地待回写字段优先。拉取正文、已有 UID 的 FLAGS 刷新均覆盖 intent；网络开始前读取 state_revision，旧 FLAGS 响应不得覆盖期间的新编辑。回写确认只清除自身字段的 revision，不能清除同字段后续修改或其他字段。确认前后核对账号 generation，旧账号响应不能作用于删除后重建的账号。

服务端不提供事务性条件写入时，不承诺跨客户端全序或 MODSEQ 冲突检测：本地未确认字段采用最后本机意图；确认后未来服务器 FLAGS 刷新可反映其他客户端的修改。

UIDVALIDITY 改变仍在成功拉取后与游标原子提交。无本地意图的旧行可清理；有意图的旧行保留并标 conflict，unknown / local 保持原状态，不将旧 UID 的修改套用到新 UID。

v4 旧缓存保留正文与状态，缺少已验证定位时修改为 conflict。可主动核对原定位与内容，再明确确认恢复；有完整原文指纹时可跨 UIDVALIDITY 完整搜索重绑定，缺少指纹的旧缓存仍不能跨代。候选较多时持久分页，确认阶段也重新完整核对，非 local 意图之后才排队回写。不会从 remote_id 字符串或单页匹配猜身份、不清库，详见 REMOTE_RECOVERY。

失败、冲突或 unknown 在阅读区展示错误。“仅保留本机”将意图转 local 并停止恢复，不撤销已发生的服务器操作。applying 时禁止此操作；unknown 归档不能从普通归档按钮重复提交。主动核对后可确认目标恢复，或在源仍存在且目标无匹配时明确重新排队；未编辑重复缓存可事务合并，有修改的两份保留待处理，详见 REMOTE_RECOVERY。

## IMAP 回写

1. LIST 后先回写本次快照中的最多 200 条 queued / failed 意图，再拉取新邮件并分组刷新已缓存 UID 的 FLAGS（每请求最多 100 UID）。期间新修改及超出 200 条的部分留待下次手动同步，结果 warnings 报告剩余数量。
2. 每条回写先 SELECT 源文件夹，核对 UIDVALIDITY，再精确 UID FETCH 确认原邮件存在；再次检查本地 revision / generation，短事务标 applying，然后释放 DB 锁发命令。
3. 已读使用 `+FLAGS.SILENT (\Seen)`，未读使用 `-FLAGS.SILENT (\Seen)`，星标只增减 `\Flagged`。不替换其他 flags / keywords。STORE 后再次 FETCH 核对 UID 和目标标记才确认；断线保留 failed，可幂等重试。
4. 归档要求服务器 CAPABILITY 含 MOVE，以及 LIST 中唯一 `\Archive` 特殊用途目录；无特殊用途时只接受唯一名称匹配的归档目录。忽略 `\Noselect`。不自动建目录，不 COPY + 整箱 EXPUNGE，也不把 Gmail Archive 当作普通文件夹模型直接套用。
5. 使用 UID MOVE；必须同时获得成功完成和单 UID COPYUID 映射才确认。imap 2.x 丢弃 tagged response code，TracedStream 只在 MOVE 时提取 tagged / untagged COPYUID；跳过 literal 字节、不扫描正文、不记录密码或协议日志；超长行和歧义 / 多 UID 映射不用于确认。
6. 确认目标后短事务保留本机 message ID、更新远端定位与文件夹、清除该归档意图，并把期间新增的 queued / failed 标记意图定位移到目标 UID。数据库确认失败也标 unknown，不再次 MOVE。

网络、凭据、SELECT / FETCH / STORE / MOVE / LOGOUT 和 MIME 解析均在 DB 锁外；锁内只读快照或做短事务。仍沿用同步调度器，不新增旁路 worker。

参考：[RFC 6851 MOVE](https://www.rfc-editor.org/rfc/rfc6851)、[RFC 4315 UIDPLUS](https://www.rfc-editor.org/rfc/rfc4315)。不能把 NO / 连接中断简单等同于移动完全未发生。

## 重启与验证

启动迁移后恢复上次 applying：幂等标记转 queued，归档转 unknown；只恢复意图状态，不自动连服务器。原同步任务队列仍为进程内队列，邮件修改意图独立持久化。

回归入口：message_changes.rs 验证落盘 / 重开、账号隔离、旧 revision、保存回滚、崩溃恢复；imap_writeback.rs 用逐命令脚本验证断线重试、精确 flags、原 UID 消失、UIDVALIDITY / 账号改变、在途再次编辑、COPYUID 和 unknown；mail_sync.rs 验证已缓存标记刷新、网络挂起时保存、旧 UID 冲突保留、稳定 ID。

TS 测试验证演示持久化、保存失败、连续切换、旧 reload 与保存响应晚于服务器确认到达；Playwright 验证离线归档、重新加载、失败反馈，以及 test-only IPC 下待回写 / 冲突 / 仅本机交互。IPC harness 不等于 Rust 或真实服务器端到端。

修改后运行 `npm run check`、`npm run check:native`、`npm run test:ui`。尚未验证真实服务器 MOVE / UIDPLUS / ACL、跨平台凭据、真实 WebView 完整闭环；主动旧缓存 / unknown 核对见 REMOTE_RECOVERY，A3.8 已有明确自动续页、IDLE / CONDSTORE、删除存在性 / 跨目录可信指纹核对、持久退避和可选自动同步，见 SYNCHRONIZATION；仍不自动采纳未确认恢复或永久删除。

补齐可信原文指纹只改变指纹元数据，不改邮件定位、本机状态或意图 revision；旧弱内容观察不能升级为历史指纹。大范围分段核对沿用本机修改保护，只有原文阶段与全段集合复核都完成且明确确认后才重新定位意图，见 REMOTE_RECOVERY。

A3.7 的 IMAP 流式限额在库缓冲前生效；超限停止连接，不以部分 flags 或 COPYUID 确认结果。尤其 MOVE 已发出后，即使先收到有效 COPYUID，后续响应超限仍为 unknown 且不重试。恢复保留上一完整检查点和本机意图，密集区间缩小不减少绑定校验，详见 IMAP_LIMITS。


A3.8 原连接配置 / 重新授权成功后旧邮件标 remoteMissing，待回写字段冲突，保留本机 ID / 内容；遇远端 canonical ID 复用则退休旧存储键，不能 UPSERT 把旧意图赋给新邮件。恢复保护在退休前生效。原文重取、附件导出和 MODSEQ 优化不改变字段意图来源 / revision；旧结果不能清新编辑。
