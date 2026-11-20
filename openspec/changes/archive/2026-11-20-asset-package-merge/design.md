# Design

## Context

现状两模块并立：`src/asset_manager/`（AssetManager 实体 + DTO + 枚举实现，
~1490 行）与 `src/tree/`（TreePanel，1546 行单文件含 ~1000 行测试）。拓扑上
双实体各自持 `WeakEntity<TerminalApp>` 回指：面板经 `app.asset_manager` 兄弟跳
拉视图/代数，manager 落地后经 `app.notify_panels` 枢纽（`cx.defer` 防重入）扇
出通知，树面板走特殊入口 `on_data_changed`（可见性闸门 → 代数比对）。枚举边界
的泄漏点见 proposal.md — Why：`FileFilter` 持有/求值、`Module.filters` pub
字段、trait 定义均在资产侧。约束：行为全部冻结；基线 268 passed / 0 warnings；
bin crate 无外部使用者；`notify_panels` 是 sidebar/terminal/tree 三家共用的全
局家具，不在本次触碰。

## Goals / Non-Goals

**Goals:**

- 单一包 `src/assets/`，命名统一（面板 `AssetView`、DTO `AssetState`）。
- 枚举边界收到 trait 上（Level 2）：manager 只见 `Box<dyn EnumeratorOwner>`，
  `Project` 类型与 filter 词汇全部退出资产侧。
- 枚举实现与 filter 归位 project 域（`project_model::enumerate.rs` + 既有
  `filter.rs`），依赖方向单向：`project_model → assets`（trait + DTO）。
- 面板构造注入 manager，砍掉经 app 的兄弟跳（可见性查询除外）。

**Non-Goals:**

- 合实体（panel 内嵌 manager 状态）——破坏 manager 无 Window 可测性。
- `notify_panels` 改订阅制——全局机制，单独立项。
- filter 语义修正：`rel` 以 module.root 为基准而 `.gitignore` 在 project root
  （今日靠 v1"Project 根 == 根 Module 根"等价兜底，规格标题写的是 relative to
  project roots）——已知注脚，本次冻结不动。
- capability 改名（`file-tree` / `asset-manager` 规格名与内容零改动）。
- crate 化 / 宿主接口抽象；`property_detail_manager` 归并。

## Decisions

**D1 包布局与依赖方向。** `src/assets/`：`state.rs`（DTO 族）、`traits.rs`
（两个 trait）、`manager.rs`、`panel.rs`、`panel_tests.rs`、`manager_tests.rs`；
`project_model/enumerate.rs` 收枚举实现。trait 与 DTO 同家住 assets（impl 的
返回类型 `AssetNode` 与契约同源才自洽），impl 块写在 project 域（持有 filter
的一侧）。Alternative：trait 放 project_model——被拒，那样"资产契约"反向定义
在实现域，Level 2 的语义倒置。

**D2 trait 契约与 spawn 边界。**

```rust
pub trait EnumeratorOwner {
    /// 身份记账（现状"身份 = project root"）与 reveal/路径语义的依据。
    fn root(&self) -> &Path;
    fn enumerator(&self) -> Box<dyn Enumerator>;
}
pub trait Enumerator: Send {
    fn enumerate(
        self: Box<Self>,
        bg: BackgroundExecutor,
    ) -> BoxFuture<'static, Result<AssetNode, String>>;
}
```

关键：**owner 不入 spawn**——refresh 在主线程逐 owner 调 `enumerator()`（同步
构造），把 `Box<dyn Enumerator>`（Send）丢后台。这避开 `Box<dyn>` 无 Clone 的
问题。Alternative：trait 加 `clone_owner()`——被拒（无谓克隆面；今日代码把
`Project` 克隆进 task 只是实现习惯，不是语义需要）。

**D3 注册面。** `AssetManager::register(owner: Box<dyn EnumeratorOwner + Send>)`
（及批量形式），记账语义冻结：保 solution 序、身份 = `owner.root()`、重复
root 的处理与今日 `register_project` 逐字一致。`open_object_for(item_id)` 从
绝对路径 id 构造 `OpenObject`，不触碰 owner 内部 ✓。`ws_dir` 来源不变。

**D4 filter 内化。** `Module.filters` 降私有，注入收敛到 `Project` 构建/
`provision_filters` 内部（刷新重跑即重读的既有语义由 `ProjectEnumerator` 枚举
期重执行承载，不变）。`filter.rs` 留在 `project_model`，可见性顺手收紧到
`pub(crate)`。求值权威仍在扫描期（design D1 of asset-manager-right-dock），
逐条目无序 OR / 目录命中减支 / 倒序首命中语义逐字不动。

**D5 改名映射（全量）。**

| 旧 | 新 |
|---|---|
| `crate::asset_manager` / `crate::tree` | `crate::assets` |
| 面板 `TreePanel` | `AssetView` |
| DTO `AssetView` | `AssetState` |
| `AssetManager::view()` | `state()` |
| `AssetEnumeratable` / `AssetEnumerator` | `EnumeratorOwner` / `Enumerator` |
| 实体字段 `ShellParts.tree` / `app.tree` | `asset_view` |
| `find_kind(view:…)` 形参 | `state:…` |

不留兼容 re-export（bin crate 一次改净）。

**D6 分组与验收。** 组1 归位+改名（纯机械，验收 = 268/0 数字原样）；组2 边界
收紧 Level 2（验收 = 规格场景原绿 + `rg FileFilter src/assets/` 归零）；组3
构造注入（验收 = panel 内 `\.asset_manager` 归零 + 基线对账）。每组独立 commit、
独立可 revert；测试只改 use 路径不改断言。

**D7 面板注入与可见性闸门。** `AssetView::new(manager: Entity<AssetManager>,
app: WeakEntity<TerminalApp>, cx)`；`sync_if_generation_changed` 改直读
`self.manager`；`right_dock_open` 保留经 app 查 `dock_area`（dock 属 shell 域，
这条边合理）；`on_data_changed` 签名不动（`notify_panels` 零改）。Alternative：
注入可见性查询闭包——过度设计，拒绝。

## Risks / Trade-offs

- [测试搬迁面广（~1500 行）] → 只改 use/路径，断言零改写；每组收尾
  `cargo test --workspace` + `cargo clippy --all-targets` 268/0 对账。
- [`Box<dyn EnumeratorOwner>` 的 Send 边界与 future 'static 性] → D2 的
  "owner 不入 spawn、enumerator 提前构造"消解；若遇借用问题在组2 内解决，
  不外溢。
- [impl 块搬家"看似行为变"，diff 噪音大] → 组1 只做移动，`git log --follow`
  可复核；对应规格场景测试（asset-manager 10 + file-tree 7 映射用例）原绿。
- [`Module.filters` 私有化触及 project_model 内测试] → 改走 `provision_filters`
  行为断言，不摸字段。
- [改名遗漏（二义期混用）] → `rg` 守卫收尾：`TreePanel|asset_manager::|crate::tree`
  归零；DTO 旧义 `AssetView::` 全库归零。
- [组1 内 trait 暂用旧名（定义先搬家）] → 仅跨一个 commit 的短暂不一致，
  组2 收口，不单独成灾。

## Migration Plan

三组三 commit 依序落地，任意一组可独立 revert 回上一组状态；无数据迁移、无
部署面（bin crate）。

## Open Questions

无——注册面重复 root 语义、字段名等已在 D3/D5 定案，实现按"语义跟现状逐字
一致"执行。
