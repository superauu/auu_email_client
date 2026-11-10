# 开发流程

## 环境

- Node.js 20.19+ 或 22.12+，使用 npm 与 package-lock.json；不混用 pnpm / Yarn lockfile。
- 桌面版需要 Rust stable、rustfmt、clippy。Windows 安装 Visual Studio C++ Build Tools / Windows SDK 和 WebView2。
- 本轮环境：Windows、Node 20.19.0、npm 11.21.1、Rust 1.98.1；未验证 macOS / Linux 打包。
- 使用 `npm ci` 安装已锁定依赖。新增依赖前确认现有组件和标准库不能直接完成任务。

前端端口固定为 **4317**，仅监听 127.0.0.1。原先尝试 1420 时本机 Windows 返回 EACCES，因此已统一改用 4317；如需修改，必须同步 Vite、Playwright、Tauri devUrl 与 devCsp。

## 日常工作顺序

1. 阅读 AGENTS、STATUS 与对应功能规范；先明确本轮可验收范围。
2. 跨模块能力先改 domain 契约，再实现 service / platform、store、UI。
3. 涉及数据的操作说明保存位置、失败处理、重试及账号隔离。
4. 运行适用检查，人工查看界面，更新文档和 STATUS；交接不要把“接口预留”写成“功能已支持”。

## 命令

| 命令 | 用途 |
| --- | --- |
| npm run dev | 前端热更新，不连接真实服务 |
| npm run desktop:dev | Rust 桌面 + Vite；不要另外启动占用同端口的 Vite |
| npm run check | lint、schema、完整类型检查、生产构建、所有 tests/*.test.ts |
| npm run test:ui | Chromium 用户行为测试，自动启动 Vite |
| npm run check:native | rustfmt、cargo test、Clippy，warnings 视为失败 |
| npm run skins:schema | 从运行时契约重新生成 JSON Schema |
| npm run desktop:build | 构建 Windows NSIS 安装包；首次编译较慢 |

首次浏览器测试执行 `npx playwright install chromium`。测试结果保存在 test-results，不提交。自动测试采用干净浏览器上下文，不读取个人浏览器数据。

CI 包含 Ubuntu 前端 / Chromium 与 Windows Rust 检查；工作流只是已配置，不代表远程运行已发生。需要真实 Windows WebView 验证时，不以 Chromium 测试代替。

## 编码规范

- UTF-8 / LF；TS 两空格，Rust 使用 rustfmt。
- React 组件 PascalCase，store 为 `xxxStore.ts`，纯逻辑按能力命名。feature 内部组件先就近放置，确有复用再移至 ui。
- 对外类型显式；输入从 unknown 校验；不静默吞掉保存失败；无业务原因不新建全局 store。
- `useShallow` 选择 Zustand 的多字段，避免返回新对象造成额外订阅或循环。
- effect 正确清理事件、订阅与定时器；异步真实请求需要取消和过期响应隔离。
- localStorage key 带 auumail 命名空间及版本；只允许 UI 偏好和本阶段明确标识的演示草稿。
- 新增 Rust 命令不接收任意路径 / shell 字符串，跨 IPC 统一 AppError 和 camelCase。
- 测试中仅用保留示例域名，不提交真实邮箱和截图中的私人信息。

## 完成标准

- 功能行为真实、可操作；未提供的能力有明确状态。
- 错误时保留编辑内容；账号切换不显示旧账号数据；无无效成功提示。
- UI 有空 / 加载 / 错误 / 禁用 / 选中 / 长内容状态，键盘可操作。
- 三种内置皮肤及明暗模式不出现硬编码色或内容溢出。
- 适用检查通过；记录环境限制，不能用“应该通过”替代实际结果。
- STATUS 写明变更、已验证事项、未完成项和下一步文件入口。

## 故障排查

- 端口 EACCES：检查 Windows 保留端口范围；不要为了启动而监听整个局域网。
- 桌面图标缺失：`node node_modules/@tauri-apps/cli/tauri.js icon public/logo.svg -o src-tauri/icons`。
- 数据库版本过新：使用对应新版本程序，不清空真实用户数据库。
- 扫描看不到皮肤：检查目录名 / manifest.id、JSON、规范版本、完整 token 与 warning。
- 演示草稿未恢复：浏览器和桌面 WebView 存储独立；不同 origin 的 localStorage 也独立。当前不会同步到 SQLite。

开发数据清理必须由开发者明确指定目录，先确认目标与保存需求；不在自动启动脚本里删除数据库或重置偏好。
