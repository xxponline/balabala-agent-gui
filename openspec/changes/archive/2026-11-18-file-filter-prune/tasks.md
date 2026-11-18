# Tasks

## 1. 过滤引擎升级(纯求值,行为基线)

- [x] 1.1 在 `src/asset_manager/filter.rs` 定义 `FileFilter` trait
      (`is_ignored(rel_path, is_dir) -> bool`, `Send`),并把现有
      `Ruleset` 升级为 `GitIgnoreStyleFilter`:`Rule` 补
      `must_be_dir`(尾斜杠)/`no_dir`(裸词)flag,求值改为
      `(rel_path, is_dir)` 倒序首命中即定(对齐 dir.c:1428)。
      验证:`cargo test filter` 绿,新单测覆盖:裸词命中任意深度任意段、
      `build/` 只命中目录不命中同名文件、`!` 最后命中胜、注释/空行/
      CRLF/行尾空白/BOM 解析。
- [x] 1.2 修正匹配器末位 `**` 语义:须消费 ≥1 段才命中(对齐 spike:
      `src/**` 不命中 `src` 自身、命中 `src/a.rs`),中段 `**` 吃零段
      不变。验证:单测 `src/**` vs `src`(不中)/ vs `src/a`(中)、
      `a/**/b` vs `a/b`(中),且 1.1 的既有 `**` 用例不回归。
- [x] 1.3 改写 `filter.rs` 既有纯求值测试到新接口(路径+类型入参),
      覆盖 spike 三事实:`*`+`!*.rs` 对嵌套 `.rs` 的判定、
      `build/`+`!build/keep.txt` 的判定、`MUSTBEDIR`/`NODIR` 组合。
      验证:`cargo test filter` 全绿。
- [x] 1.4 实现 `DefaultExcludesFilter`(包 `EXCLUDES` 常量,按 rel
      末段 label 判断,文件/目录同判)。验证:单测——任意深度的
      `node_modules` 目录与文件形态 `.DS_Store` 均命中、非排除项
      不命中,语义与原硬检查一致。

## 2. trait 接线与枚举减支

- [x] 2.1 `AssetEnumeratable::enumerator` 签名加
      `spec: FilterSpec`(可克隆配方,design D2 修订);Project/
      Module 枚举器存储并逐层克隆转发;`FileEnumerator::new(root,
      &spec)` 在构造时 `spec.build()` 构建 filter 列表并存为字段,
      另提供 `with_filters` 直接注入;`AssetEnumerator::enumerate`
      保持原签名。验证:`cargo build` 通过(`.enumerator()` 调用点
      manager.rs 随改),`cargo test` 现有全链路测试(走 manager,
      规则 `*.log`/`*.rs` 行为不变)绿。
- [x] 2.2 `scan_dir` 按 design D7 携带 `rel_prefix`,逐条目对
      `FileEnumerator.filters` 做无序 OR 查询(任一命中即排除、
      命中即停;目录不 `read_dir`),不保留 `EXCLUDES` 特例分支
      (已折叠为 filter)。验证:enumerator 新单测——被排除目录及其
      子树无任何行;spy filter 对被剪枝子树零调用(证明减支, 经
      `with_filters` 注入);注入两个 filter 时任一命中即排除;gitignore
      filter 的 `!node_modules` 救不回 `DefaultExcludesFilter` 的
      排除(OR 无跨 filter 反悔,design D5);`["build/","!build/
      keep.txt"]` 整支缺席(gotcha);`["*","!src/","!src/**"]` 救活
      `src` 子树;`["*.log"]` 下仅含 `.log` 的目录行保留空行(design
      D6);符号链接按文件求值、`build/` 不命中链接(design D3)。
- [x] 2.3 manager bg task:读规则 → `FilterSpec::new(rules)` →
      `project.enumerator(spec.clone()).enumerate(bg)`;过渡期
      `GitIgnoreStyleFilter::compile(&rules)` 仍供后处理 pass 使用
      (3.1 拆)。验证:`cargo test asset_manager` 绿,含
      `filtering_applies_to_every_project_at_land`(规则随视图携带、
      双 project 求值)与 "Dependency directories absent"(默认排除
      经 filter 链生效)。

## 3. 拆除后处理 pass

- [x] 3.1 删除 `filter_assets` 及其全部测试,manager 拆掉 land 前的
      后处理 map(约 15 行)。验证:`cargo test` 全绿;
      `rg "filter_assets" src` 零命中(单一求值权威)。
- [x] 3.2 核对 tree 集成测试(refresh 重读规则、全链路)在新语义下
      的断言:现有规则均为纯文件模式应无变化,若有目录可见性断言按
      spec delta 对齐。验证:`cargo test` 全量绿。

## 4. 文档同步

- [x] 4.1 更新 `Docs/pending-decisions.md` 中 "只作用于文件行 (目录行
      恒可见)" 的记录为新语义(目录行同等求值、被排除目录连子树不进
      视图、gotcha 与救回路径),注明 spec 变更出处。
      验证:文档措辞与 `openspec/changes/file-filter-prune/specs/
      asset-manager/spec.md` 场景逐条一致。

## 5. 集成验证

- [x] 5.1 全量验证:`cargo test` 通过;对照 spec delta 逐场景检查有
      对应测试(8 个改写场景 + 2 个新增场景, 共 10 个),spike 三事实在
      测试中断言。
      验证:测试清单与场景清单一一对应的核对记录。

## 6. Filter 域迁移与供给重划 (bundle)

- [x] 6.1 filter 引擎与 trait 迁至 `src/project_model/filter.rs`
      (更名 `GitIgnoreFilter`),`DefaultExcludesFilter` 重写为
      `DotFilter` (只藏 `.` 开头, 文件/目录/任意深度), 删除
      `EXCLUDES` 常量。验证: 迁移后单测全绿, 含 dot 判定用例
      (任意深度 dot 目录缺席、非点名放行、六项名单项如
      `node_modules` 不再命中)。
- [x] 6.2 供给与注入: `Module` 增 `filters: Vec<Arc<dyn
      FileFilter>>` (trait 界 +`Sync`, 手写 `Debug`, 摘
      `PartialEq/Eq` 并全量验证使用点); Project 增 git 判定
      (`root/.git` 存在性) + 读 `project.root/.gitignore` +
      构建 `[Dot, GitIgnore?]` + 枚举时现建 Module 并注入。
      验证: 单测 — git 项目注入生效 / 非 git 项目不读根下
      `.gitignore` / 缺 `.gitignore` 只挂 Dot / Clone 链 (manager
      spawn 路径) 不回归。
- [x] 6.3 asset_manager 消费化: 删除 `asset_manager/filter.rs` 与
      `FilterSpec`, `AssetEnumeratable::enumerator()` 回无参,
      `FileEnumerator` 走 `module.filters` (`with_filters` 主路径),
      manager 不再读 workspace 规则, `AssetView::Forest.rules` 字段
      删除且 tree 头部摘要位渲染移除 (design D9, dock-layout delta
      已随规划更新), `load/set_file_tree_rules`、app 默认规则自举、
      legacy 规则补拷路径一并清退。验证: `cargo build` + 全链路测试绿。
- [x] 6.4 spec 场景测试改造: `write_rules` → 写项目根
      `.gitignore`; refresh/full-chain/manager 过滤测试改源;
      excludes 断言反转 (`node_modules` 默认在场、dot 条目缺席、
      `!.env` 救不回)。验证: `cargo test` 全绿, 新增供给链场景
      (非 git 不读、git 生效) 有测试对应。
- [x] 6.5 文档同步: `Docs/pending-decisions.md` 三处 — 240 配置界面
      一节整体消解 (配置源退役)、263 排除表描述改 dot-only、310
      "全量递归 + 排除表" 措辞与未来信号消费记录; `file-tree`
      spec Purpose 措辞 (workspace config → 项目供给) 按规则直接改
      主 spec。验证: 文档与更新后 delta 场景逐条一致。
- [x] 6.6 集成验证: 全量 `cargo test`;对照更新后的 delta
      (asset-manager 2 MODIFIED + 1 REMOVED + 3 ADDED,
      file-tree 1 MODIFIED, dock-layout 1 MODIFIED) 逐场景核对测试对应关系。验证: 场景↔测试
      核对记录。

## Workflow follow-up

- 实现完成后走 `/opsx-archive` 归档(届时 delta 合入
  `openspec/specs/asset-manager/spec.md` 主 spec)。
