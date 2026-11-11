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
- localStorage key 带 auumail 命名空间及版本；只允许 UI 偏好及明确标识的演示草稿 / 演示邮件状态，不存真实邮件修改。
- 新增 Rust 命令不接收任意路径 / shell 字符串，跨 IPC 统一 AppError 和 camelCase。
- 草稿改动先读 DRAFTS；桌面写 SQLite，保存核对 revision 和原账号，保存中编辑必须顺序落盘。
- 同步改动先读 SYNCHRONIZATION；DB 锁只覆盖快照 / 短事务，队列锁只覆盖任务状态。网络、keyring 与 MIME 解析必须在锁外；不得另起绕过队列的同步路径。
- 已读 / 星标 / 归档改动先读 MESSAGE_CHANGES；本地状态与意图同事务，服务器结果按字段 revision 确认，旧 FLAGS 响应不得覆盖期间的新编辑。MOVE 不明结果必须 unknown，不自动再次移动。
- 旧缓存与 unknown 归档恢复先读 REMOTE_RECOVERY：只读核对后展示证据，明确确认时再次核对；恢复请求和缓存保护必须经原账号 worker，核对期间编辑使证据失效。合并副本需保留标签、事件及草稿回复引用，有编辑副本不自动丢弃。
- 补齐原文指纹必须有可信原定位，取真实完整原文并重复核对；不能哈希解码正文或将 weak recovery 的摘要升级为历史指纹。分段模式必须完成 collecting 和 validating 两阶段，确认再完整重做；每段 UIDSEARCH 必须限制区间，空区间和早期段也要纳入最终集合摘要。旧确认恢复时重新检查可信来源。
- IMAP 流式修改先读 IMAP_LIMITS：限制必须位于库缓冲之前，超限永久停用连接，不清游标、不截断后报成功、不确认已发出的 MOVE。新区间宽度只用于性能且持久固定，不减少页前后集合核对；大检查点 JSON 序列化留在 DB 锁外。修改命令 flush / continuation 行为时补预算边界测试。
- 账号连接配置变化必须使 sync_generation 失效；每批提交需同时校验 generation 和 expected cursor，UIDVALIDITY 清理与游标和邮件在同一事务。
- 调度测试使用通道 / 可控 Promise，覆盖网络挂起期间仍能保存草稿、账号失败隔离、乱序进度；浏览器 IPC mock 测试不能标为真实服务器或原生端到端验证。
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
- 草稿未恢复：浏览器与桌面存储独立；桌面从 SQLite 读取，旧 WebView 演示草稿在相同 origin 下首次导入。旧 localStorage 保留备份，导入成功后不反复覆盖本机新版本。
- 邮件状态待回写：本机保存与服务器确认分开；手动同步账号处理 queued / failed，conflict / unknown 需核对。仅本机覆盖会持续屏蔽该字段的远端观察值。v4 旧缓存缺少已验证定位时不猜 UID，详见 MESSAGE_CHANGES。
- 定位恢复：原文 SHA-256 缺失时不能跨 UIDVALIDITY 绑定；scanning 时只能继续或重新开始，不能确认。每页复核候选集合，确认也要完整重扫；后页重复、集合变化或业务限额保留 unresolved。流式限额是 network 失败，保留上一完整页；旧大区间需重新开始才采用密度估算。详见 REMOTE_RECOVERY、IMAP_LIMITS。
- IMAP 响应超过安全限额：检查服务器响应 / 邮件大小；单 literal 16 MiB、单响应 32 MiB，以及协议行 / SEARCH 限制都可能触发。恢复仍有更严格的单封声明大小 1 MiB / 每页 10 MiB。不能关闭限制、跳过候选或手工确认 unknown MOVE；正常下载已有 SIZE 规划 / 8 MiB 分组、256 KiB 分块、64 MiB 原文上限与 512 MiB 缓存，详见 IMAP_LIMITS；不关闭限制或跳过超限 UID。

开发数据清理必须由开发者明确指定目录，先确认目标与保存需求；不在自动启动脚本里删除数据库或重置偏好。

## A3.8 维护入口

先读 IMAP_COMPLETION / SYNCHRONIZATION / IMAP_LIMITS / REMOTE_RECOVERY；账号授权另读 OAUTH_SETUP。v9 仅追加迁移，历史 v1–v8 不改。自动设置不旋转 generation；授权 / 密码重配通过 account_service，每次旋转 authEpoch，原邮件先标缺失，不能在 canonical remote_id 冲突时把旧意图给新邮件。

修改 durable request 须测试同代次 request_version、取消后立即重排、旧完成 / 新 generation、启动 running 恢复和 unknown MOVE。修改池连接须重绑当前 Control；IDLE DONE / SASL continuation 不补新预算，半行超时丢连接。MODSEQ 不能越过本地编辑 / 恢复保护，空增量仍核对 UID 存在性。网络 / keyring / MIME / 缓存 I/O 不持 DB 锁。

测试只用 synthetic 账号 / 邮件 / keyring 项；OAuth UI harness 不打开真实浏览器，不称为服务器验证。真实验收所需 Client ID 在界面配置，无需把账号、授权码、令牌写进仓库。
