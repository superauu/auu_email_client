# 桌面 IMAP OAuth 配置

状态：2026-11-14。已实现 Google / Microsoft 公共桌面应用授权码 + PKCE、系统浏览器、loopback 回调、XOAUTH2、刷新轮转与重新授权入口。Google 的公共应用 Client ID 已随应用内置，用户无需填写；Microsoft 应用尚未注册，提供商卡片显示为「即将支持」。真实 Gmail 授权验收待执行。界面等待或脚本测试不代表授权成功。

## 操作

桌面设置 → 添加账号 → 选择提供商卡片。Gmail 卡片只需账号名称和邮箱地址，点「在浏览器中授权」即开始授权，进度内嵌显示在同一对话框（取消 / 重新授权 / 刷新账号列表均在此完成）。用户无需填写任何 Client ID 或 secret；授权标识与 Google 桌面应用的 client secret 由应用内置携带（公共客户端 secret 不是机密，安全由 PKCE 保证）。高级场景可在重新授权对话框中替换为自己的 Client ID（自定义桌面应用需自行满足服务商的 secret 要求，界面不提供 secret 输入）；显式填写的值优先于内置兜底。已有 OAuth 账号点重新授权，密码账号点连接设置；重配沿用本机 accountId，保留邮件和修改。OAuth 仅配置收信，SMTP 没有被验证或授权发信。

授权在系统浏览器执行，用户同意后真实 XOAUTH2 登录 + LIST 成功才存账号。授权失败 / 取消保留原账号与凭据；保存失败补偿旧凭据。完成后点刷新账号列表或关闭，会重新读取工作区。

## Google

应用已注册：Google Cloud 项目 `auumail`，Desktop app 类型，Client ID `968245167784-7t1n7fbgjqh8iu04ha9scm43jjkevjmp.apps.googleusercontent.com` 与配套 client secret 内置于 `mail_oauth.rs`（`GOOGLE_BUILTIN_CLIENT_ID` / `GOOGLE_BUILTIN_CLIENT_SECRET`）。实测 Google 令牌端点对 Desktop 类型客户端要求携带 client secret（缺失返回 `invalid_request: client_secret is missing`），secret 仅在使用内置 Client ID 时附带，自定义 Client ID 不受影响；secret 不进入 keyring / SQLite / 日志。归一化发生在授权流开始、持久化之前，因此 profile 与 keyring Secret 携带同一份配置。测试状态限制：refresh token 约 7 天过期需重新授权；`https://mail.google.com/` 为 restricted scope，授权页会出现未验证应用警告。本实现回调 http://127.0.0.1:<随机端口>/callback，使用离线授权与 S256。IMAP 固定 imap.gmail.com:993 TLS；正式分发前需完成对应审核及政策核验。组织策略与测试状态可能限制授权。

如需替换为自己的应用：在 Google 控制台注册 Desktop app，配置同意屏幕及适用测试用户，在重新授权对话框填入 Client ID 即可（显式填写优先于内置兜底）。

注册与公开客户端流程依据 [Google 桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)，IMAP 令牌使用方式见 [Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)。

## Microsoft

尚未注册 Microsoft 公共应用：提供商卡片显示为禁用「即将支持」，不开放授权入口。注册后移除 `providers.ts` 中 microsoft 卡片的 `available: false` 即可开放。在 Entra 注册公共桌面应用，选择实际需要的账号类型。为移动 / 桌面应用配置 http://localhost/callback；本实现随机端口，路径必须匹配。填 Application client ID；租户默认 common，可填 organizations / consumers 或租户 ID。组织管理员可能需要允许 IMAP 和委派授权。

固定 outlook.office365.com:993 TLS，申请 https://outlook.office.com/IMAP.AccessAsUser.All 和 offline_access，不申请 SMTP.Send。配置依据 [IMAP OAuth 权限](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/) 与 [loopback 回调规则](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)。

## 安全与故障边界

- 仅 Rust 收到授权码 / 令牌，IPC 只返回 flow ID、阶段与固定错误；不记录原始 HTTP / IMAP 错误正文。refresh token 仅 OS keyring，access token 只内存并按账号 / generation / 到期隔离。
- state、PKCE verifier 每会话随机；loopback 仅绑定 127.0.0.1、随机端口，限制 GET 路径、Host、单一 state / code、请求 16 KiB、最多 128 次、180 秒；取消在提交屏障前有效，committing 阶段等待实际结果。
- token 请求仅固定 HTTPS 服务商端点，禁止重定向，连接 10 秒 / 总 20 秒、响应 64 KiB；校验 Bearer、有效期、scope 与字段边界。配置拒绝不回显令牌。
- OAuth 仅允许各提供商官方 IMAP TLS 主机，不能把令牌发送到自定义服务器。刷新 / 轮转与重新配置通过账号凭据锁串行，网络和 keyring 不占 DB 锁。
- 重新授权 / 密码保存旋转 generation；迟到授权结果不能覆盖期间重配 / 删除。原凭据补偿失败暂停同步并要求重新授权。
- 撤销 / 权限不足使任务暂停，界面重新授权；网络 / 限流由持久任务退避。应用重启丢弃 access token，读取 keyring 刷新。

原生测试使用临时 synthetic keyring 项且清理，测试 PKCE、回调隔离、scope / token 校验、SASL 挑战与 SQL 失败补偿；未调用真实 token 服务。尚待真实验收：Google / Microsoft 登录、刷新到期 / 撤销、租户策略、系统浏览器返回及 WebView 生命周期。macOS / Linux 凭据与打包尚未验证。
