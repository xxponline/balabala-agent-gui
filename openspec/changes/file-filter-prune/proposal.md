# Proposal

## Why

过滤目前发生在整树扫完之后的 `filter_assets` 后处理 pass 里:目录行
恒不参与求值、永不剪枝,大目录(用户规则里的 `vendor/`、`dist/` 等)
照读不误,且对 gitignore 的目录语义(尾斜杠 = 只匹配目录、被忽略目录
内的 `!` 救不回)只是近似复刻。用户要的是一份**完全复刻 gitignore
格式的忽略数据**,在**枚举时就减支**——被忽略的目录根本不进扫描,
与 git 的 `treat_path` 行为一致(dir.c:2496)。

## What Changes

- 新增 `FileFilter` trait:枚举期间对每个条目按 `(相对路径, 是否目录)`
  求值,目录命中即剪枝(不 `read_dir`,子树不存在)。
- 新增首个实现 `GitIgnoreStyleFilter`:compile 单一忽略文本(gitignore
  格式行),按 git `last_matching_pattern_from_list`(dir.c:1428)倒序
  求值——最后命中者胜、`!` 反悔、`MUSTBEDIR`(尾斜杠)/`NODIR`(裸词
  basename)flag 按 `parse_path_pattern`(dir.c:702)补齐。
- `FileEnumerator` 在构造时从 `FilterSpec`(可克隆配方,携带
  workspace 忽略行)**构建并保存 filter 列表**(`Vec<Box<dyn
  FileFilter>>` 字段),`scan_dir` 查表减支;注入点:
  `enumerator(&self, spec: FilterSpec)` 随链转发(`enumerate()` 签名
  不变),测试/自定义可 `with_filters` 直接注入成品 filter。
  **任一命中即排除**(OR、早停,无跨 filter 反悔);每次运行由
  manager 重建 spec,随刷新重读。
- **BREAKING**(spec 行为变更):规则从"只作用于文件行、目录行恒显"
  改为**文件行与目录行同等求值,被忽略目录连同子树不进视图**;
  被忽略目录内部的 `!` 无法救回(git gotcha),`!目录自身` 可救回整个
  子树。`["*", "!*.rs"]` 的可见性结果随之改变。
- 删除后处理 pass `filter_assets` 及其测试(单一求值权威:扫描时一次
  决定)。`EXCLUDES` 默认排除表折叠为 `DefaultExcludesFilter` 作为
  注入的第一个 filter:OR 组合下用户规则的 `!node_modules` 照样救不回
  (无跨 filter 反悔),行为与原硬检查逐条目等价。
- 零新增 crate:手写匹配器沿用现状,不引 globset/regex。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `asset-manager`:Requirement "File-row filtering relative to project
  roots" 的求值范围从文件行扩到目录行,新增减支与 `!` 边界语义;
  相关场景(Deny-all with re-include 等)改写并新增 gotcha 场景。

## Impact

- 代码:`src/asset_manager/filter.rs`(FileFilter trait +
  GitIgnoreStyleFilter + DefaultExcludesFilter, 删 `filter_assets`)、
  `src/asset_manager/enumerator.rs`(trait 签名、
  scan_dir 减支)、`src/asset_manager/manager.rs`(compile + 组装
  filters + 传参,拆后处理 map)。
- 测试:filter.rs 纯求值测试保留改写;`filter_assets_*` 测试删除,
  由 enumerator 减支测试接替;tree/manager 集成测试用的 `*.log`、
  `*.rs` 纯文件模式行为不变。
- 配置格式不变:规则仍来自 `.balabala.workspace.json` 的 `file_tree`
  区段(行数组即 gitignore 格式),规则源条款不动。
- 文档:`Docs/pending-decisions.md` 中 "只作用于文件行" 的记录需同步。
- `file-tree` spec 的 Purpose 措辞 "filter file rows" → "filter file and
  directory rows" 属 Purpose 变更,按 OpenSpec 规则直接改主 spec
  (非 requirement 变更,不产生 delta)。
- 匹配语言保持 `* ? **` 子集(既有 spec 子集);gitignore 的**格式层**
  特性(注释/空行/CRLF/行尾空白/锚定/尾斜杠/裸词/`!`/最后命中胜)
  完整实现。字符类 `[]` 与 `\` 转义维持现状——有意不实现(design
  Non-Goals)。
