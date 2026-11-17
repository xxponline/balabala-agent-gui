# balabala-agent-gui

> 个人 Agent 工作台 — 用 Rust + GPUI 打造的桌面端，把多个 `pi` agent 会话
> 嵌进一个窗口里跑。
>
> **A personal agent workbench** — a Rust + GPUI desktop app that runs
> multiple embedded `pi` agent sessions in a single window.

## 简介 / Overview

这是一个个人 Agent 工作台：单实例一次打开一个工作区（启动恢复最近目录，
否则窗内选择），左槽是 Project/Agent 树与工作区切换入口，中央是网格化
渲染的终端控制台（多页签，console 恒为首签），右侧是可过滤的文件树。
工作区由 `balabala.solutions.json` 配置 Solution → Project/Module 结构，
agent 会话挂在其 Project 下（cwd = project root）。会话通过进程内嵌的
pi-shell-rs 引擎驱动，输出以 vt100 帧解析、逐格绘制到 GPUI 上——键盘、
IME、翻页、滚轮、点击坐标与文本选区都在宿主与会话之间原样转发。

This is a personal agent workbench: one workspace is open at a time (the
latest directory is restored at startup, otherwise an in-window picker
offers one); the left slot is a Project/Agent tree with a workspace switch
entry, a grid-rendered terminal console occupies the center tabs (the
console is always the first, non-closable tab), and a filterable file tree
sits on the right. Solutions in `balabala.solutions.json` expand to a
Project/Module structure, and agent sessions mount under their project with
the project root as cwd. Sessions are powered by an in-process pi-shell-rs
engine; their output is parsed as vt100 frames and painted cell-by-cell onto
GPUI — keyboard, IME, paging, wheel scrolling, click coordinates and text
selection are all forwarded faithfully between host and session.

## 功能 / Features

- **单工作区生命周期 / Single-workspace lifecycle** — 启动恢复最近目录、
  窗内选择/确认创建、切换三支门控（活动会话拦截/仅 exited 确认/直通）/
  restore the latest directory at startup, in-window picker + create
  confirmation, gated switching (active blocked / exited confirmed / direct)
- **Solution → Project 模型 / Solution-driven projects** —
  `balabala.solutions.json` 展开 Project/Module 树，agent 挂 Project 下
  (cwd = project root) / solutions expand to projects; agents mount per
  project with the project root as cwd
- **三槽停靠布局 / Three-pane dock layout** — 切换/项目侧栏 · 中央终端 · 文件树 /
  sidebar with project tree · center terminal · file tree
- **多页签中槽 / Center tabs** — console 恒为首签不可关，文件按需开签去重 /
  console pinned as the first non-closable tab; files open into deduplicated tabs
- **只读代码查看器 / Read-only code viewer** — `.rs` 文件内建 Editor 打开，
  syntect 语法高亮 / open `.rs` files in the built-in editor with syntect highlighting
- **终端输入全链路 / Full input forwarding** — 键盘、IME、PageUp/PageDown、
  鼠标滚轮（Alt 加速）、点击单元坐标（工具卡片展开）/ keyboard, IME, page keys,
  wheel scrolling (Alt-accelerated), cell coordinates for tool-card clicks
- **文本选区与复制 / Text selection & copy** — 拖选高亮、Cmd+C 复制到系统
  剪贴板，内容漂移哨兵自动清除失效选区 / drag to select, Cmd+C copies to the
  system clipboard, a content-drift sentinel clears stale selections
- **文件树 / File tree** — gitignore 风格过滤、右键复制路径/Finder 显示、
  左键默认打开 / gitignore-style filters, copy path / reveal in Finder, default-open on click

## 架构 / Architecture

```
src/
├── app.rs              主状态与事件漏斗 (启动/切换/门控) / main state & event funnel
├── shell/              DockArea 三槽装配 / three-pane dock assembly
├── pane/               中央终端面板: 网格渲染 + 输入转发 / center terminal
├── property_detail_manager/  默认打开系统 (页签分发/dedup) / default-open dispatch
├── sidebar/            左槽: 切换入口 + Project/Agent 树 / project tree sidebar
├── tree/               文件树面板 / file tree panel
├── project_model/      领域模型: solutions → Project/Module (纯数据层) / domain model
├── workspace/          工作区实体 + 全局历史 + legacy 迁移 / workspace, history, migration
├── workspace_property_source/  文件树数据源 (插件化) / pluggable tree data source
└── agent_entities/     会话实体 (pi-shell-rs 嵌入) / session entities

pi-shell-rs (git)       TUI 引擎 (ssh git 依赖) / TUI engine (ssh git dep)
openspec/               spec-driven 规格与归档 / specs & archived changes
```

技术栈 / Stack：`gpui` · `gpui-kit` · `gpui-component` · `vt100` · `pi-shell-rs` · Rust 2024

## 快速开始 / Quick Start

前置要求 / Requirements:

- Rust toolchain（edition 2024）
- 可访问 `ssh://git@github.com/xxponline/pi-shell-rs.git` 的 SSH 密钥
  （pi-shell-rs 为 git 依赖；本机 `net.git-fetch-with-cli = true`）
- `pi` CLI 可用（pi-shell-rs 经 `resolve_binary` 拉起 `pi --mode rpc` 会话；
  the `pi` command must be resolvable in `PATH`）

```bash
cargo run      # 启动 / launch the workbench
cargo test     # 无头测试套件 / headless test suite
```

## 规格 / Specifications

本仓库以 spec-driven 方式开发，行为契约维护在 `openspec/specs/`：

Development follows OpenSpec; durable behavior contracts live in `openspec/specs/`:

| Capability | 说明 / Description |
| --- | --- |
| `agent-shell` | 会话外壳：Project 树、焦点、键盘/滚轮/点击转发、选区复制 (27 reqs) |
| `center-tabs` | 中槽页签与默认打开系统 (6 reqs) |
| `dock-layout` | 三槽停靠布局 (5 reqs) |
| `file-tree` | 文件树数据源与交互 (11 reqs) |
| `project-model` | Solution → Project/Module 结构与加载契约 (5 reqs) |
| `theming` | 浅色/深色主题与终端配色映射 (5 reqs) |
| `workspace-management` | 单工作区生命周期：恢复/选择/创建/切换 (8 reqs) |

已完成的变更归档于 `openspec/changes/archive/`；
工作流：`openspec new change` → propose → apply → sync → archive。

Completed changes are archived under `openspec/changes/archive/`.

## License

[MIT](LICENSE) © xxponline
