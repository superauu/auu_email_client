# 当前状态

更新时间：2026-11-10（第二轮后）。当前为阶段 A 第 1 步完成：本机数据层就绪，所有邮箱仍为演示数据，未连接真实邮件服务器。

## 已完成（本轮已验证）

- 邮件工作区：统一收件箱、三账号切换、搜索、标签筛选、阅读、星标、归档、撰写和本机演示草稿。
- 事件视图：演示待办完成状态、跳转关联邮件。
- 皮肤：Paper / Graphite / Linen 内置包；明暗 / 跟随系统；桌面端扫描独立皮肤包、坏包隔离、缺失回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽 / 方向键 / Home·End / 双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界；主工作区全宽阅读窗格。
- 桌面交互：应用框架禁止文本选择（阅读区、事件视图、输入框保留），分隔条拖拽期间锁定光标且不产生选区。
- **本机数据层（阶段 A1）**：SQLite v2 迁移（`messages.recipients_json`、`accounts.profile_json`）；Rust 邮件仓库（幂等种子写入、keyset 分页查询、FTS5 短语转义搜索、账号隔离、mailbox 投影）；系统凭据服务（Windows Credential Manager，按 `provider:accountId` 作用域，机密只进不出）；9 个 Tauri 命令（含 `mail_page_messages` 分页 IPC 与凭据三命令）。
- 桌面 gateway：`DesktopMailGateway`（mode `desktop-local`）——演示数据幂等种子进 SQLite 后分页读回；浏览器仍走 `DemoMailGateway`。两个网关共享 localStorage 演示草稿存储。
- Rust / SQLite：固定数据目录、逐版本迁移与 FTS5 索引、受限皮肤扫描命令。

### 验证记录（2026-11-10，Windows / Node 20.19 / Rust 1.98）

- `npm run check`：lint、皮肤 schema 一致性、类型检查、生产构建、11 个领域测试，全部通过。
- `npm run check:native`：cargo fmt --check、cargo test（16 个 Rust 测试：迁移 / 仓库 / 分页 / 隔离 / FTS 转义 / 凭据真实读写）、clippy（-D warnings），全部通过。
- `npm run test:ui`：7 个 Playwright 测试全部通过（含侧栏拖拽 / 双击复位 / 刷新持久化）。
- **桌面端到端实测**（`tauri dev` + WebView2 CDP 连接验证）：桌面窗口正常启动（自定义窗口栏）；收件箱 6 封邮件经分页 IPC 读自 SQLite；搜索"呼吸感"FTS 命中 1 封；账号切换隔离正确；状态栏显示"桌面工作区"。数据库落盘验证：`%APPDATA%/com.auu.mailclient/mail.db`，`user_version=2`，3 账号 / 8 消息 / 2 事件。
- 浏览器预览（`npm run dev`）：界面正常渲染，账号切换、阅读、搜索、空状态可用；Graphite 暗色与 Paper 亮色、222px 与 320px 导航栏宽度目视检查通过。

### 本轮变更（2026-11-10 第二轮）

- 导航栏（左侧边栏）支持拖拽调宽，范围 190–320px，默认 222px；双击分隔条恢复默认；≤1250px 固定 204px 并隐藏分隔条。宽度与列表宽度一并持久化。
- ResizeHandle 泛化为共享组件（label / min / max / onResize / onReset），邮件列表分隔条同样支持双击复位。
- 拖拽分隔条改为 `preventDefault` + 拖拽期间 `body.is-resizing` 锁定光标、禁止文本选择；修复原先拖拽会把正文文字选中的网页化行为。
- 应用框架（`user-select: none`）与长内容区（`user-select: text`）的桌面选择策略；侧栏账号 / 文件夹行超长名称省略号截断。
- localStorage 布局键 `auumail.ui.layout.v1` 兼容旧格式（纯数字 = 旧列表宽度，迁移后侧栏取默认值）。
- 主工作区全宽度：移除阅读区与事件卡片的居中窄栏（`max-width: 850px; margin-inline: auto`），内容从左侧起铺满窗格，符合桌面软件阅读窗格习惯。

### 本轮修复

- `e2e/workspace.spec.ts`：账号按钮“独立邮箱”的选择器由 `exact: false` 改为 `exact: true`。演示邮件《你的独立邮箱已经准备就绪》与账号名撞名，子串匹配触发 Playwright strict mode violation。
- 运维提示：Vite 配置变更后，旧的 dev server 依赖缓存会失效，页面因 `504 Outdated Optimize Dep` 黑屏；关闭旧进程、删除 `node_modules/.vite`、重启 `npm run dev` 即可恢复。

### 本轮变更（2026-11-10 第三轮）

- 品牌对齐 AuuGit：重绘 `public/logo.svg`——以 "Auu" 的字母 A 为主图形（同款深蓝渐变 tile 与 teal 渐变族），A 的横笔化为信封封口折线并交汇于发光节点，区别于 AuuGit 的 git-merge 横笔；品牌词 "AuuMail" 采用本地打包的 Orbitron Variable（`--font-display`），应用于顶栏与关于页；Tauri 全套图标（含 .ico 与 Square/Store 资产）由新 logo 重新生成。

## 预留（结构已就位，未接入）

- 真实邮箱连接、OAuth 授权、真实收发（阶段 A2-A3；凭据服务与账号仓库已就绪，见 `docs/PROVIDER_CONTRACTS.md`）。
- AI 能力（界面按钮已占位，未调用任何模型）。
- 日历同步。
- 富文本撰写、附件、自动保存；当前为纯文本 + 手动保存演示草稿（草稿仍存 localStorage，SQLite drafts/outbox 表随阶段 A4 接线）。
- 星标 / 已读 / 归档的跨会话持久化与增量同步（阶段 A3，sync_states 表已建）。
- UI 侧消息增量加载与虚拟列表（当前桌面 gateway 内部循环取页拼快照，数据量增大前可接受）。

## 未验证

- `npm run desktop:build`（打包与 NSIS 安装包未在本轮运行；桌面 dev 窗口已实测）。
- 凭据服务的账号向导 UI（Rust 服务与 IPC 已有真实读写测试，前端入口随阶段 A2 向导提供）。
- 皮肤包桌面扫描依赖 Tauri 命令，浏览器预览无法覆盖（单测与 schema 校验已覆盖数据层）。
- macOS / Linux（keyring 与打包按阶段 D 扩展）。

## 已知限制（保存边界）

- 外观、列表宽度、手动保存的演示草稿存于浏览器/WebView localStorage。
- 星标、已读、归档、事件完成状态仅本次会话有效。
- 密码、OAuth refresh token、AI Key 的安全保存依赖未来的系统凭据服务，当前无任何凭据存储。

## Git / 发布

- 本轮完成 Git 初始化并推送到 GitHub（远程仓库见 `git remote -v`）。发布打包、CI 发布流水线与云服务仍未配置。
