# 独立皮肤规范 v1

状态：已实现。皮肤是纯 JSON 数据包，不含可执行插件。v1 支持颜色、圆角和密度；字体、背景图片、材质、动效和 preview.png 属于后续规范扩展，当前不会加载。不要提前向包中加入未支持字段。

## 安装与发现

1. 运行桌面版，进入外观与皮肤，打开皮肤目录。
2. 将 `examples/skins/ocean` 整个目录复制到该目录下。
3. 点击重新扫描，选中 Ocean。应用无需重新编译或打包。
4. 修改文件后再次扫描；删除当前皮肤并扫描后会显示内置 Paper。保留原偏好 ID，重新安装该皮肤后可恢复选择。

```text
<应用数据目录>/skins/
  ocean/
    manifest.json
    tokens.json
```

目录名必须等于 manifest.id。内置保留 ID：paper、graphite、linen；用户包不能覆盖它们。浏览器预览只含内置包，无法扫描操作系统目录。当前在启动和点击重新扫描时发现皮肤，尚未实现文件监听、ZIP 导入和在线市场。

## Manifest

```json
{
  "schemaVersion": 1,
  "id": "my-skin",
  "name": "我的皮肤",
  "version": "1.0.0",
  "author": "Your Name",
  "description": "一款用于长时间阅读的皮肤。"
}
```

所有字段必需；可加用于编辑器提示的 `$schema` 字符串。ID 为 2–64 位小写字母、数字或连字符，首位为字母或数字。名称 / 作者最多 80 字符，描述最多 200，版本为 x.y.z、最多 32 字符。不允许其他字段。`$schema` 不会被请求、读取或执行；复制到用户目录后可以删去此字段。

## Tokens

`tokens.json` 至少提供 light 或 dark 中一种，每种必须完整提供全部 token；不合并残缺调色板。只支持单模式时，选择另一模式会使用该包已有模式。

| 颜色 token | 用途 |
| --- | --- |
| canvas | 工作区底色 |
| surface / surface-raised | 面板 / 浮层 |
| surface-hover / surface-selected | 悬停 / 选中 |
| rail | 活动栏 |
| text / text-muted / text-subtle | 正文 / 次级 / 辅助文字 |
| border | 分隔线和边框 |
| accent / accent-foreground | 强调色及其前景 |
| danger / success / warning | 状态色 |

所有颜色必须为六位 `#RRGGBB`。不接受 url()、rgba、CSS 变量引用、HTML、任意 CSS 或外部资源。应用可用 color-mix 从这些颜色生成状态表面。

| 数值 token | 整数范围（px） |
| --- | --- |
| radius-sm | 0–16 |
| radius-md | 0–24 |
| radius-lg | 0–32 |
| control-height | 30–44 |
| list-row-height | 88–128 |

完整可运行模板见 `examples/skins/ocean/tokens.json`。不要仅复制此文档中的 manifest 然后使用不完整的 tokens。

## 校验与失败处理

Rust 只扫描固定用户目录的直接子目录，按名称排序，最多加载 128 个包。每个 JSON 文件最多 128 KiB，路径规范化后必须留在该包内部；无效文件生成逐包 warning，不阻断其余包。扫描目录本身失败时保留上一份有效 catalog，并显示错误。

前端将 unknown JSON 按运行时契约验证，拒绝未知字段、缺失 token、不支持版本、ID 不符、重复 ID 和越界值。只有完整通过验证的包进入 catalog。选中的包缺失时回退到首个内置包；所有变量全量写入，避免上一个皮肤的值泄漏。

v1 不读取图片、字体、preview.png 或脚本。设置页缩略图直接根据已校验 token 绘制；不是从皮肤提供的 HTML 生成。

## 修改规范

`src/themes/contract.ts` 为 token 名称和范围的唯一源头。修改后执行：

```powershell
npm run skins:schema
npm run check
npm run test:ui
```

生成文件为 `schemas/skin-manifest.schema.json` 与 `schemas/skin-tokens.schema.json`，应提交仓库。CI 检查 schema 是否落后于源码。破坏格式的改动必须提升 schemaVersion，并说明兼容策略。

新增内置皮肤需加入 `src/themes/builtins.ts`，外部皮肤不需要这个步骤。每个皮肤检查浅 / 深、选中 / hover / focus / disabled / error 状态及阅读对比度。
