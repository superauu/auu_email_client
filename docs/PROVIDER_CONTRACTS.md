# 邮件、账号与 AI 接入约定

状态：A3.8 已实现 IMAP TLS / STARTTLS、多目录 / 历史、持久调度、IDLE / CONDSTORE、附件缓存与 Google / Microsoft 公共客户端 OAuth 配置 / 刷新。真实服务器 / 授权未验收；真实发信、POP3、Graph / Gmail API、AI 和日历未实现。接入范围见 IMAP_COMPLETION，公共应用配置见 OAUTH_SETUP。

## 提供商与协议

| 类型 | 计划路径 | 能力边界 |
| --- | --- | --- |
| 通用 IMAP / SMTP | TLS / STARTTLS，密码或服务端授权码；提供 OAuth 扩展 | 文件夹与状态同步、发送，能力由服务端协商 |
| POP3 / SMTP | UIDL 去重，明确保留 / 删除服务器副本 | 不承诺文件夹或已读状态同步 |
| Microsoft 365 / Outlook.com | Graph 优先；OAuth 协议连接作为可选适配 | 受账号、租户授权、管理员策略和协议开关影响 |
| Gmail | Gmail API 优先；XOAUTH2 协议连接可选 | 标签与会话保留原生语义，公开分发需审核 |

Microsoft 的 POP / IMAP / SMTP OAuth 也适用于 Microsoft 365 和 Outlook.com；Google 提供 XOAUTH2，邮件读取 scope 涉及受限权限。实际接入时重新核验官方要求，不把“拥有密码”视为所有服务都能登录。

参考：[Microsoft OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)、[Graph 邮件接口](https://learn.microsoft.com/en-us/graph/outlook-mail-concept-overview)、[Google 桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)、[Gmail scope](https://developers.google.com/workspace/gmail/api/auth/scopes)。查阅基准：2026-11-11。

## 账号与认证

- 每个连接具有独立不透明 accountId，不以邮箱地址当主键；同地址可能属于不同租户或配置。
- 服务端适配器报告 capability，不可用功能在 UI 中明确禁用或标为本地。
- OAuth 使用系统浏览器、授权码与 PKCE；校验 state、一次性回调、超时、取消、目标账号。桌面安装包不能依赖保密的内置 client secret。
- refresh token / 密码 / AI Key 存系统凭据库，前端只持有凭据引用和状态；连接参数与密钥分开。
- 所需 scope 最小化。Microsoft / Google 应用注册、回调配置、测试用户和公开发布审核是实际依赖，不能用模拟登录跳过。
- 账号切换、登出或重新授权后隔离旧请求响应；UI 不得展示前一个账号的正文。

## 同步与发送

- 每账号独立的任务队列与游标，限制全局并发；单账号失败不阻塞其他账号。
- 当前已实现进程内 FIFO（全局 2 个 worker、同账号活动任务去重）、排队取消、运行中停止和手动重试。网络 / keyring / 正文解析在 DB 锁外；批次提交核对 generation 与游标，细节见 SYNCHRONIZATION。v9 已持久化请求 / 退避、运行中停止、重启恢复和默认关闭的自动同步。
- 先同步最近邮件索引，再按需正文 / 附件与历史；UI 需要分页 / 虚拟化，不全量驻留。
- IMAP 维护 folder / UIDVALIDITY / UID；POP 维护 UIDL；Graph / Gmail 保存各自增量游标，失效时有明确重建流程。
- 同步和本地编辑明确冲突策略；删除、移动、已读及标签要有离线任务和失败状态。
- 旧 IMAP 缓存和 unknown 归档已有主动核对 / 二次确认流程，见 REMOTE_RECOVERY。remote_id / Message-ID 仅作为候选线索；不跨 UIDVALIDITY 猜测身份。原文指纹缺失时明确标出缓存字段证据的局限；歧义保留原缓存，有编辑副本不自动合并。
- 跨 UIDVALIDITY 仅凭已有完整原文 SHA-256 进行候选匹配，完整扫描唯一后再明确确认并重新完整核对；分页进度不等于唯一性证明。继续页必须复核 generation、消息版本、UIDVALIDITY 和候选集合，不能把某一页的一条匹配直接绑定。
- 完整原文指纹需有可信来源：普通 FETCH 或可信原定位下重复读取并明确确认的补齐；相似正文或历史弱恢复观察不产生跨代可信证据。超过 50000 候选时用 UIDNEXT 固定边界的区间 SEARCH，再完成全段集合复核，不删减搜索范围后断言唯一。
- IMAP 库前必须保留 LimitedStream 的流式限额与超限后永久停止连接；不能把部分 SEARCH / literal 当作成功。密度估算只决定持久固定区间宽度，不以 EXISTS 证明完整性或减少页前后 / 最终集合验证。大邮件 / 批次超限明确失败，MOVE 保留 unknown；见 IMAP_LIMITS。
- 已读 / 星标 / 归档已有 SQLite 字段意图；下次手动同步尝试回写，本机未确认字段优先、旧版本不清除新修改。仅实现通用 IMAP FLAGS 与有唯一目标的 UID MOVE；结果不明的移动不自动重试。完整约定见 MESSAGE_CHANGES；远端删除只保留并标缺失，外部移动可明确进行可信指纹跨目录核对；永久删除、keywords、Gmail 原生多标签写入未实现。
- 本地草稿已持久化 SQLite，支持自动保存、revision 冲突核对、最后成功版本恢复和退出保护，详见 DRAFTS；服务器草稿可收取及复制为本地草稿，上传 APPEND 未实现。
- 发件人默认原收件账号，别名和签名按账号；发送前展示最终身份。
- outbox 存 queued / sending / sent / failed / unknown；网络中断后结果不明时不直接重发。本地 idempotency_key 并不保证 SMTP 或服务商提供幂等，必须核查已发送记录或由用户处理。
- 测试至少覆盖断线、令牌过期、限流、游标失效、重复下载、部分失败及重启恢复。

## 标签与事件

本地标签可跨账号，不暗示同步；服务器标签 / Microsoft 分类 / IMAP keywords 使用各自映射，不能把不支持的标记静默上传。账户级服务器标签不得关联其他账号邮件，需在写入 repository 校验。

事件区分 local / ics / provider；时间保存 UTC，事件另存 IANA 时区。首步实现本地提醒和 ICS 解析，随后才接 Google / Microsoft Calendar 双向同步。协议收信不自动获得日历权限。

## 正文与附件

当前只渲染 bodyText，React 正常转义。接入真实 HTML 需单独清洗、沙箱、禁脚本 / 表单 / IPC、阻断远程跟踪、限制导航；不要直接 dangerouslySetInnerHTML 到主应用。

已有完整原文 / 附件元数据、显式另存为、原文重取与配额，详见 IMAP_LIMITS；附件预览 / 显式打开仍未实现。附件需继续保留 MIME / 大小 / 路径校验。不得自动执行附件。嵌入图片和链接不放宽主窗口 CSP。

## AI

`src/domain/ai.ts` 已定义任务、请求和结果契约；当前面板是未启用状态，没有网络请求。

- 初期能力：摘要、回复草稿、翻译、事件提取、标签建议。通过独立 AiProvider 适配模型服务与本地模型。
- 统一支持 AbortSignal、流式文本回调、超时、结果归属和过期响应丢弃；不要在 React 中拼模型请求。
- contextText 只能包含用户选择 / 授权的内容，邮件正文视为不可信数据。邮件中的指令不能获得发信、文件、账号或工具权限。
- 自动撰写默认生成可编辑草稿，发送由用户明确触发。未来若加入规则自动回复，另行设计授权、限额、审计和防循环。
- 云端服务默认关闭，明确展示发送的数据范围；附件和整段会话单独选择。诊断不记录正文或完整 prompt。
- Gmail 数据用于第三方 AI 时，发布前核验 Google 数据使用政策、审核和所选服务的数据处理方式。
