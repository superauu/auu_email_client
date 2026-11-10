# ADR 0001：基础工程与扩展边界

日期：2026-11-10。状态：已采用。

## 背景

产品沿用 AuuGit 技术栈和桌面设计方向，但核心业务变为多账号邮件。独立换肤、协议差异、离线数据和未来 AI 不应迫使 UI 重写。

## 决策

1. Tauri 2 + Rust、React 19 + TypeScript + Vite、Tailwind 4、Radix、Zustand。依据现有项目实际依赖建立新工程，不复制 Git 业务模块、发布配置或凭据。
2. 先建立演示 gateway 和无平台领域契约，UI 通过 store 动作访问；真实收发以 Rust 服务接入。当前不拉入尚无调用的邮件协议和模型 SDK。
3. SQLite 是真实数据持久化方向；先建立可测试的版本迁移，不把 fixture 写入真实数据库。演示草稿仅暂存本机 WebView 存储并明确标识。
4. 皮肤使用纯 JSON token 包；内置皮肤编译入前端，用户包由 Rust 固定目录扫描。禁止任意 CSS / JS，保留格式升级路径。
5. 皮肤与明暗模式分离；结构行为不交给皮肤覆盖。三个明显不同的内置包检验同一套变量。
6. Windows 优先验收；浏览器预览用于快速 UI 开发，但不假装具有桌面目录和数据库能力。
7. 真实 OAuth、keyring、HTML 清洗、邮件同步、发送和 AI 作为下一阶段实装，接口预留不代表已经支持。

## 影响

基础工程能独立构建和测试；接真实服务需要补 repositories、IPC 与分页面向用例接口。v1 皮肤不能提供图片 / 字体 / 任意布局，换取稳定和可校验。当前演示邮件状态不跨会话保存，真实数据阶段必须由持久化层解决。

## 验证来源

参考本机 AuuGit 的 package.json、Cargo.toml、index.css 和 themeStore.ts；依赖最终以新工程 lockfile 为准。

Tauri 调用与权限参考：[Rust commands](https://v2.tauri.app/develop/calling-rust/)、[Capabilities](https://v2.tauri.app/security/capabilities/)、[Configuration](https://v2.tauri.app/reference/config/)。Vite 环境要求参考：[Getting started](https://vite.dev/guide/)。
