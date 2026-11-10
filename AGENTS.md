# AuuMail 工作约定

开始工作先读 `README.md`、`docs/STATUS.md` 和本次修改涉及的专项文档。本文适用于整个仓库。

## 产品与边界

- 本地优先的 Tauri 邮件桌面客户端，沿用 AuuGit 的技术栈与桌面设计语言。
- 当前为基础架构阶段，所有邮箱均为明确标识的演示数据。不得把模拟连接、发送或 AI 结果伪装成真实成功。
- 没有明确任务时，不提前实现真实账号授权、云端上传、自动发送或自动发布。
- 后续交接更新 `docs/STATUS.md`，准确区分完成、预留、未验证。

## 分层

- `src/domain` 是无 React、Tauri、存储依赖的领域契约。
- `features` 和 `components` 通过 store / gateway 使用业务能力，不直接调用 `invoke`、网络、文件系统或数据库。
- Tauri API 仅位于 `src/platform`；演示服务位于 `src/services/demo`。
- Zustand 保存界面状态与当前视图；真实邮件、同步游标和发送队列由 Rust / SQLite 管理。
- `MailMessage.mailbox` 是当前 UI 的简化投影。持久化允许同一邮件属于多个文件夹；接入 Gmail 时不要把简化字段当成数据库模型。
- 密码、OAuth refresh token、AI Key 只能通过未来系统凭据服务保存，不得进入 localStorage、SQLite 普通字段、源码或日志。

## UI 与皮肤

- 先读 `docs/UI_GUIDELINES.md`、`docs/SKIN_SPEC.md`；共享组件放在 `components/ui`。
- 禁止业务组件硬编码皮肤颜色、外部字体地址或私有样式覆盖。使用语义 token 和共享状态样式。
- 保持 Radix 弹窗与 Tabs 的键盘行为，图标按钮必须有可访问名称，焦点可见，尊重 reduced motion。
- 新交互覆盖空、加载、失败、禁用、选中和长内容状态。按钮必须执行实际行为；尚未提供的能力明确说明或禁用。
- 皮肤是 JSON 数据，禁止执行 JS、任意 CSS 或网络资源。改 token 同步更新校验、内置包、示例、生成 schema 和文档。
- 当前中文界面集中在功能文件；加入第二种语言前建立消息字典，不临时混用多套 i18n 方案。

## 实现与验证

- TypeScript strict，避免 `any`；跨 IPC 的结构变更同时修改 Rust 和 TypeScript。
- 领域 ID 是不透明字符串；同步远端 ID 必须包含账号和协议作用域。时间存 UTC ISO-8601，事件额外保存 IANA 时区。
- 数据库迁移按版本追加，不重写已发布迁移；新版本数据库不得被旧程序静默降级。每个连接启用 foreign_keys。
- `npm run check` 必须通过。UI 改动运行相关 `npm run test:ui`；Rust / SQL 改动运行 `npm run check:native`。
- 测试验证用户行为和关键边界（账号隔离、坏皮肤、草稿恢复、迁移），不要为纯装饰写实现镜像测试。
- 不提交依赖目录、构建输出、本机数据、真实邮件、凭据或用户皮肤目录。提交两份 lockfile。
- Windows 文件操作使用 PowerShell 原生命令；删除或移动前确认绝对目标在工作目录内。

## 完成标准

功能可实际操作、错误不丢编辑内容、检查通过、文档与当前行为一致，交接中列明未完成事项。详见 `docs/DEVELOPMENT.md`。
