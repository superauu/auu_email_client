# 当前状态

更新时间：2026-11-11。A3.8 IMAP 收信 / 同步整批收尾已完成代码与自动检查；真实服务器、OAuth 注册应用、原生 WebView 和性能验收仍未完成。完整范围及明确剩余事项见 [IMAP_COMPLETION](IMAP_COMPLETION.md)。以下 A3.7 及更早是历史记录，当前行为以本节和更新的专项文档为准。

## 最新增量：IMAP 整批收尾（A3.8）

- TLS 与强制 STARTTLS、握手 / 命令 / worker 预算、运行中停止；SASL 和 IDLE DONE 不能重置逻辑命令响应配额。超限 / 半行超时后永久丢弃连接，保留已提交内容。
- 下载先读 SIZE，按 8 MiB 计划分批，较大邮件 256 KiB 精确分块写临时文件；64 MiB 原文上限、完整 SHA-256、512 MiB 缓存配额 / 崩溃临时清理。附件 MIME 元数据、显式另存为与原文重新下载入口已接通；原文重取不升级弱历史证据。
- 独立可访问目录作用域 / 导航、UTF-7 显示、不可用目录保留、可选历史向前回填；FLAGS 缺失需额外 UID 存在性核对，远端删除保留本机及修改。服务器草稿可读取和复制为独立本地草稿，没有 APPEND 上传。
- v9 持久同步请求、重启恢复、指数退避、明确启用自动同步、运行中停止、请求版本竞争保护；仍全局两 worker、同账号互斥。迟到旧结果不能清理新请求。
- IDLE 5 秒公平片段、最多 64 个代次隔离会话、无能力 / 拒绝时定时回退；CONDSTORE / CHANGEDSINCE 加速，保留存在性检查，编辑竞争 / 水位失效回完整标记读取。
- 完整可信指纹可在源仍可访问的全部可访问目录中核对外部移动，固定范围 / 全段摘要 / 重复拒绝 / 明确确认重扫均保留。自动续页需用户启动，首次检查到 located 必停；局部匹配不自动绑定。
- Google / Microsoft 公共客户端 PKCE + loopback + XOAUTH2、keyring 刷新令牌 / 轮转、重新授权 / 密码连接重配与失败补偿。没有内置 Client ID，真实授权待应用注册配置与测试；配置指南见 [OAUTH_SETUP](OAUTH_SETUP.md)。
- 重配每次旋转账号 generation，UID 复用时保留原 ID / 正文 / 意图并隔离旧缓存，不把本机修改赋给新账号邮件；恢复中的缓存先保护。
- 更新 README、架构、开发、UI、同步、恢复、限额、接入、路线与 AGENTS；新增收尾验收单和 OAuth 配置文档。未改 v1–v8 历史迁移，新增 009；Cargo.lock 已更新，package-lock.json 无依赖变化并保留。

### 验证证据

- npm run check：通过 lint / skin schema / 类型检查 / 生产构建及 **34 个 TypeScript 测试**。
- npm run check:native：通过 rustfmt、**159 个 Rust 测试**、Clippy -D warnings。imap-proto 0.10.2 future-incompatibility 和 Windows linker stdout 提示仍存在。
- npm run test:ui：**28 个 Chromium 回归通过**；已目视检查同步设置、STARTTLS / 仅收信验证和 OAuth 等待截图。浏览器使用 test-only IPC harness。
- 新增相对 A3.7：41 个 Rust、2 个 TS、5 个 UI 回归。覆盖实际 17 MiB 原文、分块错位 / 截断 / UID / 命名空间、磁盘配额 / 半文件清理、附件路径 / 账号归属、原文下载新编辑竞争、目录 / 历史、删除独立复核、跨目录重复 / 确认重扫 / 自动续页、取消 / 重排 / 代次、IDLE / MODSEQ 回退、OAuth PKCE / state / scope / SASL、keyring 补偿、v8 旧库升级等。
- 测试仅使用 synthetic 邮件 / 协议脚本、临时 SQLite 与自行清理的 synthetic 系统凭据；没有读取真实邮箱、申请真实授权或发送邮件。

### 使用默认值与未完成事项

1. 新建 / 升级真实账号默认同步全部目录，历史下载 / 自动同步默认关闭；设置开启后生效。手动停止暂停该账号，需手动同步恢复。退出重启续跑已登记请求；授权失败 / 冲突暂停，网络退避最多 8 次。
2. 真实 TLS / STARTTLS、Gmail / Microsoft 授权 / 刷新 / 撤销、FLAGS / MOVE、ACL / 限流和大邮箱吞吐需授权测试账号验收。缺 Client ID 时授权按钮要求配置，不伪装可用。
3. 前端 gateway 仍聚合全部分页为快照，虚拟列表 / 按视图加载未做；64 MiB 原文解析仍有额外内存，不是总 RSS 上限。恢复单候选 1 MiB / 页 10 MiB 更严，不能跳过超限候选。
4. 无服务器目录 CRUD、任意 MOVE / 永久删除 / EXPUNGE、keywords / 原生 Gmail 标签、QRESYNC、服务器草稿上传、HTML 沙箱、附件预览；跨目录无事务快照，源目录失效不猜重命名。IDLE 仅监测 INBOX，其余目录定时核对。
5. SMTP / outbox、POP3、Graph / Gmail API、AI、日历仍未开发；原生 WebView、安装包与 macOS / Linux 未新增验收。

## 历史增量：流式限额与密集分段优化（2026-11-11，A3.7）

- 新增 imap_limits，放在 IMAP 库缓冲之前，覆盖账号验证和生产同步 / 回写 / 恢复。单 literal 16 MiB、单命令响应 32 MiB、协议行 1 MiB / 10000 行、SEARCH token 合计 50000，单次上游读取最多 8192 字节；保留现有 socket 超时，没有新增依赖或迁移。
- 分片 literal 声明、整数溢出、无换行长行、多行 SEARCH 都在输入层拒绝；跳过 literal 正文中的伪协议，普通 OK 文本不能伪装 literal 绕过 SEARCH 计数。超限后连接永久失效，不能截断后成功或复用剩余输入。
- 保留本机邮件、意图、失败批次前的游标和上一完整恢复检查点；不提交当前不完整页，不提前清理旧 UIDVALIDITY 缓存。MOVE 即使先收到 COPYUID，后续超限仍 unknown、不重复移动。恢复大小 1 MiB / 页合计 10 MiB、实际长度、UID、可信完整原文来源等业务校验未放宽。
- 新 ALL 分段按 UIDNEXT / EXISTS 密度选择固定 1024–50000 数值 UID 区间；密集邮箱即使低于 50000 也可分段。有 Message-ID 提示时保持宽区间，避免以整箱密度制造大量空段。EXISTS 不作绑定证据；旧检查点继续原宽度与摘要，默认宽度省略的 JSON 兼容。
- 页前 / 页后完整集合检查、collecting + validating 全段滚动摘要、最终原文复核、明确确认重扫、generation / revision 保护均保留。没有减少候选范围、选择重复中的第一份或开放部分结果确认。
- 避免格式探测额外构建完整 JSON Value 树与当前段列表克隆；分页 / 分段检查点序列化移到 DB Mutex 外，持锁阶段仍仅做核对与短事务。
- 新增 IMAP_LIMITS 专项文档，更新恢复、同步、架构、开发、接入、邮件状态、路线、README 与 AGENTS。

### 验证结果

- `npm run check` 通过：lint / schema / 类型检查 / 生产构建，32 个 TS 测试。
- `npm run check:native` 通过：rustfmt、118 个 Rust 测试（新增 15 个）、Clippy -D warnings。既有 imap-proto future-incompatibility 与 Windows 链接器提示仍存在。
- `npm run test:ui` 通过：23 个 Chromium 回归。本轮未修改 UI / IPC 契约，浏览器仍为 test-only IPC harness，不称为真实服务器或 WebView 验收。
- 新增协议输入测试覆盖超大 / 溢出声明在读取正文前中止、跨分片、正文伪 SEARCH / COPYUID、超长无换行行、多行累计 SEARCH、完整 50000 UID、LIST literal、响应合计与行数预算、命令间重置和失效后禁止读写。
- 原生脚本验证超限保留完整检查点 / 本地星标并可明确续查、失败拉取不推进旧游标 / 清理缓存、COPYUID 后超限仍 unknown 且不重发 MOVE；网络步骤检查 DB 未持锁。密集 50001 / 4096 EXISTS 的脚本使用 1024 区间，其中 50001 场景实际连续检查两页原文，验证检查点小于 10 KB、EXISTS 改变不重新划段；另覆盖 HEADER 提示保持宽区间、页集合变化 / 坏宽度继续拒绝。
- 既有跨代可信来源、离线意图、跨段重复、全段复核、确认重扫、重开 / SQL 回滚、账号隔离和调度回归继续通过。没有使用真实邮件账号或服务器。

### 明确边界与下一步

1. IMAP 库仍在限额内缓冲完整响应，MIME 与解析结构另有内存开销；这不是总 RSS 配额或正文流式写盘。正常大邮件 / 超大批次也可能被拒绝，尚无自动拆小失败 FETCH、大附件落盘或跳过策略，不会报成功收齐。
2. 1024 区间降低密集 ALL 分页重复传输 / 排序 / 检查点规模；原文读取次数及严格集合核对保持。HEADER 低选择性、稀疏 UID、高延迟与持续新信仍可能很慢；测试不是五万封完整原文吞吐基准，没有真实服务端性能 / 差异验收。
3. 旧宽区间检查点不在中途改变边界，重新开始才采用新的密度策略；缺少有效 UIDNEXT 时继续受小快照上限约束。无后台自动续查、MODSEQ / IDLE、任意目录映射或跨目录事务快照。
4. 后续用明确授权的测试账号验收性能及 FLAGS / MOVE / 恢复，设计大邮件受限拉取与附件配额，补 STARTTLS；SMTP / outbox 和未知发送结果语义仍未接入。

以下为 A3.6 及更早记录；与本节冲突时以本节、IMAP_LIMITS 和 REMOTE_RECOVERY 为准。

## 历史增量：安全补齐指纹与大范围分段（2026-11-11，A3.6）

- 追加 v8：locator_verified / content_hash_verified、message_recovery.operation 及分段进度。来源标记保守迁移，不把过去弱恢复观察或无法审计的摘要升级为历史指纹；未改写历史迁移，没有新增依赖。
- 可信原账号代次、文件夹、UIDVALIDITY / UID 仍有效时，可从阅读区核对并补齐完整原文指纹。检查阶段两次读取原 UID 完整 BODY.PEEK，明确确认时再次重复读取和证据比较，最后核对 generation / revision 再写可信摘要。
- 指纹补齐只改指纹元数据，不改本地正文、定位、已读 / 星标 / 归档意图或其 revision。缺少可信锚点、原文差异、命名空间改变、期间编辑或 SQL 失败时保留原数据；不哈希 bodyText 冒充原文。
- 普通弱内容恢复产生的摘要保持未验证，不能用于跨代绑定；旧分页 / 分段确认恢复时再次检查来源，不能绕过迁移后收紧的可信标记。完整同步 FETCH 建立新的完整原文来源。
- 大范围根据 UIDNEXT / UIDVALIDITY 固定边界，使用每段最多 50000 UID 数值的区间 SEARCH；每页最多读取 8 个候选原文，只保留当前段列表与滚动摘要，不创建全局大 UID 快照。
- collecting 完成所有原文后，validating 逐段重新查询完整 UID 集合并比较全段摘要。跨段重复、早期段候选删除 / 新增、UIDNEXT / 代次改变、越界响应都拒绝绑定。确认时重新完整执行两阶段；单段唯一或只完成原文阶段均不开放绑定。
- UI 区分指纹补齐与定位恢复确认，缺少锚点时禁用并说明原因；分段界面显示正文检查 / 全段集合复核进度与独立继续入口。本机修改、请求、检查点仍持久化并复用账号队列，不自动发 MOVE。
- 更新 REMOTE_RECOVERY、MESSAGE_CHANGES、架构、开发、UI、同步、接入、路线、README 与 AGENTS。

### 验证结果

- `npm run check` 通过：lint / schema / 类型检查 / 生产构建与 32 个 TS 测试（新增 1 个），核对补齐专属 action / revision 与本地状态保留。
- `npm run check:native` 通过：rustfmt、103 个 Rust 测试（新增 13 个）、Clippy -D warnings。既有 imap-proto future-incompatibility 与 Windows 链接器提示仍存在。
- `npm run test:ui` 通过：23 个 Chromium 测试（新增 3 个）。覆盖补齐的明确确认、本地星标保留、不可信锚点禁用、50001 候选的分段界面及全段复核未完成 / 失败时无绑定入口。已目视检查补齐与全段复核截图。
- 原生逐命令脚本覆盖补齐重复原文读取、同正文不同原文、定位 / 账号不一致、期间编辑、SQL 回滚、弱证据不升级、跨段确认重扫 / 重复、早期段删除后的摘要失效、UIDNEXT / 越界响应、断线 / 重开和旧确认来源降级保护。
- 真实脚本接收一段 50000 UID 的密集搜索响应并仅读取 8 封原文，另用稀疏跨段样例完整执行 collecting + validating 及确认重做。未遍历真实五万封原文，不称为大邮箱性能验收。v7 升级测试覆盖无恢复记录的有效旧定位回填与有恢复历史的保守处理。
- 既有分页、跨代、同步、调度、草稿及迁移测试继续通过；没有使用真实邮件服务器 / 账号，也没有原生 WebView、安装包或跨平台新增验收。

### 明确边界与下一步

1. 原始全文已丢失且原可信定位也失效时，不能从相似新邮件恢复历史指纹；继续保留本机。无来源标记的旧恢复观察不会静默获得跨代资格，可能需要重新取得可信完整邮件。
2. 分段需要服务器提供有效 UIDNEXT / UIDVALIDITY；单封 1 MiB / 单页声明正文 10 MiB 限制保留，不跳过过大的候选来断言唯一。大 SEARCH / literal 的恶意响应流式硬限额仍未实现。
3. UID 空洞很多或持续新信会使范围扫描较慢 / 保守失效；真实大邮箱吞吐、服务器差异和跨目录快照限制待授权测试账号验收。没有任意目录映射、歧义副本自动选择、后台自动续查、IDLE / MODSEQ。
4. 下一步用明确授权的测试账号验收补齐 / 分段 / FLAGS / MOVE，补 STARTTLS；之后再推进 SMTP / outbox 与未知发送结果语义。

以下为 A3.5 及更早记录；与本节冲突时以本节和 REMOTE_RECOVERY 为准。

## 历史增量：跨代重绑定与分页核对（2026-11-11，A3.5）

- 追加 v7 recovery_scan，保存完整候选集合 / 代次、分页位置、匹配摘要、进度和跨代标识。保留旧 v6 证据读取；没有修改历史迁移或新增依赖。
- 源 UIDVALIDITY 改变时不再查询旧数值 UID 并套用意图；仅在已有完整原文 SHA-256 时搜索新一代候选。完整集合唯一指纹匹配后才提供明确确认，缺少指纹或重复副本则保留本机 / unresolved。
- 每页最多检查 8 个候选，合计快照最多 50000 UID；保留单封 1 MiB 和单页声明正文 10 MiB 上限。原来超过 100 候选的恢复可分页推进，不将第一页的唯一匹配当成全局唯一。
- continue 保留原检查 / 确认 / 明确重排决定；确认阶段重新创建快照并完整重扫、比较集合摘要、复核匹配原文。后页重复、候选增删或 UIDVALIDITY 再变均拒绝旧证据。网络失败可从上一完整页面继续，已提交页面跨重启保留；checking 强制中断仍退回全新 check。
- UI 显示 checked / total / matches、跨代说明和继续下一页；未完成时不出现确认按钮。后台刷新 / 低版本 IPC 不回退较新的核对结果；期间本地编辑使页面过期，不能继续绕过版本检查。
- unknown 归档可对新一代源与保存的目标共同核对；确认恢复目标或明确重排时更新所有待回写定位，包括归档意图本身。保留原本机 ID、正文、修改和既有标签 / 事件 / 回复引用合并规则；核对不发送 MOVE。
- 更新 REMOTE_RECOVERY、MESSAGE_CHANGES、同步、架构、开发、UI、接入、路线、README 与 AGENTS，明确较弱旧缓存证据不能跨代使用。

### 验证结果

- `npm run check` 通过：lint / schema / 类型检查 / 构建，31 个 TS 测试（新增 1 个），覆盖 continue 的最新 revision 和迟到页面不能覆盖更新的失效结果。
- `npm run check:native` 通过：rustfmt、90 个 Rust 测试（新增 12 个）、Clippy -D warnings。既有 imap-proto future-incompatibility 与 Windows 链接器提示仍存在。
- `npm run test:ui` 通过：20 个 Chromium 测试（新增 2 个），覆盖分页进度 / 无提前确认、失败续查、确认阶段继续完整检查和后页重复。已目视检查分页进度、跨代说明和继续入口截图。
- 原生脚本覆盖旧数值 UID 复用不误绑、后页重复、候选集合 / 代次变化、确认重扫、编辑 / 锁边界、断线续查 / SQLite 重开、跨代未知归档的目标恢复与新源重排、50000 快照硬上限、v6 证据 / local 意图升级保留。
- 101 候选完整扫描并确认再扫的回归实际遍历 13 页两次，最终只绑定第 101 个指纹匹配 UID，并保留本机星标。既有恢复、同步、调度、草稿与迁移测试继续通过。
- 未使用真实账号 / 邮件服务器，浏览器仍是 test-only IPC harness；没有新增真实 WebView、安装包或跨平台验收。

### 明确边界与下一步

1. 跨代绑定要求已保存的完整原文指纹。只有解码正文的旧缓存、空正文或歧义副本不强行重绑；保留本地修改和核对说明。
2. 正文核对已分页，IMAP SEARCH 元数据仍由库整批缓冲，并在每页复核完整集合；超过 50000 UID 仍停止，不以部分快照证明唯一。超大 SEARCH / literal 流式限制与真实大邮箱性能待后续验证。
3. 频繁新增 / 删除候选会使整份扫描失效；没有跨目录事务快照、任意目录映射、人工选择歧义副本、后台自动续查或 MODSEQ / IDLE。
4. 下一步用明确授权的测试账号验收跨代、FLAGS / MOVE 和恢复，补 STARTTLS；之后继续 SMTP / outbox，先明确未知发送结果语义。

以下为 A3.4 及更早记录；与本节冲突时以本节和 REMOTE_RECOVERY 为准。

## 历史增量：旧缓存与未知归档恢复（2026-11-11，A3.4）

- v6 迁移追加原文 SHA-256、搜索提示 Message-ID 和 message_recovery 持久请求 / 证据表。没有改写 v1–v5；sha2 升为直接依赖，Cargo.lock 已更新，前端依赖未改变。
- 阅读区可“核对远端定位”，先展示位置与证据，再明确“确认恢复定位”。确认时重新联网核对；低版本响应、候选变化、账号代次改变或期间本机编辑都不能应用旧证据。只读核对复用账号调度，不新增旁路联网 worker。
- 旧缓存核对原文件夹、UIDVALIDITY、UID 与内容。有原文指纹时验证原文；无指纹时要求主题、地址、收件列表、非空正文和内部时间一致，明确提示证据较弱，待用户确认后绑定当前定位。不会从字符串直接猜 UID 或清库。
- unknown 归档检查保存的源与目标，扫描后复核源。源缺失且目标唯一匹配时可明确恢复；源仍在且目标无匹配时可明确“重新排队归档”；重复、两边均有 / 均无、原定位改变或超限均保持 unresolved / unknown。核对与确认不执行 MOVE，恢复意图下次手动同步才回写。
- 恢复保护原缓存，暂停该邮件回写，避免普通 BODY / FLAGS 刷新及 UIDVALIDITY 清理覆盖核对证据。每任务最多 20 请求；候选最多 100 UID，单封 1 MiB、单请求声明正文总量 10 MiB，超限不采用部分结果。
- 确认时原子保留本机 ID、重新定位非 local 意图；未编辑目标缓存可合并标签、事件和回复草稿引用，草稿版本递增。目标有编辑或内容不同则保留两份；SQL 失败完整回滚。
- 核对请求跨重启保留；checking 中断后退回 queued / check 并清空证据，不继续应用旧确认。任务提交失败可重新启动已登记请求，核对途中可仅保留本机取消结果。
- 新增 REMOTE_RECOVERY，更新 MESSAGE_CHANGES、SYNCHRONIZATION、架构、开发、UI、接入、路线、README 和 AGENTS。

### 验证结果

- `npm run check`：lint / schema / 类型检查 / 构建与 30 个 TS 测试通过（新增 2 个），覆盖恢复排在本机保存之后并提交最新 revision、请求失败保留编辑。
- `npm run check:native`：rustfmt、78 个 Rust 测试通过（新增 15 个）、Clippy -D warnings 通过。沿用 imap-proto future-incompatibility 与 Windows 链接器提示。
- `npm run test:ui`：18 个 Chromium 测试通过（新增 3 个），另单独复跑新增恢复测试通过；覆盖初次核对 / 二次确认、未知归档歧义 / 明确重排、已登记请求和失败重试。已目视检查候选位置及确认入口截图。
- Rust 使用逐命令 IMAP 脚本，断言恢复过程不发 STORE / MOVE / EXPUNGE；覆盖原 UID / UIDVALIDITY / 内容 / 文件夹歧义、源目标组合、候选 / 大小上限、原文匹配、账号 / 编辑 / 仅本机取消、回写派发竞争、未编辑副本合并、有编辑副本保留和 SQL 回滚。
- SQLite 覆盖请求 / 待确认结果落盘重开、核对中断降级、账号隔离，以及普通同步不覆盖核对中的正文 / flags 或删除受保护缓存。此前同步 / 草稿 / 调度测试继续通过。
- 浏览器恢复测试使用 test-only IPC harness；没有真实账号、真实邮件服务器或原生 WebView 完整端到端验收。

### 明确边界与下一步

1. 使用明确授权的测试账号验收收信、FLAGS、MOVE / UIDPLUS 及恢复流程，补 STARTTLS 与真实服务器差异验证。
2. 缺少原文指纹的旧缓存只提供缓存字段证据，不能证明未缓存附件 / 其他头部一致；必须明确确认。空正文、原 UIDVALIDITY 已变、原文件夹歧义、超限或有编辑副本继续保留 unresolved，不静默选择候选。
3. 尚无跨 UIDVALIDITY 内容重绑定、任意目录选择 / 重映射、超限候选分页、完整远端删除 / 外部移动追踪、后台自动核对或 MODSEQ / IDLE。IMAP 库仍缓冲响应，尚无恶意超大 literal 的流式硬限制。
4. 同步任务本身仍为进程内队列；持久的是邮件意图和恢复请求。没有定时重试、运行中取消、跨平台凭据或安装包新增验收。
5. 后续再推进 SMTP / outbox，明确未知发送结果语义后开放发送；标签、附件、事件持久化和 AI 不在本轮。

以下为 A3.3 及更早记录；与本节冲突时以本节与 REMOTE_RECOVERY 为准。

## 历史增量：邮件状态与 IMAP 回写（2026-11-11，A3.3）

- 追加 v5 迁移：邮件 state_revision、已验证 IMAP 定位与 message_changes 字段意图。状态和意图在同一事务保存，断网仍可操作，重启保留。演示种子不再删除邮件 / 文件夹或重置本地修改。
- store 按邮件串行保存，成功后更新界面，失败保留原状态并显示错误；后台 reload 合并新版本，避免覆盖期间的本机修改。阅读区新增已读切换、保存 / 待回写 / 错误状态、同步此账号和仅保留本机入口。草稿不提供邮件标记操作。
- 手动同步先处理最多 200 条 queued / failed 意图，沿用同账号互斥与全局两个 worker；本次不自动联网或定时重试。已读 / 星标只增减 Seen / Flagged，STORE 后核对 UID 与目标状态再确认，保留其他 flags。
- 归档要求 MOVE 与唯一归档文件夹；执行 UID MOVE，取得成功完成及 COPYUID 映射后原子更新定位，保留本机 ID / 事件引用，并重定位期间新增的标记意图。不使用 COPY + 整箱 EXPUNGE，不自动创建目录。
- per-field revision / account generation 防止旧确认清除新修改；FLAGS 刷新保护网络期间的本地编辑。UIDVALIDITY 变更保留有本地意图的旧邮件并标冲突，不把修改套用到新 UID。归档断线 / 无映射 / 确认失败为 unknown，不自动重复移动。
- 启动恢复 applying：标记转 queued，归档转 unknown；演示账号仅本机保存，浏览器演示状态按已知 fixture ID 存 localStorage。真实邮件修改只写 SQLite。
- 新增 `docs/MESSAGE_CHANGES.md`，同步更新 README、AGENTS、架构、同步、开发、UI、接入与路线文档。

### 验证结果

- `npm run check` 通过：lint / schema / 类型检查 / 生产构建，28 个 TS 测试（新增 4 个）。
- `npm run check:native` 通过：rustfmt、63 个 Rust 测试（新增 17 个）、Clippy -D warnings。既有 imap-proto future-incompatibility 提示与 Windows 链接器信息仍存在，未新增依赖。
- `npm run test:ui` 通过：15 个 Chromium 测试（新增 3 个）。覆盖离线归档、已读 / 星标 / 归档跨重新加载保留、本机保存失败不丢原状态；IPC harness 覆盖冲突、仅本机、回写重试与同步进度。已目视检查离线归档及回写失败截图。
- Rust 用逐命令脚本会话覆盖断线后幂等标记重试、原 UID 消失、UIDVALIDITY / 账号改变、在途编辑、COPYUID 解析与 literal 隔离、不明 MOVE 不重试；测试在 STORE / MOVE / FLAGS 等网络步骤中验证 DB 未被锁住。
- SQLite 测试覆盖落盘重开、v4 升级保留正文、账号隔离 / 同 ID 重建、字段版本确认、事务失败回滚、applying 崩溃恢复、UIDVALIDITY 冲突保留、稳定 ID 和演示重新初始化。

### 明确未完成 / 未验证

- 未使用真实账号或真实邮件服务器，未验证真实服务器 MOVE / UIDPLUS / ACL、原生 WebView 完整闭环、安装包或跨平台行为。
- v4 缓存缺少已验证远端定位时，修改保存在 conflict，不从字符串猜测可写身份。专用旧缓存重建 / 重新绑定向导未实现；详见 MESSAGE_CHANGES。
- unknown 归档仅提供核对说明和“仅保留本机”；不会自动重试、自动核对或合并可能出现的重复缓存。远端删除 / 外部移动、任意多目录、Gmail 多标签、keywords、CONDSTORE / IDLE、自动退避与持久同步任务恢复仍未实现。
- “仅保留本机”会持续保留本地覆盖，不撤销已经发生的服务器操作。事件完成状态仍只在当前会话。

### 下一步入口

1. 使用明确授权的测试账号验收收信 / 标记 / MOVE，补 STARTTLS 与服务器差异测试。
2. 为旧缓存重新验证定位、unknown 归档核对和远端移动设计专用恢复入口，不通过清库恢复。
3. 继续 SMTP / outbox，明确 unknown 发送语义后再开放发送按钮。

以下为 A3.2、A4.1 及更早记录；与本节冲突时以本节、MESSAGE_CHANGES 和 SYNCHRONIZATION 为准。

## 历史增量：同步锁与多账号调度（2026-11-11，A3.2）

- `mail_sync` 改为短锁快照 → 锁外 keyring / DNS / TLS / IMAP / MIME 解析 → 短事务批次提交。每次 FETCH 最多 100 个 UID 范围，慢网络与退出会话不占用 DB 锁。
- UIDVALIDITY 清理、该批邮件与游标在同一事务提交；失败 FETCH 不先删除旧邮件，后批失败保留先批提交。缺失 / 超范围 / 重复响应不会推进游标。
- v4 迁移新增账号 sync_generation；提交同时核对账号代次与 expected cursor，拒绝删除 / 重建 / 重配账号期间返回的旧网络结果。
- 新增后端 FIFO：两个 worker、同账号活动请求合并、最多 128 个排队任务、保留 64 个终态；独立失败 / 异常隔离，排队可取消，已运行任务继续完成，不自动重试。
- 新增排队 / 进度 / 取消 IPC；兼容 `mail_sync_account` 也进入同一队列。账号页可同步全部、查看各账号状态、取消排队、单独重试；关闭设置后任务继续。
- App 层 syncStore 监测后端，revision 防止乱序覆盖；终态触发合并的后台刷新，刷新不卸载已就绪工作区，保护期间保存的草稿。账号凭据删除 / 补偿也移出 DB 锁。
- 新增 `docs/SYNCHRONIZATION.md`，同步更新 AGENTS、架构、接入、开发、路线与 UI 约定。

### 该阶段验证

- `npm run check`：lint / schema / 类型检查 / 构建与 24 个领域测试通过。
- `npm run check:native`：rustfmt、46 个 Rust 测试、Clippy（-D warnings）通过；既有 imap-proto future-incompatibility 提示仍存在。
- `npm run test:ui`：12 个 Chromium 测试通过，包括多账号状态在设置关闭后保留、同步时保存草稿、失败独立重试和取消排队。
- Rust 脚本流测试在 FETCH 尚未放行时验证 DB try_lock / 工作区读取 / 保存草稿成功；另验证两个真实调度 worker 同时阻塞于脚本 IMAP 流时，共享数据库仍可保存草稿。没有使用真实邮件服务器或真实账号。
- 队列单测覆盖并发上限、去重、FIFO、取消竞争、失败 / 异常隔离、容量、历史有界；事务测试覆盖 UIDVALIDITY、SQL 回滚、部分提交、账号重配 / 重建、游标 CAS。
- TS 覆盖乱序进度、提交去重、轮询恢复 / 重挂载、终态刷新合并及刷新与草稿保存竞争。浏览器多账号测试使用 test-only IPC harness，仅验证 UI；界面截图已目视检查。
- 未新增真实服务器成功同步、原生队列 UI 端到端、安装包或跨平台验收；沿用下方未验证记录。

### 接下来

1. 明确已读 / 星标 / 归档的本地持久化及 IMAP 回写语义。
2. 验证真实测试账号的成功收信与队列行为，补 STARTTLS。
3. 继续 SMTP / outbox；持久任务恢复、运行中取消和自动退避按独立增量设计。

以下为 A4.1 草稿增量及接手时历史记录；若与最新增量冲突，以上述内容和 SYNCHRONIZATION 为准。

## 本轮增量（2026-11-11，A4.1）

- 修复真实账号不能保存草稿：桌面 gateway 不再调用只接受演示账号的 localStorage 服务，新增草稿仓库与三个 IPC，桌面统一保存 SQLite。
- v3 迁移追加 revision、索引和导入收据，保留已有 v2 草稿；旧 WebView 演示草稿事务性一次导入，原 localStorage 保留备份，不覆盖较新的数据库草稿。
- 演示种子初始化不再删除账号行，避免工作区 reload / 启动时级联删除持久草稿；仍按既有行为刷新演示邮件。
- 800ms 防抖自动保存；保存期间允许继续编辑，串行合并新修改；保存失败保留内容和 dirty 状态，不显示假成功。
- 保存核对 revision、previousAccountId 和目标账号；普通草稿可原子切换发件账号；回复草稿保留原始 replyToId 与空主题，不误存列表占位文案。
- 编辑器内关闭确认、浏览器 beforeunload 和 Tauri close-requested 保护已实现；最后已保存版本可从草稿箱重新打开。强制结束进程不保证最后 800ms 未落盘内容恢复。
- 修正 README、架构、接入规范和 UI 中过时的存储 / 连接说明；新增 `docs/DRAFTS.md`。

### 本轮验证

- `npm run check`：lint、schema、类型检查、构建与 16 个领域测试通过。
- `npm run test:ui`：10 个 Chromium 行为测试通过，包括自动保存后 reload、更换发件账号不重复、失败保留输入、回复元数据恢复及原有布局 / 皮肤功能。
- `npm run check:native`：rustfmt、34 个 Rust 测试和 Clippy（-D warnings）通过。新增覆盖数据库重新打开恢复、旧版本与错误账号拒绝覆盖、长度 / 回复账号校验、旧数据一次导入和批次回滚、v2→v3 保留内容，以及演示种子刷新不删除草稿。
- 依赖现有提示：`imap-proto 0.10.2` 有 Rust future-incompatibility 警告；本轮未升级协议栈。
- 隔离桌面启动：使用工作目录 `artifacts/native-draft-smoke`，确认实际创建 SQLite v3 与 revision 列；未读写用户现有数据库或邮箱凭据。目标桌面 UI 捕获未能可靠显示工作区，因此不把原生关闭保护记作端到端通过。测试进程已停止。
- Browser 技能检查：浏览器草稿恢复及保存提示可见，三款皮肤明暗切换；截图在忽略目录 `artifacts/ui-review`。最后确认的编辑器截图为 `compose-linen-dark.png`。

### 下一步入口

1. 同步锁拆分与进程内多账号调度已在 A3.2 完成，见 SYNCHRONIZATION。
2. 明确已读 / 星标 / 归档的本地持久化与 IMAP 回写语义。
3. 实现 SMTP / outbox 状态机后再开放发送；结果不明不自动重发。
4. 用真实测试账号验收成功收信，补 STARTTLS；原生窗口关闭保护需在可可靠捕获目标 UI 的环境补测。

以下 2026-11-10 内容为接手时的历史验证记录。

## 已完成（本轮已验证）

- 邮件工作区：统一收件箱、多账号切换、搜索、标签筛选、阅读、星标、归档、撰写和本机演示草稿。
- 事件视图：演示待办完成状态、跳转关联邮件。
- 皮肤：Paper / Graphite / Linen 内置包；明暗 / 跟随系统；桌面端扫描独立皮肤包、坏包隔离、缺失回退。
- UI：自定义窗口栏、导航栏与邮件列表宽度可调（拖拽 / 方向键 / Home·End / 双击复位）、窄窗口阅读切换、可访问弹窗、空状态、错误边界；主工作区全宽阅读窗格；Auu 字母 A 品牌标与 Orbitron 品牌字。
- 账号向导（阶段 A2）：IMAP / SMTP 真实 TLS 连接验证；账号保存前二次验证；密码只写入 Windows 凭据管理器；两步确认删除。
- **增量同步（阶段 A3.1）**：`mail_sync_account` 从系统凭据库内部取密码（不经 IPC），IMAP LIST 同步文件夹结构（INBOX / Sent / Drafts / Archive / Trash / 其他），INBOX 与 Sent 走 UID 游标增量（`sync_states` 存 `{uidValidity, lastUid}`，首次窗口最近 50 封）；UIDVALIDITY 变更时清理该文件夹旧行防止重复；正文经 mail-parser 解析（RFC 2047 主题解码、text/plain 正文、发件人/收件人），只存纯文本。账号页显示同步按钮、上次同步时间与真实同步结果。
- 桌面交互：应用框架禁止文本选择，分隔条拖拽锁定光标且不产生选区。
- Rust / SQLite：固定数据目录、逐版本迁移与 FTS5 索引、受限皮肤扫描命令。

### 验证记录（2026-11-10，Windows / Node 20.19 / Rust 1.98）

- `npm run check`：lint、皮肤 schema 一致性、类型检查、生产构建、11 个领域测试，全部通过。
- `npm run check:native`：cargo fmt --check、cargo test（27 个 Rust 测试：迁移 / 仓库 / 分页 / 隔离 / FTS 转义 / 凭据真实读写 / IMAP+SMTP 协议会话 / 同步增量与 UIDVALIDITY 清理）、clippy（-D warnings），全部通过。
- `npm run test:ui`：7 个 Playwright 测试全部通过（含侧栏拖拽 / 双击复位 / 刷新持久化）。
- **桌面端到端实测**（`tauri dev` + WebView2 CDP 连接验证）：
  - 工作区：收件箱 6 封邮件经分页 IPC 读自 SQLite；搜索"呼吸感"FTS 命中 1 封；账号切换隔离正确；状态栏显示"桌面工作区"。数据库落盘验证：`%APPDATA%/com.auu.mailclient/mail.db`，`user_version=2`。
  - 账号向导错误路径：对 `127.0.0.1:1`（无服务端口）执行真实"测试连接"，IMAP 与 SMTP 双双返回真实 TCP 拒绝错误（os error 10061）并展示在报告区——错误链路端到端为真，无任何伪造成功。
  - 同步错误路径：插入真实账号行并写入 Windows 凭据后点击"同步"，keyring 读取成功 → TLS 连接真实被拒（os error 10061）→ 账号页展示"同步失败：无法连接到 127.0.0.1:1…"。测试账号与凭据已清理。
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

- **发送队列与真实发信（阶段 A4 后续）**：SMTP 发送、outbox 表状态机、发送结果不明时不重复发送；附件和服务器草稿同步。本地草稿持久化与自动保存已在 A4.1 完成。
- 星标 / 已读 / 归档的跨会话持久化与会话内状态回写 IMAP（当前仅同步时读取标志）。
- 正文按需读取（当前同步时一并拉取全文并仅保存 text/plain）；HTML-only 邮件正文暂为空。
- 进程内多账号队列与独立手动重试已完成；持久任务恢复与自动退避重试仍未实现。
- AI 能力（界面按钮已占位，未调用任何模型）；日历同步。
- UI 侧消息增量加载与虚拟列表（当前桌面 gateway 内部循环取页拼快照，数据量增大前可接受）。

## 未验证

- **真实邮件服务器上的成功同步**：协议状态机、增量与 UIDVALIDITY 逻辑已用脚本化 MockStream 单测覆盖；成功路径（真实 TLS + 登录 + SELECT/FETCH）需要你的真实测试账号，属阶段 A5 验收。
- STARTTLS（143/587）路径：当前仅隐式 TLS（993/465）。
- `npm run desktop:build`（打包与 NSIS 安装包未在本轮运行；桌面 dev 窗口已实测）。
- 皮肤包桌面扫描依赖 Tauri 命令，浏览器预览无法覆盖（单测与 schema 校验已覆盖数据层）。
- macOS / Linux（keyring 与打包按阶段 D 扩展）。

## 已知限制（保存边界）

- 外观、分栏宽度存 localStorage；桌面草稿 / 账号 / 同步邮件存 SQLite；浏览器演示草稿存 localStorage。
- 星标、已读、归档、事件完成状态仅本次会话有效。
- Windows 系统凭据服务已实现并用于 IMAP 账号密码；OAuth 与 AI Key 的业务接入尚未实现。密码不存 SQLite 或 localStorage。

## Git / 发布

- 本轮完成 Git 初始化并推送到 GitHub（远程仓库见 `git remote -v`）。发布打包、CI 发布流水线与云服务仍未配置。
