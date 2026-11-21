# 桌面 IMAP OAuth 配置

状态：2026-11-16。已实现 Google / Microsoft 公共桌面应用授权码 + PKCE、系统浏览器、loopback 回调、XOAUTH2、刷新轮转与重新授权入口。Google 的公共应用 Client ID 已随应用内置，用户无需填写；Microsoft 应用尚未注册，提供商卡片显示为「即将支持」。既有真实 Gmail 授权与基础发送已验收；本轮新 secret 的实际刷新与 Gmail 连接已验收，旧 secret 停用后的再次刷新仍待验证。界面等待或脚本测试不代表授权成功。

## 操作

桌面设置 → 邮箱账号 → 添加账号 → 选择提供商卡片。Gmail 卡片只需账号名称和邮箱地址，点「在浏览器中授权」即开始授权，进度内嵌显示在设置窗体的当前页（取消 / 重新授权 / 刷新账号列表均在此完成）。用户无需填写任何 Client ID 或 secret；Client ID 仍是内置的公开标识；配套 secret 仅在 Rust 构建时从私有配置注入。桌面二进制不能保守共享 secret；PKCE 防护授权码兑换，不证明应用身份。高级场景可在重新授权页中替换为自己的 Client ID（自定义桌面应用需自行满足服务商的 secret 要求，界面不提供 secret 输入）；显式填写的值优先于内置兜底。已有账号从「管理」进入「连接设置」，OAuth 账号点「重新授权」，密码账号点「编辑连接」。重配沿用本机 accountId，保留邮件和修改。Gmail OAuth 已接入 IMAP / SMTP；Microsoft 的 SMTP 能力仍未开放。

授权在系统浏览器执行，用户同意后真实 XOAUTH2 登录 + LIST 成功才存账号。授权失败 / 取消保留原账号与凭据；保存失败补偿旧凭据。完成后点刷新账号列表或关闭，会重新读取工作区。

## Google

应用已注册：Google Cloud 项目 `auumail`，Desktop app 类型，Client ID `968245167784-7t1n7fbgjqh8iu04ha9scm43jjkevjmp.apps.googleusercontent.com` 保留于 `mail_oauth.rs`（`GOOGLE_BUILTIN_CLIENT_ID`）；配套 secret 不再出现在受版本控制的源码中，由 `build.rs` 生成到忽略的 Cargo OUT_DIR。实测 Google 令牌端点对 Desktop 类型客户端要求携带 client secret（缺失返回 `invalid_request: client_secret is missing`），secret 仅在使用内置 Client ID 时附带，自定义 Client ID 不受影响；secret 不进入 keyring / SQLite / 日志。归一化发生在授权流开始、持久化之前，因此 profile 与 keyring Secret 携带同一份配置。测试状态限制：refresh token 约 7 天过期需重新授权；`https://mail.google.com/` 为 restricted scope，授权页会出现未验证应用警告。本实现回调 http://127.0.0.1:<随机端口>/callback，使用离线授权与 S256。IMAP 固定 imap.gmail.com:993 TLS；正式分发前需完成对应审核及政策核验。组织策略与测试状态可能限制授权。

### 私有构建配置

- 首选构建进程环境变量 `AUUMAIL_GOOGLE_CLIENT_SECRET`（不要使用 `VITE_` 前缀）；CI / 正式打包从受保护的 secret 配置注入，不把值写入工作流或命令行。
- 本机开发可用 `src-tauri/google-oauth-secret.local`，UTF-8、只含 secret 值、不含变量名或引号。该文件被 Git 忽略，示例说明见同目录 `google-oauth-secret.example`。首次迁移时旧值转入此文件；现已替换为用户提供的新值，Client ID 和现有邮箱授权保持原样。
- 环境变量优先于本机文件；显式空白环境值禁用本机回退。配置只在构建时读取，修改后须重建 Rust 并重启应用，单独前端热更新不会切换 secret。
- build.rs 监听环境变量和本机文件变化。值不通过 cargo:rustc-env 输出，不进入前端、SQLite、keyring 或应用日志；仅写入忽略的 Cargo 生成文件并编入桌面二进制。不要上传 target / 私有配置作为 CI 工件。
- 未提供值时开发 / 测试仍可构建，内置 Google 客户端在请求令牌前显示明确错误；密码账号与 Microsoft 不受此项门控影响。release 构建缺少值直接失败，防止发布不能刷新既有 Gmail 账号的程序。单元测试只用虚构凭据，不要求 CI 配置真实 secret。
- 仅从源码移除不会清除 Git 历史，旧值仍须在 Google 端轮换。此改动没有修改邮箱账号、Client ID、refresh token、generation、数据库或系统凭据。

### 同一 Client ID 的无中断轮换顺序

依据 [Google 管理 OAuth 客户端](https://support.google.com/cloud/answer/15549257?hl=en) 的双 secret 流程：

1. Google Cloud 项目 `auumail` → Google Auth Platform → Clients，打开现有 Desktop app；核对 Client ID，保持原客户端，不删除、不重建，也不撤销用户授权。
2. 在该客户端选择 **Add Secret**，立即安全保存新值。此时保持旧值启用。若页面只有 **Reset Secret**、不支持并存，不直接重置；先确认该客户端支持的迁移方式，不能承诺立即重置后旧版本不停用。
3. 用文本编辑器打开本机 `.local` 文件，将内容替换为新值（不要提交、粘贴到聊天或终端命令历史）；CI 更新受保护的同名环境变量。
4. 保存正在编辑的邮件，退出旧桌面进程，重新运行 `npm run desktop:dev`，或以新配置打包并启动更新版本。重启清除内存 access token 后，手动同步现有 Gmail 账号：确认既有 refresh token 能换得 access token、同步成功；无需删除邮箱或撤销权限。不同 Client ID 的 refresh token 不能直接沿用。
5. 确认所有仍在使用的构建 / 设备均切换后，Google 控制台只 **Disable 旧 secret**，再次重启并同步验证。刚启动前的旧 access token 仍能使用并不证明新 secret 刷新成功；应以重启后的同步为验证点。此验证不需要自动发信。
6. 若停用后失败，在仍可回退阶段重新启用旧 secret，排查是否仍运行旧构建；新值稳定后才 **Delete 旧 secret**（不可撤销）。不要依赖永久保留已泄露值作为兼容方案。

用户提供的新值已写入本机忽略配置并重建；2026-11-16 已按用户授权重启新程序，验证既有授权在新 secret 下可刷新并连接 Gmail，多个目录成功提交。早期全目录任务触及 300 秒预算，已保留批次并停止；后续修复有界提交、目录续跑和 Gmail 大小差异后，25 个目录完整同步及额外手动复核均已通过（2026-11-16），详见 STATUS / SYNCHRONIZATION。本任务没有停用或删除 Google 旧值，旧值停用后的再次刷新仍待验证。Google 测试状态下 refresh token 自身可能到期，若出现 invalid_grant 需重新授权，不能保证已有过期令牌靠轮换恢复。

如需替换为自己的应用：在 Google 控制台注册 Desktop app，配置同意屏幕及适用测试用户，在重新授权页填入 Client ID 即可（显式填写优先于内置兜底）。

注册与公开客户端流程依据 [Google 桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)，IMAP 令牌使用方式见 [Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol)。

## Microsoft

2026-11-21 已注册公共桌面应用（Entra 应用注册 AuuMail，允许公共客户端流，重定向 `http://localhost/callback`——微软忽略端口、路径必须匹配），Client ID 随应用内置（`MICROSOFT_BUILTIN_CLIENT_ID`，公开信息非机密），公共桌面应用无需 secret。租户默认 common，要求应用的受支持账户类型为「任何组织目录 + 个人 Microsoft 账户」（`signInAudience: AzureADandPersonalMicrosoftAccount`）；重新授权页可显式替换 Client ID / 租户（organizations / consumers / 租户 ID），显式值优先于内置兜底。

**实测陷阱（2026-11-21）**：组合类型 `AzureADandPersonalMicrosoftAccount` **只能在创建应用注册时选择**；创建后在「身份验证」页改动「受支持的账户类型」并保存，会把清单静默降级为 `AzureADMultipleOrgs`（仅组织），个人账户授权即报 `unauthorized_client: not enabled for consumers`。判别以清单 `signInAudience` 为准，不以身份验证页下拉显示为准；补救只能重建应用注册。

个人 Outlook / Hotmail / Live 账户不需要在门户预配 API 权限：`https://outlook.office.com/IMAP.AccessAsUser.All` 委托权限在首次登录同意页动态授权（与 Thunderbird 相同的方式）；MSA 账户也无法在租户内预配 Exchange Online 服务主体。组织 / 学校租户的账号可能需要管理员对 IMAP 委派权限做租户级同意，属当前边界。

固定 outlook.office365.com:993 TLS，请求 https://outlook.office.com/IMAP.AccessAsUser.All 和 offline_access，不申请 SMTP.Send。配置依据 [IMAP OAuth 权限](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/) 与 [loopback 回调规则](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)。

## 安全与故障边界

- 仅 Rust 收到授权码 / 令牌，IPC 只返回 flow ID、阶段与固定错误；不记录原始 HTTP / IMAP 错误正文。refresh token 仅 OS keyring，access token 只内存并按账号 / generation / 到期隔离。
- state、PKCE verifier 每会话随机；loopback 仅绑定 127.0.0.1、随机端口，限制 GET 路径、Host、单一 state / code、请求 16 KiB、最多 128 次、180 秒；取消在提交屏障前有效，committing 阶段等待实际结果。
- token 请求仅固定 HTTPS 服务商端点，禁止重定向，连接 10 秒 / 总 20 秒、响应 64 KiB；校验 Bearer、有效期、scope 与字段边界。配置拒绝不回显令牌。
- OAuth 仅允许各提供商官方 IMAP TLS 主机，不能把令牌发送到自定义服务器。刷新 / 轮转与重新配置通过账号凭据锁串行，网络和 keyring 不占 DB 锁。
- 重新授权 / 密码保存旋转 generation；迟到授权结果不能覆盖期间重配 / 删除。原凭据补偿失败暂停同步并要求重新授权。
- 撤销 / 权限不足使任务暂停，界面重新授权；网络 / 限流由持久任务退避。应用重启丢弃 access token，读取 keyring 刷新。

原生测试使用临时 synthetic keyring 项且清理，测试 PKCE、回调隔离、scope / token 校验、SASL 挑战与 SQL 失败补偿；未调用真实 token 服务。尚待真实验收：Google / Microsoft 登录、刷新到期 / 撤销、租户策略、系统浏览器返回及 WebView 生命周期。macOS / Linux 凭据与打包尚未验证。
