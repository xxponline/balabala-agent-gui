# Design

## Context

现状(见 proposal.md - Why):过滤是扫完之后的 `filter_assets` 整树
后处理 pass(`src/asset_manager/filter.rs`),目录行恒不参与求值、永不
剪枝;`EXCLUDES` 硬编码排除表在 `scan_dir` 里按 label 无条件砍。规则
来自 workspace 配置的 `file_tree` 行数组,manager 在 bg task 里读取。

约束:零新增 crate(proposal 既定);枚举跑在 background executor 上
(`BoxFuture: Send`);`AssetEnumeratable`/`AssetEnumerator` 对象安全
契约是既有 design D2/D2 架构,测试全走 manager 全链路。

git 语义依据:`ref/git/` 的 `dir.c`(`parse_path_pattern` dir.c:702、
`last_matching_pattern_from_list` dir.c:1428、`treat_path` dir.c:2496)
与 `../gitignore-解析与使用逻辑.md`。**三处行为已用真 git spike 验证**
(/tmp 临时仓库,`git check-ignore`):

```
pattern src/**      -> src 本身 NOT ignored, src/a.rs ignored
                       (现匹配器尾部 ** 吃零段, 会误判 src -> 必须修)
pattern * + !*.rs   -> src ignored, src/a.rs ignored (嵌套 .rs 救不回)
pattern build/ + !build/keep.txt -> keep.txt ignored (被忽略目录内 ! 无效)
```

## Goals / Non-Goals

**Goals:**

- 过滤在枚举时一次决定:目录命中即减支(不 `read_dir`,子树不产生)。
- 单一求值权威:删除后处理 pass,不存在两处求值。
- gitignore **格式层**完整复刻:注释(行首 `#`)、空行、CRLF、
  行尾空白裁剪、BOM、`!` 否定、尾斜杠 `MUSTBEDIR`、裸词 `NODIR`
  (任意深度任意段)、路径锚定、后写覆盖先写。
- 匹配语言维持 `* ? **` 子集(既有 spec 承诺),零新依赖。

**Non-Goals:**

- 逐目录 `.gitignore` 发现与惰性加载(`exclude_stack`/`base` 锚定/
  EXC 三组优先级)——明确选了"只吃单一忽略数据"(探索分叉 1 = A)。
- 字符类 `[]` 与 `\` 转义:维持 `filter.rs` 现状的有意不实现。
- 规则配置界面(pending-decisions 已立项另议)。
- `EXCLUDES` 默认排除表的用户可配置化。

## Decisions

**D1 过滤进扫描,后处理 pass 下场(单权威)**
`scan_dir` 对每个条目依次查 `filters`(OR、任一命中即 `continue`),
目录不递归。备选"扫描减支 + 后处理保目录行"的混合被否:两处求值
必然漂移,且 spec 已改为目录行同等求值,后处理没有独立存在理由。
`filter_assets` 及其测试整体删除。

**D2 管子:`FilterSpec` 挂 `enumerator()`,filter 构建并保存在
`FileEnumerator`(修订 — 用户裁定)**
`AssetEnumeratable::enumerator(&self, spec: FilterSpec)`;
`AssetEnumerator::enumerate(self, bg)` **保持原签名**(过滤不是运行时
参数)。`FilterSpec { rules: Vec<String> }` 是可克隆的**配方**:
Project/Module 枚举器存储并逐层 `clone()`(便宜),`FileEnumerator` 在
构造时 `spec.build()` 构建 filter 列表并存为字段。修订理由:枚举链
Project→Module→File 三层转发,成品 `Vec<Box<dyn FileFilter>>` 不可
克隆,按值只能传到第一层(初版 P1 的死墙);链上改传配方后墙消失,
当初对 P2 的否决理由(“成品 Box 焊进身份”)不再成立 — 刷新重读保留:
每次枚举 manager 重新读规则重建 spec,枚举器本就每次运行新建。
**组合写死在 `FilterSpec::build()`**(默认排除第一员 + 用户规则,
design D5),未来新增 filter 类型在此扩;测试/自定义用
`FileEnumerator::with_filters` 直接注入成品 filter。备选(被否):
按值 + `cloned_box` 样板;`Arc` + `Sync`;`Arc<dyn Fn() -> Vec<Box<..>>`
工厂(组合权在 manager,但当前仅两个 filter,多一层机制无收益)。
全仓 `.enumerator()` 调用点仅 1 处(manager.rs)。

**D3 trait 形状:无状态、对象安全**

```rust
pub trait FileFilter: Send {
    /// rel_path: '/' 分隔、相对枚举根; is_dir 来自 lstat
    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool;
}
```

减支 = `is_dir && is_ignored(...)`。`Send` 满足入 bg spawn;无
`Sync` 需求(单线程扫描内持有)。filter 列表构建并保存在
`FileEnumerator`(见 D2),首批两个实现:`GitIgnoreStyleFilter`、
`DefaultExcludesFilter`;**组合语义 = 无序 OR,任一命中即排除、
命中即停(早退),无跨 filter 反悔**——filter A 排除的条目,
filter B 写 `!` 救不回;gitignore 的 `!` 反悔只在自身 filter 内生效。
`is_dir` 参数是 `MUSTBEDIR` 的载体。
符号链接按 lstat = 文件 → 目录专属规则不命中(与 git `resolve_dtype`
不跟随链接大体一致),测试钉住。

**D4 `GitIgnoreStyleFilter` = 现 `Ruleset` 的语义升级**
`Rule` 补 `must_be_dir`/`no_dir` flag(对齐 `parse_path_pattern`);
`matches(rel_path, is_dir)`:`must_be_dir && !is_dir` → 跳过该规则;
`no_dir`(裸词)→ 只比 basename 段;倒序扫描首命中即定(对齐
`last_matching_pattern_from_list`)。编译一次、逐条目复用
(`view()` 时代的逐次编译随 `filter_assets` 一并消失)。
**匹配器修正**(spike):末位 `**` 须消费 ≥1 段才命中——`src/**`
不命中 `src` 自身,与 git 一致;中段 `**` 吃零段保持不变
(`a/**/b` 命中 `a/b`,git 文档 "zero or more directories")。

**D5 `EXCLUDES` 折叠为 `DefaultExcludesFilter`,`FilterSpec::build()`
组装的第一员**
spec "SHALL exclude … at any depth" 是无条件语气:`!node_modules`
救不回。OR 组合下该性质自动成立——默认排除与用户规则是两个独立
filter,用户 filter 的 `!` 无法跨层反悔,行为与原硬检查逐条目等价
(既有 EXCLUDES 测试守卫)。`scan_dir` 因此完全均匀:一个
`filters.iter().any(...)` 查询点,无特权分支。备选“保留硬检查特例”
被否:两套排除体系并存,后续 filter 作者无法推理优先级;备选“有序
优先级链/跨 filter 反悔”被否:引入排序心智负担,且 spec 无此诉求。

**D6 空目录行策略:保留显示**
目录自身未被排除 → 枚举行 → 子树被过滤光也显示空行
(spec 场景已钉:Unexcluded directory with all children filtered stays
visible)。备选"过滤后隐藏空目录"被否:把"被排除"与"恰好为空"两件
事混同,需要整树回填簿记,还牵动刷新/展开态。

**D7 相对路径的构建**
`scan_dir` 逐层携带 `rel_prefix`,条目 rel = prefix + label,避免每条
目对绝对路径做 `strip_prefix`(后处理时代的 `rel_of` 逻辑随 pass 删除)。

## Risks / Trade-offs

- [deny-all 类规则在新语义下可能"清空"用户视图(`["*","!*.rs"]` 只剩
  根层 .rs)] → 这是 git 同款行为,spec 场景已改写并钉住 gotcha;
  文档 `Docs/pending-decisions.md` 同步说明,免得被当回归。
- [手写匹配器的边界错漏(`**` 段数、`MUSTBEDIR` 与 lstat 的组合)]
  → 三处 spike 事实 + 逐 flag 单测;不引 globset(regex 依赖与零依赖
  约束冲突)。
- [filter 求值在超大目录上逐条目调用,trait 动态分发开销] → 单次
  调用即比较,相对 `read_dir`/`lstat` 可忽略;若 profile 显示热点,
  后续可给 `Ruleset` 加 `nowildcardlen` 式字面前缀快路径(git 同款,
  dir.c:702),不影响本 design 的接口。
- [多 filter 被误用为“有序压制”或期待跨 filter `!` 反悔] → 语义钉死
  为无序 OR(design D2),测试:`!node_modules` 在 gitignore filter
  中救不回 `DefaultExcludesFilter` 的排除。
- [`enumerator()` 签名变更波及对象安全与既有测试] → `.enumerator()`
  调用点仅 1 处(manager),测试走 manager 全链路不直连 trait;
  tree 测试用 `*.log`/`*.rs` 纯文件模式,行为不变。

## Migration Plan

单提交原子切换(枚举与视图同源,不存在新旧两套求值并存的中间态):
先落地 filter trait + 升级匹配器(行为不变,测试基线绿)→ 接管
`scan_dir` → 删 `filter_assets` 与后处理 → 改 spec/文档。回滚 = revert
该提交。无数据迁移(配置格式不变)。

## Open Questions

无。挂起的三个下游决策均已定案:`src/**` 边界由 spike 定为"不命中
自身"(D4);空目录行保留显示(D6);`EXCLUDES` 无条件压制(D5)。
