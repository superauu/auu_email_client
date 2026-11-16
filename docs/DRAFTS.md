# 本地草稿

状态：2026-11-16。保存草稿只写本机；发送由独立 outbox 路径完成，不上传服务器 Drafts。撰写交互见 [COMPOSING](COMPOSING.md)，发送见 [SENDING](SENDING.md)。

## 存储与边界

- 桌面 gateway：用户添加账号的本地草稿经 IPC 写 SQLite drafts 表；旧演示草稿不进入桌面工作区。已保存的真实 IMAP 账号即使离线也可撰写，不需要读取密码或建立连接。
- 浏览器 gateway：只有演示账号，使用 `auumail.demo.drafts.v1` localStorage，最多 100 份。不是实际邮箱连接。
- UI 统一经过 workspaceStore.saveDraft；组件不操作数据库或存储。
- `MailMessage.draft` 保存可重新编辑的原始 MailDraft。列表可显示“未命名草稿”，但编辑器里的空主题仍为空；replyToId 和 revision 不丢失。
- 撰写附件（v15，2026-11-16）：元数据存 `drafts.attachments_json`（id / name / size / mediaType / sha256），字节存 `draft_attachment_blobs` 子表并随草稿级联删除。选取经 `mail_compose_pick_attachments`（Rust 对话框 + 进程内暂存，前端不接触路径）；保存事务内核对 sha256 后落库，暂存缺失且无已落库内容时明确失败。每封邮件最多 20 个、合计 16 MiB；演示环境不支持选取并禁用说明。

## 自动保存

`DraftSession` 是无平台依赖的状态机；800ms 防抖。新建空白编辑器不会自动制造空草稿，明确点击保存可以保存空草稿。

每份编辑会话只允许一个保存请求在途。保存期间输入框仍可编辑；新修改合并到后续写入，在最新版本保存完成前不会显示“草稿已保存”。显式保存和“保存并关闭”会等待最新缓冲区落盘。

失败时保留输入、dirty 状态和上次成功 revision，显示错误，不无限自动重试；可继续修改或点击保存重试。关闭兜底条出现期间（保存失败）暂停待执行的自动保存，避免选择放弃草稿后定时器又写入。已进入数据库的更早自动保存版本不会因放弃草稿之外的关闭路径而丢失。

## 账号与并发

新草稿 revision=0，首次保存返回 1，之后逐次递增。数据库事务同时核对 ID、revision、previousAccountId；不匹配返回非重试型 conflict，保持已有数据和当前编辑内容。不能自动提高 revision 强行覆盖。

普通草稿允许更换发件账号，保存时携带上次成功保存的账号，将同一 ID 原子移动到新账号，不创建第二份草稿。回复草稿锁定原邮件所属账号；Rust 也校验仍存在的关联邮件所属账号。原邮件已被清理时保留 replyToId 作为历史关联，不把它当成发信授权。

账号被删除时草稿按外键级联删除；仍打开的编辑器保存会失败并保留内容。已有 outbox 引用的草稿拒绝改写，为后续发送状态机保留边界。

发送队列中 queued 可明确取消；明确 failed 可「返回草稿修改」（mail_outbox_return_draft），条件删除队列占用后原草稿恢复编辑。正文、收件人、附件、原账号及 revision 均不改，不另建副本、不提高版本绕过冲突。返回后定位本机草稿，关闭设置可继续编辑；已打开的其他撰写内容保留。unknown 与由其转来的 keep-local 不允许退回或普通重试，sent / sending 也不能退回。重试未修改内容复用原 Message-ID；退回编辑后的显式发送作为新提交生成新 Message-ID。详见 SENDING。

浏览器演示具有版本核对，但 localStorage 不提供跨进程事务；严格多窗口并发保证由桌面 SQLite 提供。当前默认单主窗口。

## 关闭与恢复

- 编辑器关闭 / Esc / 桌面关闭按钮：先自动保存草稿再收起，无需手动确认；自动保存失败时显示兜底条（保存并关闭 / 放弃草稿 / 继续编辑），内容不静默丢弃。
- 放弃草稿是唯一的删除入口：撰写面底部显式按钮，调用 mail_delete_draft 删除本机草稿行（幂等；已进入发送流程的草稿拒绝删除）并关闭撰写面；附件字节随外键级联删除，Rust 暂存一并回收。
- 浏览器刷新 / 离开：dirty 或保存中通过 beforeunload 请求原生离页确认。浏览器可能不触发该事件，不能用它替代自动保存。
- 强制结束进程、断电或系统崩溃不保证最后 800ms 内的未落盘输入恢复。重启后从草稿箱打开最后成功保存版本；未落库的暂存附件随进程消失，重开草稿时按已保存内容呈现，不显示丢失内容。

## 数据库与旧版迁移

003_local_drafts.sql 追加 revision、账号 / 时间索引和 local_imports 收据表。已有 v2 drafts 保留内容并赋 revision=1。不得修改 001 / 002。v15 追加附件列与 BLOB 子表，旧草稿缺省无附件；旧版导入（v3 收据流程）拒绝带附件元数据的草稿而不是静默剥离。

桌面正常启动不再初始化演示账号，也不再自动导入旧 WebView localStorage 演示草稿。原 localStorage 与 SQLite 中已导入的演示草稿保留原样，但不进入真实工作区；浏览器仍使用独立演示存储。旧导入 IPC 仅保留兼容与测试能力：只允许旧演示账号，最多 100 份，事务与导入收据去重，相同 ID 的已有 SQLite 草稿不覆盖。

演示种子 IPC 保留兼容与测试用途，不由桌面正常加载调用；不通过删除演示账号行来隔离数据，避免级联删除持久草稿。

## IPC

| 命令 | 输入 | 返回 |
| --- | --- | --- |
| mail_list_drafts | 无 | MailMessage[]，包含 draft 元数据与附件元数据 |
| mail_delete_draft | draftId | void |
| mail_save_draft | draft、previousAccountId | 持久化后的 MailMessage，含新 revision |
| mail_import_legacy_drafts | drafts | void，事务完成或完整回滚 |
| mail_compose_pick_attachments | draftId、currentTotalBytes、currentCount | 新增附件元数据；取消为空数组；不接收路径 |
| mail_outbox_return_draft | id | 仅明确 failed 的原发送条目；解除占用，草稿保持原样 |

服务器时间用于新保存的 updatedAt；日期与 ID 不作为并发控制替代物。收件人 / 抄送 / 密送和主题上限各 1000 个 UTF-16 单位，正文 50000、HTML 200000。v12 追加 cc / bcc，旧草稿缺省为空。地址无效时可继续保存草稿，发送时另做语法校验。附件名称 ≤240 字符、id 为 `att-` 前缀、sha256 为 64 位十六进制。

## 后续

图片发送（正文内联 / 粘贴图片）、服务器草稿同步、冲突内容对照及多窗口恢复待后续。SMTP / outbox、富文本、抄送 / 密送与撰写附件已实现，验证边界见 STATUS 与 COMPOSING。
