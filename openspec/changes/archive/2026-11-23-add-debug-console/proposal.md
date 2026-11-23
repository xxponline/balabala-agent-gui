# Proposal

## Why

开发期排查性能与管线问题只能靠 stderr 里定时喷的 `[term-rate]` 行 —— 数据已经
存在但不可交互、不可按需查看，服务侧的丢帧/积压计数则完全没有出口；同时应用
至今没有菜单栏，已有快捷键（如 cmd-c 复制选区）没有任何可见入口。这个 change
立起一个 debug-only 的调试面板体系（模块可注册自己的调试 section、以中槽页签
打开），并把菜单栏作为其常驻入口立起来。

## What Changes

- **新增 debug-only 调试面板模块**（`src/debug/`，整体 `#[cfg(debug_assertions)]`）：
  进程内模块向注册表登记 section（Entity 形态，能持状态/订阅/响应点击），
  release 构建中注册表为空、代码零成本。
- **debug section 以中槽页签打开**：平行于现有 `open_object` 路径的开签入口，
  dedup key = section id（非文件路径）；已开则聚焦，未开则尾插（console 恒
  第一签不变）。
- **新增常驻应用菜单栏**（OS 原生优先：macOS 屏幕顶部系统菜单栏；Win/Linux
  窗口内 AppMenuBar 兜底）：release 提供标准菜单（Edit/Copy 接现有
  `TerminalCopy` action，可用性自动灰显）；**Debug 子菜单整块仅 debug 构建
  存在**，内容 = 已注册 section 列表（关闭的调试签由此再打开）。
- **rate_log 增加报告槽**：`maybe_print` 每秒汇总时在 `eprintln` 之外写入
  全局 `LATEST` 槽（单写者，计数器语义不变，eprintln 保留）；GUI 只读该槽。
- **agent-service 暴露 debug 快照通路**（process 全局静态 + `pub fn`）：
  首批仅 `dropped_frames` 需要跨 crate，其余指标 GUI 本地直读。
- **首批两个 debug section**：
  1. term-rate 统计（回显延迟/帧→画/feed 速率/帧大小，镜像 stderr 口径）；
  2. 会话线程数与帧管道深度（进程真实线程数走 OS 真值：macOS
     `proc_pidinfo` / Windows Toolhelp32 / Linux `/proc/self/status`；
     加 `dropped_frames`、router 在途 ask、GUI listener 积压、会话数）。
- **依赖新增**：`libc`（macOS target）、`windows-sys`（Windows target，均已在
  依赖树中，仅提升为直接依赖）。
- **明确不在范围**：最近关闭栈、终端 Paste 链路、contracts/dylib 侧注册通路、
  环境变量开关、服务状态页（既有 deferral 不变）。

## Capabilities

### New Capabilities

- `app-menu-bar`: 常驻应用菜单栏 —— 平台呈现（OS 原生 / 窗口内兜底）、标准
  菜单内容、菜单项经 action 分发且可用性自动灰显、内容变更后的菜单重建。
- `debug-console`: debug-only 调试面板 —— section 注册表的编译期门控、Debug
  子菜单入口、首批 section 的数据口径（term-rate 镜像；线程数与管道深度指标）。

### Modified Capabilities

- `center-tabs`: 中槽新增 debug section 页签形态 —— 尾插在 console 之后、
  按 section id 去重（区别于文件路径身份）、关闭后可经菜单重开。

## Impact

- **新增**：`src/debug/`（注册表 + 首批 section + OS 线程数三平台实现）。
- **修改**：`src/main.rs`/`src/app.rs`（菜单栏装配、action 监听、Win/Linux
  窗口内菜单挂载点）、`src/property_detail_manager/` 邻近的开签路径（平行
  入口，不动 `open_object`）、`crates/agent-service/src/terminal/mod.rs`
  （rate_log 报告槽）、`crates/agent-service/src/service.rs`（`dropped_frames`
  全局计数出口）。
- **依赖**：`Cargo.toml` 增加 target 专属 `libc`/`windows-sys`（已在
  Cargo.lock 中，无新增下载面）。
- **不影响**：contracts 契约、service-plugin/dylib 装载、右 dock 行为、
  现有文件开签去重语义、`cfg(test)` 测试地基。
