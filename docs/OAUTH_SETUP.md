# 桌面 IMAP OAuth 配置

状态：2026-11-11。已实现 Google / Microsoft 公共桌面应用授权码 + PKCE、系统浏览器、loopback 回调、XOAUTH2、刷新轮转与重新授权入口；没有内置 Client ID，没有真实注册应用 / 邮箱验收。界面等待或脚本测试不代表授权成功。

## 操作

桌面设置 → 添加 IMAP 账号，先填写名称和邮箱 → 使用 Gmail / Microsoft 授权。已有 OAuth 账号点重新授权，密码账号点连接设置；重配沿用本机 accountId，保留邮件和修改。OAuth 仅配置收信，SMTP 没有被验证或授权发信。

Client ID 是公共应用标识，可在界面填写并保存到账号公开配置；不要输入 client secret。授权在系统浏览器执行，用户同意后真实 XOAUTH2 登录 + LIST 成功才存账号。授权失败 / 取消保留原账号与凭据；保存失败补偿旧凭据。完成后点刷新账号列表或关闭，会重新读取工作区。

## Google

在 Google 控制台注册 Desktop app，配置同意屏幕及适用测试用户，复制 Client ID；本实现回调 http://127.0.0.1:<随机端口>/callback，使用离线授权与 S256。IMAP 固定 imap.gmail.com:993 TLS，申请 https://mail.google.com/；这授予广泛邮箱权限，正式分发前需完成对应审核及政策核验。组织策略与测试状态可能限制授权。

注册与公开客户端流程依据 [Google 桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)，IMAP 令牌使用方式见 [Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)。

## Microsoft

在 Entra 注册公共桌面应用，选择实际需要的账号类型。为移动 / 桌面应用配置 http://localhost/callback；本实现随机端口，路径必须匹配。填 Application client ID；租户默认 common，可填 organizations / consumers 或租户 ID。组织管理员可能需要允许 IMAP 和委派授权。

固定 outlook.office365.com:993 TLS，申请 https://outlook.office.com/IMAP.AccessAsUser.All 和 offline_access，不申请 SMTP.Send。配置依据 [IMAP OAuth 权限](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/) 与 [loopback 回调规则](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)。

## 安全与故障边界

- 仅 Rust 收到授权码 / 令牌，IPC 只返回 flow ID、阶段与固定错误；不记录原始 HTTP / IMAP 错误正文。refresh token 仅 OS keyring，access token 只内存并按账号 / generation / 到期隔离。
- state、PKCE verifier 每会话随机；loopback 仅绑定 127.0.0.1、随机端口，限制 GET 路径、Host、单一 state / code、请求 16 KiB、最多 128 次、180 秒；取消在提交屏障前有效，committing 阶段等待实际结果。
- token 请求仅固定 HTTPS 服务商端点，禁止重定向，连接 10 秒 / 总 20 秒、响应 64 KiB；校验 Bearer、有效期、scope 与字段边界。配置拒绝不回显令牌。
- OAuth 仅允许各提供商官方 IMAP TLS 主机，不能把令牌发送到自定义服务器。刷新 / 轮转与重新配置通过账号凭据锁串行，网络和 keyring 不占 DB 锁。
- 重新授权 / 密码保存旋转 generation；迟到授权结果不能覆盖期间重配 / 删除。原凭据补偿失败暂停同步并要求重新授权。
- 撤销 / 权限不足使任务暂停，界面重新授权；网络 / 限流由持久任务退避。应用重启丢弃 access token，读取 keyring 刷新。

原生测试使用临时 synthetic keyring 项且清理，测试 PKCE、回调隔离、scope / token 校验、SASL 挑战与 SQL 失败补偿；未调用真实 token 服务。尚待真实验收：Google / Microsoft 登录、刷新到期 / 撤销、租户策略、系统浏览器返回及 WebView 生命周期。macOS / Linux 凭据与打包尚未验证。
