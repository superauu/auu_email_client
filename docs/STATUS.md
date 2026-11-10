# 当前状态

更新时间：2026-11-10（第四轮后）。阶段 A2 完成：IMAP / SMTP 账号可通过向导做真实 TLS 连接验证并保存本机；邮件同步（A3）未接入。

## 已完成（本轮已验证）

- 邮件工作区：统一收件箱、多账号切换、搜索、标签筛选、阅读、星标、归档、撰写和本机演示草稿。
- 事件视图：演示待办完成状态、跳转关联邮件。
- 皮肤：Paper / Graphite / Linen 内置包；明暗 / 跟随系统；桌面端扫描独立皮肤包、坏包隔离、缺失回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽 / 方向键 / Home·End / 双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界；主工作区全宽阅读窗格；Auu 字母 A 品牌标与 Orbitron 品牌字。
- **账号向导（阶段 A2）**：IMAP / SMTP 真实 TLS 连接验证（native-tls 走系统证书库）；IMAP 用 imap 2.4，SMTP 为自实现状态机（greeting/EHLO/AUTH PLAIN/QUIT，多行响应解析带长度上限）；两端独立出结果并区分 network / auth 错误；验证通过后账号写入 SQLite（provider=imap，profile 含服务器端点与能力），密码只写入 Windows 凭据管理器；保存路径自带二次验证并带补偿回滚；真实账号支持两步确认删除（级联清库 + 删凭据）。
- Rust 协议单测：MockStream 脚本化 IMAP（经真实 imap crate）与 SMTP 会话、凭据拒绝映射 auth、协议垃圾映射 network、host 校验、完整 verify 双端独立报告；不依赖任何真实服务器。
- 桌面交互：应用框架禁止文本选择，分隔条拖拽锁定光标且不产生选区。
- Rust / SQLite：固定数据目录、逐版本迁移与 FTS5 索引、受限皮肤扫描命令。

### 验证记录（2026-11-10，Windows / Node 20.19 / Rust 1.98）

- `npm run check`：lint、皮肤 schema 一致性、类型检查、生产构建、11 个领域测试，全部通过。
- `npm run check:native`：cargo fmt --check、cargo test（23 个 Rust 测试：迁移 / 仓库 / 分页 / 隔离 / FTS 转义 / 凭据真实读写 / **IMAP+SMTP 协议会话脚本测试**）、clippy（-D warnings），全部通过。
- `npm run test:ui`：7 个 Playwright 测试全部通过（含侧栏拖拽 / 双击复位 / 刷新持久化）。
- **桌面端到端实测**（`tauri dev` + WebView2 CDP 连接验证）：
  - 工作区：收件箱 6 封邮件经分页 IPC 读自 SQLite；搜索"呼吸感"FTS 命中 1 封；账号切换隔离正确；状态栏显示"桌面工作区"。数据库落盘验证：`%APPDATA%/com.auu.mailclient/mail.db`，`user_version=2`。
  - **账号向导错误路径**：对 `127.0.0.1:1`（无服务端口）执行真实"测试连接"，IMAP 与 SMTP 双双返回真实 TCP 拒绝错误（os error 10061）并展示在报告区——错误链路端到端为真，无任何伪造成功。
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

- **邮件同步（阶段 A3）**：增量同步、正文按需读取、文件夹拉取入库、sync_states 游标、断线重试。账号向导已能在 IMAP LIST 中数出文件夹（报告区展示），但不入库。
- 发送队列与真实发信（阶段 A4，outbox 表已建）；富文本、附件、自动保存。
- AI 能力（界面按钮已占位，未调用任何模型）；日历同步。
- 星标 / 已读 / 归档的跨会话持久化。
- UI 侧消息增量加载与虚拟列表（当前桌面 gateway 内部循环取页拼快照，数据量增大前可接受）。

## 未验证

- **真实邮件服务器上的成功连接**：协议状态机与错误路径已用脚本化 MockStream 单测 + 本机真实 TCP 拒绝实测覆盖；成功路径（TLS 握手 + 真实登录 + LIST）需要你的真实测试账号，属阶段 A5 验收。
- STARTTLS（143/587）路径：当前仅隐式 TLS（993/465）。
- `npm run desktop:build`（打包与 NSIS 安装包未在本轮运行；桌面 dev 窗口已实测）。
- 皮肤包桌面扫描依赖 Tauri 命令，浏览器预览无法覆盖（单测与 schema 校验已覆盖数据层）。
- macOS / Linux（keyring 与打包按阶段 D 扩展）。

## 已知限制（保存边界）

- 外观、列表宽度、手动保存的演示草稿存于浏览器/WebView localStorage。
- 星标、已读、归档、事件完成状态仅本次会话有效。
- 密码、OAuth refresh token、AI Key 的安全保存依赖未来的系统凭据服务，当前无任何凭据存储。

## Git / 发布

- 本轮完成 Git 初始化并推送到 GitHub（远程仓库见 `git remote -v`）。发布打包、CI 发布流水线与云服务仍未配置。
