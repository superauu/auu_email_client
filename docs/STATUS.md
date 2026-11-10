# 当前状态

更新时间：2026-11-10。当前为基础架构预览阶段，所有邮箱均为本机演示数据，未连接真实邮箱。

## 已完成（本轮已验证）

- 邮件工作区：统一收件箱、三账号切换、搜索、标签筛选、阅读、星标、归档、撰写和本机演示草稿。
- 事件视图：演示待办完成状态、跳转关联邮件。
- 皮肤：Paper / Graphite / Linen 内置包；明暗 / 跟随系统；桌面端扫描独立皮肤包、坏包隔离、缺失回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽 / 方向键 / Home·End / 双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界。
- 桌面交互：应用框架禁止文本选择（阅读区、事件视图、输入框保留），分隔条拖拽期间锁定光标且不产生选区。
- Rust / SQLite：固定数据目录、v1 迁移与 FTS5 索引、受限皮肤扫描命令。

### 验证记录（2026-11-10，Windows / Node 20.19 / Rust 1.98）

- `npm run check`：lint、皮肤 schema 一致性、类型检查、生产构建、11 个领域测试，全部通过。
- `npm run check:native`：cargo fmt --check、cargo test、clippy（-D warnings），全部通过。
- `npm run test:ui`：7 个 Playwright 测试全部通过（含新增侧栏拖拽 / 双击复位 / 刷新持久化）。
- 浏览器预览（`npm run dev` → http://127.0.0.1:4317）：界面正常渲染，账号切换、阅读、搜索、空状态可用；Graphite 暗色与 Paper 亮色、222px 与 320px 导航栏宽度目视检查通过。

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

## 预留（结构已就位，未接入）

- 真实邮箱连接、OAuth 授权、真实收发（见 `docs/PROVIDER_CONTRACTS.md` 的接入约定）。
- AI 能力（界面按钮已占位，未调用任何模型）。
- 日历同步。
- 富文本撰写、附件、自动保存；当前为纯文本 + 手动保存演示草稿。
- SQLite 真实邮件读写；Rust / SQLite 侧结构就绪，演示邮件尚未落库。

## 未验证

- `npm run desktop:dev` / `npm run desktop:build`（本轮未启动 Tauri 桌面窗口与打包；WebView2 / Rust 工具链在本机已安装，`check:native` 通过）。
- 皮肤包桌面扫描依赖 Tauri 命令，浏览器预览无法覆盖（单测与 schema 校验已覆盖数据层）。

## 已知限制（保存边界）

- 外观、列表宽度、手动保存的演示草稿存于浏览器/WebView localStorage。
- 星标、已读、归档、事件完成状态仅本次会话有效。
- 密码、OAuth refresh token、AI Key 的安全保存依赖未来的系统凭据服务，当前无任何凭据存储。

## Git / 发布

- 本轮完成 Git 初始化并推送到 GitHub（远程仓库见 `git remote -v`）。发布打包、CI 发布流水线与云服务仍未配置。
