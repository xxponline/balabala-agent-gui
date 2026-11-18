# Design

## Context

首版(枚举期过滤 + 减支)已落地:过滤在扫描时一次决定、后处理 pass
已删、gitignore 语义经 ref/git 三处 spike 验证。本 design 覆盖固化后的
完整形态:**供给与归属重划**(bundle:dot-only 默认、workspace 源退役、
Project 级 `.gitignore` 供给、Filter 域迁出 asset_manager)。

约束:零新增 crate;枚举跑在 background executor 上(`FileFilter:
Send + Sync`,`Arc` 跨线程);测试全走 manager 全链路。

git 语义依据:`ref/git/` 的 `dir.c`(`parse_path_pattern` dir.c:702、
`last_matching_pattern_from_list` dir.c:1428、`treat_path` dir.c:2496)
与 `../gitignore-解析与使用逻辑.md`。**三处行为已用真 git spike 验证**:

```
pattern src/**      -> src 本身 NOT ignored, src/a.rs ignored
                       (末位 ** 须消费 ≥1 段, 已修)
pattern * + !*.rs   -> src ignored, src/a.rs ignored (嵌套 .rs 救不回)
pattern build/ + !build/keep.txt -> keep.txt ignored (被忽略目录内 ! 无效)
```

## Goals / Non-Goals

**Goals:**

- 过滤在枚举时一次决定:目录命中即减支(不 `read_dir`,子树不产生)。
- 单一求值权威:扫描期一次决定,无后处理。
- 默认行为 = 只藏 `.` 开头;其余一切(node_modules 等)默认可见,
  想藏就写 `.gitignore`。
- 规则源单一收敛:Project 一级判定 git、读自身根 `.gitignore`,
  注入它创建的每个 Module;workspace 不参与过滤事务。
- Filter 归属下沉:`project_model` 拥有类型、引擎、构建与规则源;
  asset_manager 只消费 `module.filters` 并在扫描时执行求值。
- gitignore **格式层**完整复刻;匹配语言维持 `* ? **` 子集,零依赖。

**Non-Goals:**

- 逐目录 `.gitignore` 发现与惰性加载(`exclude_stack`/per-dir base/
  EXC 三组优先级);子目录 Module 自身的 `.gitignore` 不读——配置点
  唯一在 Project 根。
- `.git/info/exclude`、全局 gitignore(`core.excludesFile`)不读。
- 字符类 `[]` 与 `\` 转义。
- 过滤的配置界面或任何 workspace 侧自定义入口(已随配置源退役作废)。
- filter 组合的优先级链/跨 filter 反悔(现为无序 OR;将来再议)。

## Decisions

**D1 过滤进扫描,后处理 pass 下场(单权威)**
`scan_dir` 对每个条目依次查 `filters`(OR、任一命中即 `continue`),
目录不递归。`filter_assets` 及其测试已删除。

**D2 归属与供给:Filter 域在 project_model,枚举触发 Module 创建
(bundle 核心)**
- 类型与引擎迁至 `src/project_model/filter.rs`
  (`FileFilter` trait、glob 引擎、`DotFilter`、`GitIgnoreFilter`);
  `src/asset_manager/filter.rs` 与 `FilterSpec` 删除,
  `AssetEnumeratable::enumerator()` 恢复无参——链上不再携带任何东西,
  所有输入从 `root` 自身推导。
- `Module` 增字段 `filters: Vec<Arc<dyn FileFilter>>`(注入点)。
  机械账:`Arc` 保住 `Project: Clone`(manager 把 projects 克隆进 bg
  spawn,是承重墙)、trait 界涨 `Send + Sync`;`Debug` 手写(只打
  `name/root`);`PartialEq/Eq` 疑似零使用直接摘,实现时全量验证,
  若有人用则手写比 `name+root`。
- **枚举触发创建**:Project 提供"现建带 filter 的 Module"入口,
  ProjectEnumerator 每次运行调用——"创建时读"字面成立,IO 在 bg task
  内,⟳ 天然重读 `.gitignore`(spec refresh 条款存活)。
- Manager 只消费:拿到 Module → `FileEnumerator::with_filters(root,
  module.filters.clone())`(Arc 克隆即引用计数);扫描执行求值是
  "吃"的唯一方式,留在 asset_manager。
- 备选(被否):filter 做成 `from_root` 时的永久字段(刷新带陈旧值);
  资产侧继续组装(违背"Module 给什么 Manager 吃什么")。

**D3 trait 形状**
```rust
pub trait FileFilter: Send + Sync {
    /// rel_path: '/' 分隔、相对枚举根; is_dir 来自 lstat
    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool;
}
```
减支 = `is_dir && is_ignored(...)`。`is_dir` 是 `MUSTBEDIR` 的载体;
符号链接按 lstat = 文件,目录专属规则不命中。

**D4 `GitIgnoreFilter` = glob 引擎(原 GitIgnoreStyleFilter 迁址更名)**
`Rule` 带 `must_be_dir`/`no_dir` flag(对齐 `parse_path_pattern`);
`matches(rel, is_dir)` 倒序首命中即定(对齐 dir.c:1428);编译一次、
逐条目复用。**匹配器修正**(spike):末位 `**` 须消费 ≥1 段,中段
吃零段不变。行解析:空行/行首 `#`/BOM/`\r`/行尾空白。

**D5 默认排除 = dot-only 硬 filter(`DotFilter`)**
判定:rel 末段 label 以 `.` 开头 → 忽略(文件/目录同判、任意深度)。
六项名单(`EXCLUDES` 常量)整体撤销——`node_modules/target/build/dist`
默认可见,归 `.gitignore` 管。独立 filter、OR 组合、**无跨层反悔**:
用户 `.gitignore` 的 `!.env` 救不回 dot 排除(spec 场景钉住)。
备选"`.*` 注入规则链可被 `!` 救回"被否:要的是硬默认,无逃生门。

**D6 空目录行策略:保留显示**
目录自身未被排除 → 枚举行 → 子树被过滤光也显示空行(spec 已钉)。

**D7 相对路径的构建**
`scan_dir` 逐层携带 `rel_prefix`,条目 rel = prefix + label。

**D8 规则源 = Project 一级 git 供给**
Project 判定 `root/.git` 存在(存在即算,含 worktree/submodule 的
`.git` 文件形态;不 shell out);是 → 读 `project.root/.gitignore`
(逐行即规则,相对 project 根——与既有 anchor 一致)→ 构建
`[DotFilter, GitIgnoreFilter?]` 注入每个 Module;否 → 只有
`DotFilter`,**根下若躺着 `.gitignore` 也不读**(配置点唯一,spec
场景钉住)。子目录 Module 继承同一套(v1 单 root module 时两者重合,
多模块时代由 D2 的注入路径天然成立)。

**D9 workspace 配置源退役**
`load_file_tree_rules`/`set_file_tree_rules`、marker `file_tree` 字段
的读写、app 侧默认规则自举全部删除;`AssetView::Forest` 的 `rules`
字段失去来源 → 删除,缓存失效输入收敛为"新枚举落地/刷新"(.gitignore
改动经 ⟳ 触发新枚举即失效,无独立监听——与既有 refresh 机制一致)。
marker 中残留的 `file_tree` 字段被 serde 忽略,无数据迁移。
**头部摘要位一并撤除**(用户裁定, 否决"保留字段供摘要"方案):
`tree/mod.rs` 的 `rules_summary` 渲染删除,`dock-layout` 的 header
chrome 条款以 delta 收窄为标题 + ⟳ + 关闭 — 过滤的可见去处是
`.gitignore` 文件本身。

## Risks / Trade-offs

- [deny-all 类规则在新语义下可能"清空"视图(`["*","!*.rs"]` 只剩根层
  .rs)] → git 同款,spec 场景已钉 gotcha。
- [六项撤销后,无 `.gitignore` 的大仓 node_modules/target 全量进树]
 → 这是"全部撤掉"的直接推论,spec 用显式场景钉住;背景是
  pending-decisions 里"用户要求浏览被排除目录"的既知信号。
- [手写匹配器边界错漏] → spike 三事实 + 逐 flag 单测;零依赖。
- [`PartialEq/Eq` 摘除可能碰到未知使用点] → 实现时全量验证;真有人
  用就手写比 `name+root`。
- [filter 迁移的大范围文件移动] → 引擎测试随文件平移,行为不变的
  迁移与行为改造分组落地(见 tasks 6.x),每组测试先行。
- [非 git 项目里躺着 `.gitignore` 却不生效,可能被当 bug] → 这是
  D8 的既定语义,spec 场景明文钉住,文档同步说明。

## Migration Plan

分两步落地(同 change 内两组任务,各自测试基线绿):
① **迁移与重写**(行为重排):filter 域搬家 + DotFilter 语义 +
Project 供给 + Module 字段 + 消费化改造;
② **场景对齐**(spec 测试):供给链、excludes 反转、refresh 改源。
回滚 = revert 对应提交。无数据迁移(marker 字段残留被忽略)。

## Open Questions

无。挂起项均已定案:git 判定范围 = Project 根(D8);子目录 Module
自身 `.gitignore` 不读(D8/Non-Goals);`AssetView.rules` 删除(D9);
`PartialEq/Eq` 摘除(实现时验证,见 Risks)。
