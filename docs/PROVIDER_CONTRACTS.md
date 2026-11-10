# 邮件、账号与 AI 接入约定

状态：本文定义后续实现要求。除演示 gateway 外，协议、OAuth、keyring、AI 和日历服务均未实现。

## 提供商与协议

| 类型 | 计划路径 | 能力边界 |
| --- | --- | --- |
| 通用 IMAP / SMTP | TLS / STARTTLS，密码或服务端授权码；提供 OAuth 扩展 | 文件夹与状态同步、发送，能力由服务端协商 |
| POP3 / SMTP | UIDL 去重，明确保留 / 删除服务器副本 | 不承诺文件夹或已读状态同步 |
| Microsoft 365 / Outlook.com | Graph 优先；OAuth 协议连接作为可选适配 | 受账号、租户授权、管理员策略和协议开关影响 |
| Gmail | Gmail API 优先；XOAUTH2 协议连接可选 | 标签与会话保留原生语义，公开分发需审核 |

Microsoft 的 POP / IMAP / SMTP OAuth 也适用于 Microsoft 365 和 Outlook.com；Google 提供 XOAUTH2，邮件读取 scope 涉及受限权限。实际接入时重新核验官方要求，不把“拥有密码”视为所有服务都能登录。

参考：[Microsoft OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)、[Graph 邮件接口](https://learn.microsoft.com/en-us/graph/outlook-mail-concept-overview)、[Google 桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)、[Gmail scope](https://developers.google.com/workspace/gmail/api/auth/scopes)。查阅基准：2026-11-10。

## 账号与认证

- 每个连接具有独立不透明 accountId，不以邮箱地址当主键；同地址可能属于不同租户或配置。
- 服务端适配器报告 capability，不可用功能在 UI 中明确禁用或标为本地。
- OAuth 使用系统浏览器、授权码与 PKCE；校验 state、一次性回调、超时、取消、目标账号。桌面安装包不能依赖保密的内置 client secret。
- refresh token / 密码 / AI Key 存系统凭据库，前端只持有凭据引用和状态；连接参数与密钥分开。
- 所需 scope 最小化。Microsoft / Google 应用注册、回调配置、测试用户和公开发布审核是实际依赖，不能用模拟登录跳过。
- 账号切换、登出或重新授权后隔离旧请求响应；UI 不得展示前一个账号的正文。

## 同步与发送

- 每账号独立的任务队列与游标，限制全局并发；单账号失败不阻塞其他账号。
- 先同步最近邮件索引，再按需正文 / 附件与历史；UI 需要分页 / 虚拟化，不全量驻留。
- IMAP 维护 folder / UIDVALIDITY / UID；POP 维护 UIDL；Graph / Gmail 保存各自增量游标，失效时有明确重建流程。
- 同步和本地编辑明确冲突策略；删除、移动、已读及标签要有离线任务和失败状态。
- 真实草稿持久化 SQLite，补自动保存、防过期写入、崩溃恢复和应用退出保护。
- 发件人默认原收件账号，别名和签名按账号；发送前展示最终身份。
- outbox 存 queued / sending / sent / failed / unknown；网络中断后结果不明时不直接重发。本地 idempotency_key 并不保证 SMTP 或服务商提供幂等，必须核查已发送记录或由用户处理。
- 测试至少覆盖断线、令牌过期、限流、游标失效、重复下载、部分失败及重启恢复。

## 标签与事件

本地标签可跨账号，不暗示同步；服务器标签 / Microsoft 分类 / IMAP keywords 使用各自映射，不能把不支持的标记静默上传。账户级服务器标签不得关联其他账号邮件，需在写入 repository 校验。

事件区分 local / ics / provider；时间保存 UTC，事件另存 IANA 时区。首步实现本地提醒和 ICS 解析，随后才接 Google / Microsoft Calendar 双向同步。协议收信不自动获得日历权限。

## 正文与附件

当前只渲染 bodyText，React 正常转义。接入真实 HTML 需单独清洗、沙箱、禁脚本 / 表单 / IPC、阻断远程跟踪、限制导航；不要直接 dangerouslySetInnerHTML 到主应用。

附件需 MIME / 大小 / 路径校验、按需下载、本地缓存配额和显式打开流程。不得自动执行附件。嵌入图片和链接不放宽主窗口 CSP。

## AI

`src/domain/ai.ts` 已定义任务、请求和结果契约；当前面板是未启用状态，没有网络请求。

- 初期能力：摘要、回复草稿、翻译、事件提取、标签建议。通过独立 AiProvider 适配模型服务与本地模型。
- 统一支持 AbortSignal、流式文本回调、超时、结果归属和过期响应丢弃；不要在 React 中拼模型请求。
- contextText 只能包含用户选择 / 授权的内容，邮件正文视为不可信数据。邮件中的指令不能获得发信、文件、账号或工具权限。
- 自动撰写默认生成可编辑草稿，发送由用户明确触发。未来若加入规则自动回复，另行设计授权、限额、审计和防循环。
- 云端服务默认关闭，明确展示发送的数据范围；附件和整段会话单独选择。诊断不记录正文或完整 prompt。
- Gmail 数据用于第三方 AI 时，发布前核验 Google 数据使用政策、审核和所选服务的数据处理方式。
