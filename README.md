# AuuMail

一款以漂亮界面、独立皮肤和多邮箱统一管理为中心的本地桌面邮件客户端。技术栈：Tauri 2 / Rust、React 19、TypeScript、Vite 8、Tailwind CSS 4、Radix UI、Zustand、SQLite。

**当前包含演示工作区与桌面 IMAP / POP3 收信 / 同步、SMTP 发送队列（outbox，结果不明不自动重发）、Outlook 式撰写（地址标签 / 抄送密送 / 顶部格式栏 / 附件发送）：TLS / STARTTLS、系统凭据、按提供商卡片添加账号（Gmail 内置 OAuth 应用标识，Office 即将支持）、多目录 / 历史（POP3 为单一收件箱 + UIDL 增量）、持久调度 / 停止、IDLE / CONDSTORE、离线标记 / 归档恢复、完整原文缓存与附件另存为、中英文界面切换（`src/i18n` 消息字典）。代码与脚本检查通过，中文附件名 MIME 参数编码已按 RFC 2231 修复（待真实重验收），真实 Gmail 授权、基础发送、自发自收附件下载及回复会话归组已验收；抄送密送独立收件、其他客户端兼容与真实 POP3 仍待验收；AI 助手（OpenAI 兼容服务，摘要 / 回复草稿 / 提取待办，桌面版）与 ICS 邀请解析已接入，AI 输出仅为文本、回复作为草稿确认发送。具体限制与新发现问题见 STATUS。**

## 运行

Node.js 20.19+（或 22.12+）；桌面开发另需 Rust stable、Windows C++ Build Tools 与 WebView2。本轮验证环境为 Windows / Node 20.19.0 / Rust 1.98.1。其他系统前置依赖见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)。

```powershell
npm ci
npm run dev             # http://127.0.0.1:7317，浏览器预览
npm run desktop:dev     # Tauri 桌面窗口
```

Google OAuth 的 Client Secret 不在源码中，桌面开发 / 打包需通过环境变量 `AUUMAIL_GOOGLE_CLIENT_SECRET` 或忽略的 `src-tauri/google-oauth-secret.local` 注入；缺值的 release 构建会失败。配置与轮换顺序见 [OAuth 配置](docs/OAUTH_SETUP.md)。

桌面版默认只显示你添加的邮箱，没有账号时显示添加入口；旧演示数据保留但不混入邮件、搜索、标签、待办和统计。浏览器预览独立使用演示数据。

`npm run dev` 和 `npm run desktop:dev` 二选一启动；桌面命令会自行启动 Vite。浏览器不提供本机目录扫描、打开目录或 SQLite；不会静默模拟这些能力。

```powershell
npm run check           # lint、皮肤 schema 一致性、类型检查、构建、领域测试
npx playwright install chromium
npm run test:ui         # 真实浏览器交互测试
npm run check:native    # Rust 格式、单元测试、Clippy
npm run desktop:build   # 当前默认生成 Windows NSIS 安装包，未配置签名发布
```

## 已有基础

- 邮件工作区：常用快捷入口、可折叠账号目录树、目录固定 / 重启恢复、统一收件箱、搜索、保存的搜索（本机偏好一键入口）、标签筛选、自建标签（本机注解，侧栏管理与阅读区打标，不写服务器）、阅读、星标、归档与撰写；本机 / 服务器草稿独立；桌面只显示你添加的邮箱，统一入口只汇总真实账号。
- 事件视图：演示待办完成状态、跳转关联邮件；ICS 日历邀请（text/calendar 附件）随同步解析为事件（只读，不做 RSVP）。
- 皮肤：Paper / Graphite / Linen；明暗 / 跟随系统；桌面扫描独立皮肤包、坏包隔离、缺失皮肤回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽、方向键、双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界；阅读正文字号四档可调（设置 → 外观，纯文本与 HTML 正文同时生效）。
- Rust：SQLite v1–v15 迁移、FTS5、Windows 凭据服务、IMAP / POP3 分批增量同步、持久草稿、离线状态意图与受限皮肤扫描。
- 同步调度：后端 FIFO，最多两账号并行，同账号请求合并；同步全部、逐账号进度、排队取消 / 运行中停止、持久请求 / 退避与重启恢复。网络等待与正文解析不占用数据库锁。
- 草稿：停止输入 800ms 后自动保存，关闭撰写面或窗口时先自动保存（失败才拦截并保留内容），显式「放弃草稿」按钮删除草稿；支持手动保存、版本冲突检测。每账号多签名（默认签名自动追加，撰写时可在单封邮件中修改）。
- 发送队列：设置 → 邮箱账号中按账号 / 状态查看全部未发送任务与最近 128 条已发送记录；排队可取消，明确失败可重试或返回原草稿修改。结果不明不自动重发，选「仅保留本机」后不开放普通重试。
- POP3：设置页单账号 / 同步全部及自动间隔可用，只提供单收件箱 UIDL 下载并保留服务器副本；连接重配保留 POP3 协议，不提供 IMAP 文件夹 / 历史或服务器标记选项。
- 邮件状态：本机保存成功后更新界面；下一次手动同步或已开启的自动同步回写已读与星标，归档 / 删除到废纸篓 / 移动到文件夹使用 UID MOVE。列表支持多选批量已读 / 未读 / 星标 / 归档 / 删除 / 移动；冲突保留本机，移动结果不明时禁止盲目重试，阅读区提供状态与处理入口。
- 定位恢复：旧缓存与未知归档支持只读核对、候选展示和再次联网确认；有完整原文指纹时支持跨 UIDVALIDITY 重绑定。候选可分批核对、保存进度并续查，完整检查前不能确认；歧义、证据不足或硬上限时保留本机，不重复移动。
- 指纹补齐与分段：可信原定位仍在时可重新读取完整原文，经二次核对与明确确认补齐指纹；弱正文观察不升级为历史证据。大范围候选按 UID 区间分段，完成原文检查和全段集合复核后才可确认，保留本地修改。
- IMAP 限额：库缓冲前限制 literal / 响应 / 协议行 / SEARCH 输入，超限停止连接并保留已提交数据。密集邮箱采用更小的固定核对区间，保留每页集合检查与完整确认重扫。
- IMAP 收尾：独立目录导航 / 历史回填、缺失 UID 复核、可信指纹跨目录恢复 / 明确自动续页、按大小 FETCH 与分块缓存、附件另存为、原文重取；同步设置默认关闭自动同步，支持 IDLE / CONDSTORE 回退。
- 授权：添加账号按提供商卡片选择（Gmail、QQ、163、其他 IMAP；Office 即将支持），Gmail 公共应用 Client ID 已内置、系统浏览器 PKCE / loopback / XOAUTH2、keyring 刷新与重新授权，密码账号可原地重配，详见 OAuth 配置文档。
- 界面语言：中英文切换，设置 → 外观与皮肤即时生效并持久化；首次启动按系统语言检测。`zh-CN` 为源语言字典，`en-US` 类型强制全覆盖；邮件内容与皮肤 JSON 属数据不翻译。
- 大列表：邮件列表虚拟化（只渲染视口窗口 + overscan，皮肤行高 token 自适应），600+ 封滚动与批量操作保持流畅；各账号分页并行读取（每页 200）。前端仍加载完整快照（按视图懒加载待计数查询支持）。
- 工程：类型契约、检查命令、浏览器回归测试、Windows 原生 CI；数据备份与恢复（数据库快照 + 原文缓存导出为目录，恢复 staging 后重启生效，系统凭据不在备份内）。

**保存边界：**外观与分栏宽度保存在 localStorage；桌面账号、邮件、草稿及已读 / 星标 / 归档的本地修改存 SQLite，密码和 OAuth 刷新令牌存 Windows 凭据管理器，access token 仅内存；完整原文缓存有 512 MiB 配额。浏览器仅提供演示数据，演示草稿、邮件状态与事件完成状态存 localStorage（桌面版事件完成状态存 SQLite）。撰写在右栏工作区完成（Outlook 式），支持收件人标签、账号内联系人建议、抄送 / 密送和展开编辑；正文上方富文本工具栏提供加粗 / 标题 / 列表 / 引用 / 颜色 / 链接等（前端无新增编辑器依赖），HTML 经服务端消毒后随 multipart/alternative 发送；撰写附件（桌面版，每封 ≤20 个、合计 16 MiB）经 Rust 对话框选取、随草稿存 SQLite、发送时组装 multipart/mixed；已接入 SMTP 发送队列（outbox：结果不明不自动重发，可经 Sent 目录核对或手动确认）；尚未实现正文图片插入。收到的附件可从完整原文另存为；图片与文本类附件支持内联预览（白名单类型，8 MiB 上限）。收到的邮件支持 HTML 沙箱渲染：正文从已验证的完整原文缓存按需提取并消毒，在唯一源沙箱 iframe 中显示；远程图片默认拦截、每封可手动允许，缓存被淘汰时回退纯文本（链接可点击，统一经系统应用打开）。

## 开发入口

| 文件 | 用途 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | 所有后续开发应遵循的约定 |
| [docs/STATUS.md](docs/STATUS.md) | 当前进度、验证证据、下一步入口 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 模块边界、数据流、IPC、数据库 |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | 环境、日常流程、检查和完成标准 |
| [docs/UI_GUIDELINES.md](docs/UI_GUIDELINES.md) | 界面、组件、排版、交互和可访问性约定 |
| [docs/SKIN_SPEC.md](docs/SKIN_SPEC.md) | 可实际使用的 v1 皮肤规范与安装步骤 |
| [docs/PROVIDER_CONTRACTS.md](docs/PROVIDER_CONTRACTS.md) | 多账号、协议、OAuth、标签和 AI 接入约定 |
| [docs/NAVIGATION.md](docs/NAVIGATION.md) | 常用入口、账号目录树、目录映射、草稿区分与导航偏好 |
| [docs/COMPOSING.md](docs/COMPOSING.md) | Outlook 式撰写、收件人编辑、抄送密送与验证边界 |
| [docs/DRAFTS.md](docs/DRAFTS.md) | 草稿持久化、自动保存、冲突、迁移和退出保护 |
| [docs/SYNCHRONIZATION.md](docs/SYNCHRONIZATION.md) | 同步锁边界、多账号队列、批次提交、取消和回归测试 |
| [docs/MESSAGE_CHANGES.md](docs/MESSAGE_CHANGES.md) | 邮件状态持久化、离线意图、IMAP 回写与冲突处理 |
| [docs/REMOTE_RECOVERY.md](docs/REMOTE_RECOVERY.md) | 旧缓存验证、未知归档核对、明确恢复及副本合并 |
| [docs/IMAP_COMPLETION.md](docs/IMAP_COMPLETION.md) | 本轮 IMAP 收尾验收范围与剩余事项 |
| [docs/SENDING.md](docs/SENDING.md) | SMTP 发送与 outbox 设计：未知发送结果语义、幂等与验收标准 |
| [docs/OAUTH_SETUP.md](docs/OAUTH_SETUP.md) | Google / Microsoft 公共应用注册、授权与安全边界 |
| [docs/IMAP_LIMITS.md](docs/IMAP_LIMITS.md) | IMAP 流式限额、失败保留及密集邮箱分段成本 |
| [docs/ROADMAP.md](docs/ROADMAP.md) | 分阶段开发与验收 |
| [docs/decisions/0001-foundation.md](docs/decisions/0001-foundation.md) | 基础技术决策与取舍 |

## 添加皮肤

在桌面版打开“外观与皮肤 → 打开皮肤目录”，把 `examples/skins/ocean` 整个文件夹复制进去，点击“重新扫描”。无需修改应用或重新打包。格式见 [皮肤规范](docs/SKIN_SPEC.md)。

代码已初始化 Git 并托管在 GitHub（远程地址见 `git remote -v`）；发布打包与云服务仍未配置。实际依赖版本以 `package-lock.json` 和 `src-tauri/Cargo.lock` 为准。
