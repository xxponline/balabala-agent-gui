# Proposal

## Why

资产视图代码散在两个语义错位的模块里：`src/tree/`（1546 行单文件，名字叫 tree，
实际是资产面板）与 `src/asset_manager/`（服务层 + DTO）。原设计意图是"filter
只是某些 project/module 的内部功能，资产侧只见 `EnumeratorOwner`/`Enumerator`
两个 trait"，但现状边界画在 `enumerator.rs` 文件中间：`FileFilter` 的持有与
逐条目求值在资产侧、`Module.filters` 是被资产侧 clone 的 pub 字段、trait 定义
也在资产侧。本次把两者合并为一个包并把枚举边界收回 trait 上，一次还清命名债
与边界债。

## What Changes

- 包归并：`src/tree/` + `src/asset_manager/` → `src/assets/`
  （`state`/`traits`/`manager`/`panel` + 测试外提两个 `#[cfg(test)]` 模块）。
- 改名：面板 `TreePanel` → `AssetView`；DTO `AssetView` → `AssetState`；
  `AssetManager::view()` → `state()`；实体字段 `tree` → `asset_view`。
- 枚举实现归位 `project_model::enumerate.rs`（Project/Module/FileEnumerator
  与对应 impl），filter 域保持在 `project_model` 内部。
- **BREAKING**（crate 内 API，无外部使用者）：枚举边界收紧到 Level 2——
  trait 改名 `AssetEnumeratable`/`AssetEnumerator` → `EnumeratorOwner`/
  `Enumerator` 并补身份访问器 `root()`；`AssetManager::register_project(Project)`
  → `register(owner)`，manager 持 `Box<dyn EnumeratorOwner>`，不再可见
  `Project` 类型；`Module.filters` 降为私有（注入收敛到 project 构建期）。
- 面板构造注入 `Entity<AssetManager>`：拉视图/代数不再经 app 兄弟跳；
  `WeakEntity<TerminalApp>` 收敛为两项职责——右 dock 可见性查询与打开
  分发（`open_object` 属 app 域）。
- 行为全部冻结：规格零改动（`.openspec.yaml` 声明 `skip_specs: true`）。

## Capabilities

### New Capabilities

（无——纯重构，不引入新能力。）

### Modified Capabilities

（无——`asset-manager`（10 需求）与 `file-tree`（7 需求）等全部规格行为不变；
对应场景逐条原绿是本 change 的验收铁律。）

## Impact

- 代码：新增 `src/assets/`（state.rs / traits.rs / manager.rs / panel.rs /
  panel_tests.rs / manager_tests.rs）与 `src/project_model/enumerate.rs`；
  删除 `src/tree/`、`src/asset_manager/`；消费方 `src/app.rs`（字段、
  notify_panels、注册点、测试）、`src/shell/mod.rs`（use/构造/ShellParts）、
  `src/main.rs`（mod 声明）、`src/property_detail_manager/`（3 处 import）。
- API：manager 注册面 trait 化（见 What Changes 的 BREAKING 项）；
  crate 内波及封闭。
- 测试：约 1500 行测试随迁（tree/mod.rs 内约 1000 + manager.rs 内约 550），
  全程只改 use 路径不改断言；基线 **268 passed / 0 failed / 0 warnings**
  数字不变为组 1 验收。
- 风险：改动面广但行为冻结；`Box<dyn EnumeratorOwner>` 的 Send 边界与
  refresh 时 enumerator 的构造时机需在 design 定案（owner 不入 spawn）。
