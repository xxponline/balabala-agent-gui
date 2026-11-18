# Proposal

## Why

过滤目前发生在整树扫完之后的 `filter_assets` 后处理 pass 里:目录行
恒不参与求值、永不剪枝,大目录照读不误,gitignore 目录语义只是近似
复刻。需要在**枚举时就减支**,并完全复刻 gitignore 格式与求值语义
(ref/git 对照,spike 验证)。

首版(枚举期过滤)落地后,供给与归属两个问题随之而来:默认排除的
六项名单既武断又漏(`node_modules` 类需求本就是一条 gitignore 规则),
workspace 配置源与 `.gitignore` 双源并存无人维护;filter 的类型、组装
与规则源全挤在 asset_manager 里,而它本应只消费。本 change 一并重划:
**默认排除只藏 `.` 开头,规则源收敛到 Project 一级的 `.gitignore`,
Filter 域下沉到 Module 一侧,Manager 只吃供给**。

## What Changes

- 新增 `FileFilter` trait 与 `GitIgnoreStyleFilter`:gitignore 格式层
  完整复刻(注释/空行/CRLF/行尾空白/BOM/`!`/尾斜杠 MUSTBEDIR/裸词
  NODIR/锚定/后写覆盖先写),倒序首命中即定(对齐 dir.c:1428)。
- **默认排除改为只藏 `.` 开头**(BREAKING):六项名单
  (`.git node_modules target build dist .DS_Store`)整体撤销,
  `node_modules/target/build/dist` 默认出现在视图中;点开头条目
  (文件与目录、任意深度)一律排除。
- **硬排除、无逃生门**:默认 dot 排除是独立 filter,OR 组合、不可被
  任何 filter 的 `!` 跨层反悔。
- **workspace 退出过滤事务**(BREAKING):`file_tree` 配置源退役——
  `load/set_file_tree_rules`、marker 字段的使用、app 侧默认规则自举
  全部删除;spec 中"规则来自 workspace 配置"的条款重写。
- **新规则源在 Project 一级**:Project 判定自身根是否 git 管理
  (`root/.git` 存在),是则读 `project.root/.gitignore` 构建
  GitIgnoreFilter,连同默认 DotFilter 一起注入它创建的每个 Module;
  子目录 Module 继承同一套配置,非 git 项目不读任何 ignore 文件。
- **枚举触发 Module 创建**:Project 在被枚举需要时现建 Module 并注入
  filter(IO 在 bg task 内),刷新天然重读 `.gitignore`。
- **Filter 域迁出 asset_manager**(架构重排):类型与引擎移至
  `src/project_model/filter.rs`,`asset_manager/filter.rs` 删除,
  `FilterSpec` 与 `enumerator(spec)` 参数退役(`enumerator()` 恢复无参);
  `Module` 携带 `filters: Vec<Arc<dyn FileFilter>>`(trait 涨
  `Send + Sync`,手写 `Debug`,摘 `PartialEq/Eq`),Manager 只消费,
  扫描期执行求值。
- **BREAKING**(spec 行为变更):规则对文件行与目录行同等求值,被
  排除目录连同子树不进视图;被排除目录内部的 `!` 无法救回(gotcha),
  `!目录自身` 可救回子树。后处理 pass `filter_assets` 删除,单一求值
  权威在扫描期。
- 零新增 crate;匹配语言维持 `* ? **` 子集,字符类 `[]` 与 `\` 转义
  有意不实现。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `asset-manager`:规则源从 workspace 配置改为 Project 供给的
  filter 集(`File-row filtering relative to project roots`、
  `Refresh re-enumerates in the background` 重写);默认排除从六项
  名单改为 dot-only——原 requirement 与其 "Dotfiles visible" 场景
  语义反转无法原块修改,走 REMOVED + ADDED "default dot exclusions";
  新增减支与 Project 级过滤供给两条 requirement。
- `file-tree`:Refresh 对过滤规则的重读来源改为项目根 `.gitignore`
  (`Refresh triggers and behavior` 重写);Purpose 措辞按 OpenSpec
  规则直接改主 spec。
- `dock-layout`:右 dock header 的只读过滤规则摘要位撤除
  (`Right dock renders by default` 重写 — 规则源退役后头部无数据源,
  过滤的可见去处是 `.gitignore` 文件本身)。

## Impact

- 代码:`src/project_model/filter.rs`(新家:trait + 引擎 + Dot +
  GitIgnore)、`src/project_model/project.rs`(判 git、读 `.gitignore`、
  构建并注入 filter、枚举时现建 Module)、`src/asset_manager/filter.rs`
  (删除)、`src/asset_manager/enumerator.rs`(无参化、with_filters
  消费、EXCLUDES 删除)、`src/asset_manager/manager.rs`(不再读规则、
  `AssetView.rules` 字段处置)、`src/tree/mod.rs`(头部摘要位渲染移除)、
  `src/project_model/files.rs`
  (load/set_file_tree_rules 删除)、`src/app.rs:557`(默认规则自举删除)。
- 测试:约十处反转/迁移(excludes 反转、`write_rules` → 写
  `.gitignore`、refresh/chain/manager 测试改源、供给链新场景)。
- 配置:marker 的 `file_tree` 字段成为无用字段(读写逻辑移除即可,
  无数据迁移;旧文件中该字段自然忽略)。
- 文档:`Docs/pending-decisions.md` 三处(240 配置界面一节整体消解、
  263/310 排除表记录);`file-tree` spec Purpose 措辞直接改主 spec。
- 用户可见:自定义过滤的唯一入口从此是项目根 `.gitignore`;新增过滤
  体验(即输即筛等)的既有立项方向作废。
