# AuuMail

一款以漂亮界面、独立皮肤和多邮箱统一管理为中心的本地桌面邮件客户端。技术栈：Tauri 2 / Rust、React 19、TypeScript、Vite 8、Tailwind CSS 4、Radix UI、Zustand、SQLite。

**当前是基础架构预览，使用演示邮箱。真实收发、OAuth、AI 和日历同步尚未接入。**

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
- UI：自定义窗口栏、可调邮件列表宽度、窄窗口阅读切换、可访问弹窗、空状态、错误边界。
- Rust：固定数据目录、SQLite v1 迁移和 FTS5 索引、受限皮肤扫描命令。
- 工程：类型契约、检查命令、浏览器回归测试、Windows 原生 CI。

**保存边界：**外观、列表宽度及手动保存的演示草稿保存在浏览器/WebView 的 localStorage。星标、已读、归档、事件完成状态仅本次会话有效。真实邮箱数据尚未写入 SQLite。撰写界面当前为纯文本、手动保存，未实现富文本、附件和自动保存。

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
| [docs/ROADMAP.md](docs/ROADMAP.md) | 分阶段开发与验收 |
| [docs/decisions/0001-foundation.md](docs/decisions/0001-foundation.md) | 基础技术决策与取舍 |

## 添加皮肤

在桌面版打开“外观与皮肤 → 打开皮肤目录”，把 `examples/skins/ocean` 整个文件夹复制进去，点击“重新扫描”。无需修改应用或重新打包。格式见 [皮肤规范](docs/SKIN_SPEC.md)。

代码已初始化 Git 并托管在 GitHub（远程地址见 `git remote -v`）；发布打包与云服务仍未配置。实际依赖版本以 `package-lock.json` 和 `src-tauri/Cargo.lock` 为准。
