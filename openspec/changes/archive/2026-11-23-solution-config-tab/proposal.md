# Proposal: solution-config-tab

## Why

Solution 创建目前是「左槽入口 → 系统目录 picker → 立即提交」的黑盒：用户看不到
预检（重名/路径有效性），也没有承载将来配置项的位置。solution-service 落地后
（change `solution-service` 已归档）服务侧 Create 与幂等预检已就绪，缺的是把
提交点升级为中槽内建的 SolutionConfig Tab —— 这是拆分表中 SolutionOverview
重构的 UI 前置。

## What Changes

- 左槽 add-solution 入口的动作由「打开系统目录 picker 并立即提交」改为
  「打开中槽 **SolutionConfig Tab**」（入口位置与零 project 可见性规则不变）。
- Tab 内为可观察的表单流：path 输入 + 预检区（空 / 重名 / 路径合法性——
  存在且为目录）+ 提交钮（全部预检通过才启用）+ 结果处理。
- 提交成功：solutions 文件追加、项目行无重启出现、**Tab 自关**；
  提交失败：错误行内显示、Tab 保留；关闭未提交的 Tab：文件与视图零变更。
- 提交仍走 solution-service 的 `Create` ask（幂等语义与回执处理不变），
  **服务与契约零改动**。
- root 口径（决策 2026-11-23）：solution 的 root 只需是合法路径（存在的
  目录），**没有 workspace 位置约束**；solutions 持久化文件照旧在 workspace 内。
- 旧 `add_solution` 的 rfd 直连提交链退役（Tab 内可保留 browse 按钮，picker
  仅用于填充输入、不再直连提交）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `center-tabs`: ADDED —— SolutionConfig Tab 作为非文件对象的中槽签：入口、
  单实例 dedup（再点入口聚焦既有签）、正常可关/可重开的生命周期。
- `project-model`: RENAMED + MODIFIED —— `Solution creation via path picker`
  → `Solution creation via the SolutionConfig tab`：入口打开 Tab 而非系统
  picker，预检门控提交，提交成功自关、关闭零写入。

## Impact

- **代码**：`src/app.rs`（`AddSolution` 事件路由、`add_solution` rfd 链退役、
  `request_create_solution` 提交缝保留给 Tab 与既有测试）、新 Tab 组件
  （沿 `property_detail_manager` 的 PanelView 模式）、`src/sidebar`（事件
  不变，仅落点变）。
- **不改**：solution-service 契约与服务、入口可见性规则、
  `balabala.solutions.json` 格式、资产注册链（复用树镜像 → handoff）。
- **测试**：既有 `request_create_solution` 直调测试缝保持可用；新增 Tab 渲染/
  预检门控/关签零写入/成功自关场景；遵循确定性调度器纪律（无 rfd、无跨线程 await）。
