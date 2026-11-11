# AuuMail

一款以漂亮界面、独立皮肤和多邮箱统一管理为中心的本地桌面邮件客户端。技术栈：Tauri 2 / Rust、React 19、TypeScript、Vite 8、Tailwind CSS 4、Radix UI、Zustand、SQLite。

**当前包含演示工作区与桌面 IMAP 收信 / 同步：TLS / STARTTLS、系统凭据、Google / Microsoft OAuth 配置入口、多目录 / 历史、持久调度 / 停止、IDLE / CONDSTORE、离线标记 / 归档恢复、完整原文缓存与附件另存为。代码与脚本检查通过，真实服务器及 OAuth 授权仍待注册应用 / 测试账号验收。发信、POP3、AI 和日历尚未接入。**

## 运行

Node.js 20.19+（或 22.12+）；桌面开发另需 Rust stable、Windows C++ Build Tools 与 WebView2。本轮验证环境为 Windows / Node 20.19.0 / Rust 1.98.1。其他系统前置依赖见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)。

```powershell
npm ci
npm run dev             # http://127.0.0.1:4317，浏览器预览
npm run desktop:dev     # Tauri 桌面窗口
```

`npm run dev` 和 `npm run desktop:dev` 二选一启动；桌面命令会自行启动 Vite。浏览器不提供本机目录扫描、打开目录或 SQLite；不会静默模拟这些能力。

```powershell
npm run check           # lint、皮肤 schema 一致性、类型检查、构建、领域测试
npx playwright install chromium
npm run test:ui         # 真实浏览器交互测试
npm run check:native    # Rust 格式、单元测试、Clippy
npm run desktop:build   # 当前默认生成 Windows NSIS 安装包，未配置签名发布
```

## 已有基础

- 邮件工作区：统一收件箱、三账号切换、搜索、标签筛选、阅读、星标、归档、撰写和本机演示草稿。
- 事件视图：演示待办完成状态、跳转关联邮件。
- 皮肤：Paper / Graphite / Linen；明暗 / 跟随系统；桌面扫描独立皮肤包、坏包隔离、缺失皮肤回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽、方向键、双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界。
- Rust：SQLite v1–v9 迁移、FTS5、Windows 凭据服务、IMAP 分批增量同步、持久草稿、离线状态意图与受限皮肤扫描。
- 同步调度：后端 FIFO，最多两账号并行，同账号请求合并；同步全部、逐账号进度、排队取消 / 运行中停止、持久请求 / 退避与重启恢复。网络等待与正文解析不占用数据库锁。
- 草稿：停止输入 800ms 后自动保存，支持手动保存、失败保留输入、版本冲突检测及窗口关闭保护。
- 邮件状态：本机保存成功后更新界面；下一次手动同步或已开启的自动同步回写已读与星标，归档使用 UID MOVE。冲突保留本机，归档结果不明时禁止盲目重试，阅读区提供状态与处理入口。
- 定位恢复：旧缓存与未知归档支持只读核对、候选展示和再次联网确认；有完整原文指纹时支持跨 UIDVALIDITY 重绑定。候选可分批核对、保存进度并续查，完整检查前不能确认；歧义、证据不足或硬上限时保留本机，不重复移动。
- 指纹补齐与分段：可信原定位仍在时可重新读取完整原文，经二次核对与明确确认补齐指纹；弱正文观察不升级为历史证据。大范围候选按 UID 区间分段，完成原文检查和全段集合复核后才可确认，保留本地修改。
- IMAP 限额：库缓冲前限制 literal / 响应 / 协议行 / SEARCH 输入，超限停止连接并保留已提交数据。密集邮箱采用更小的固定核对区间，保留每页集合检查与完整确认重扫。
- IMAP 收尾：独立目录导航 / 历史回填、缺失 UID 复核、可信指纹跨目录恢复 / 明确自动续页、按大小 FETCH 与分块缓存、附件另存为、原文重取；同步设置默认关闭自动同步，支持 IDLE / CONDSTORE 回退。
- 授权：系统浏览器 PKCE / loopback / XOAUTH2、keyring 刷新与重新授权，密码账号可原地重配；需先注册公共应用 Client ID，见 OAuth 配置文档。
- 工程：类型契约、检查命令、浏览器回归测试、Windows 原生 CI。

**保存边界：**外观与分栏宽度保存在 localStorage；桌面账号、邮件、草稿及已读 / 星标 / 归档的本地修改存 SQLite，密码和 OAuth 刷新令牌存 Windows 凭据管理器，access token 仅内存；完整原文缓存有 512 MiB 配额。浏览器仅提供演示数据，演示草稿与邮件状态存 localStorage。事件完成状态仍仅本次会话有效。撰写界面为纯文本，尚未实现发送、富文本和撰写附件；收到的附件可从完整原文另存为。

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
| [docs/DRAFTS.md](docs/DRAFTS.md) | 草稿持久化、自动保存、冲突、迁移和退出保护 |
| [docs/SYNCHRONIZATION.md](docs/SYNCHRONIZATION.md) | 同步锁边界、多账号队列、批次提交、取消和回归测试 |
| [docs/MESSAGE_CHANGES.md](docs/MESSAGE_CHANGES.md) | 邮件状态持久化、离线意图、IMAP 回写与冲突处理 |
| [docs/REMOTE_RECOVERY.md](docs/REMOTE_RECOVERY.md) | 旧缓存验证、未知归档核对、明确恢复及副本合并 |
| [docs/IMAP_COMPLETION.md](docs/IMAP_COMPLETION.md) | 本轮 IMAP 收尾验收范围与剩余事项 |
| [docs/OAUTH_SETUP.md](docs/OAUTH_SETUP.md) | Google / Microsoft 公共应用注册、授权与安全边界 |
| [docs/IMAP_LIMITS.md](docs/IMAP_LIMITS.md) | IMAP 流式限额、失败保留及密集邮箱分段成本 |
| [docs/ROADMAP.md](docs/ROADMAP.md) | 分阶段开发与验收 |
| [docs/decisions/0001-foundation.md](docs/decisions/0001-foundation.md) | 基础技术决策与取舍 |

## 添加皮肤

在桌面版打开“外观与皮肤 → 打开皮肤目录”，把 `examples/skins/ocean` 整个文件夹复制进去，点击“重新扫描”。无需修改应用或重新打包。格式见 [皮肤规范](docs/SKIN_SPEC.md)。

代码已初始化 Git 并托管在 GitHub（远程地址见 `git remote -v`）；发布打包与云服务仍未配置。实际依赖版本以 `package-lock.json` 和 `src-tauri/Cargo.lock` 为准。
