# Proposal

## Why

中槽文件打开目前只认领 `.rs`——`.md` 文件（README、文档、笔记）落在 "unclaimed opens nothing" 的空白上。文档查看是个人工作台的高频需求，而用户明确不需要 WYSIWYG 编辑，需要的是**渲染视图与原文之间一键切换**的只读查看器。

## What Changes

- 新增 `MdPanel`：md 文件专用查看面板，双模式 `Rendered | Source`，**默认渲染视图**
- 渲染模式：`TextView::markdown` 纯静态渲染（已源码验证：全量替换路径立即显示、Motion 默认全零，无淡入动画；保险行 `.stream_fade(false)` 兜底 theme 时序）
- 源码模式：复用现有 Editor 只读管线（syntect `markdown` 语法），与 `.rs` 查看器同栈
- 切换 UI：**tab 条右侧一个按钮**（单按钮 toggle）+ 对应 toggle action
- 模式**不记忆**：每次打开 md 均回到渲染视图（无持久化、无 per-file 状态）
- default-open 认领表扩展：`md` 扩展名 → `MdPanel`（"unclaimed opens nothing" 语义不变，md 从此有主）
- 焦点路径：source 模式下 Editor 可聚焦，tab 切换的焦点跟随行为与既有面板一致

## Capabilities

### New Capabilities

（无——复用既有 `center-tabs` 能力域，不引入新 capability）

### Modified Capabilities

- `center-tabs`：default-open 认领表扩展（`md` 认领）、中槽面板双模式与切换行为、tab 条切换按钮的可见性与行为
