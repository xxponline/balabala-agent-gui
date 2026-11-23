# Tasks

## 1. 依赖与模块骨架

- [x] 1.1 `Cargo.toml` 增加 target 专属依赖：`libc`（macOS）、`windows-sys`
      0.61（Windows，features 按 Toolhelp32 所需），验证 `cargo check`
      通过且 `Cargo.lock` 无新增下载面（两者已在 lock 中）
- [x] 1.2 建 `src/debug/` 模块骨架：`#![cfg(debug_assertions)]` 模块门控 +
      `DebugSection { id, title }` 注册表结构 + 空注册函数，验证 debug
      构建编译通过、`cargo build --release` 编译通过且无 debug 模块符号
      （`nm`/`strings` 抽查或确认 cfg 块整块不进二进制）

## 2. rate_log 报告槽（design D7）

- [x] 2.1 `crates/agent-service/src/terminal/mod.rs` 的 `maybe_print` 在
      `eprintln` 之外写入 `static LATEST: Mutex<Option<Report>>`（同份汇总，
      计数器 swap 语义不变），验证：现有 rate_log 单测全过 + 新增单测断言
      写槽后 `eprintln` 输出格式不变
- [x] 2.2 agent-service 暴露 `pub fn` 读取最新 `Report`（含 None = 静默
      窗口未写过），验证单测：无数据时返回 None、有 feed 后返回 Some

## 3. 跨 crate 观测通路（design D8/D9 的 dropped_frames 部分）

- [x] 3.1 `AgentService::dropped_frames` 改为 process 全局原子计数 +
      `pub fn` 读取（原字段访问点同步改写），验证：agent-service 现有
      丢帧行为测试全过 + 新增读取函数单测

## 4. OS 线程数三平台（design D9）

- [x] 4.1 实现 `process_thread_count() -> Option<u32>` 三平台分支：
      macOS `proc_pidinfo(PROC_PIDTASKINFO)` 取 `pti_threadnum`、Linux 读
      `/proc/self/status` 的 `Threads:` 行、Windows Toolhelp32 快照数本
      pid；验证：macOS 本机单测（值 > 0 且 >= 1）、Linux 分支单测（可用
      条件编译在 CI/本机验）、`rustup target add x86_64-pc-windows-msvc
      && cargo check --target x86_64-pc-windows-msvc` 通过
- [x] 4.2 Windows 真机验收：在 Windows 环境跑 debug 构建，打开线程数
      section，核对显示值与任务管理器线程数一致（沿用 shell.rs pi.cmd
      的既有实测流程），验证：数值一致且开/关 section 数值随会话数变化
      - 记录 (2026-11-23)：用户选定收口方案 B — 记为**已知待验项**（同
        6.4 记法）：Windows 分支已有编译验证兜底（4.1 独立 crate
        `x86_64-pc-windows-msvc` check 通过），真机验收挂既有 pi.cmd
        Windows 实测流程后续补验；风险已在 design.md Risks（D9 条）登记。

## 5. 中槽开签平行入口（specs/center-tabs delta）

- [x] 5.1 实现 section 开签入口：查 section id dedup 表 → 已开
      `select_panel` 聚焦 / 未开 `add_panel_view(Center)` 尾插 + `on_removed`
      摘 dedup，验证：无头测试覆盖"尾插在 console 后 / 二次打开聚焦不
      重复 / 关闭后可重开 / 与文件路径身份互不干扰"四个场景
- [x] 5.2 实现 debug section 的 Panel 包装（Entity section 渲染进签，
      `closable=true`），验证：无头测试断言签可关且 console 恒第一签
      不变

## 6. 菜单栏（specs/app-menu-bar + design D4/D6/D11）

- [x] 6.1 装配期调用 `cx.set_menus`：Edit 菜单含 Copy 项（接现有
      `TerminalCopy` action），验证：无头测试断言菜单内容存在 + Copy 项
      可用性随选区灰显（`is_action_available` 路径）
- [x] 6.2 Debug 子菜单整块 `#[cfg(debug_assertions)]`：菜单构建函数从
      注册表产出 section 项，release 构建整块不产出，验证：debug 构建
      菜单含 Debug 菜单、`cargo build --release` 二进制不含 Debug 菜单
      字符串
- [x] 6.3 菜单项 action 分发接线：点击菜单项 → `cx.on_action` 全局监听 →
      走 5.1 开签入口，验证：无头测试断言选菜单项后对应 section 签打开
- [x] 6.4 Win/Linux 兜底路径 headed 验收（design D4 Open Question）：
      先验 `set_menus` 在 Win/Linux 的呈现，不理想则把 gpui-component
      `AppMenuBar` 挂进 `TerminalApp::render` 的 dock_area 之上，验证：
      菜单可点、Copy 行为与 macOS 一致（无 Windows/Linux 机器时记录为
      已知待验项）

## 7. 首批 section（specs/debug-console 数据口径）

- [x] 7.1 term-rate section：Entity 按秒读 `Report` 槽渲染全部指标
      （回显/帧→画/feed 速率/帧 KB/since_feed/render_cost），验证：注入
      已知 `Report` 的无头测试断言各字段渲染 + 关闭页签后无采样（读取
      停止）
- [x] 7.2 threads/pipeline section：Entity 按秒采样并渲染 —— OS 线程数
      （None 显示 unavailable）、`dropped_frames`、`manager.pending_count()`
      、GUI listener 积压（如需给 listener 加 `len()`）、`sessions_view`
      会话数，验证：无头测试注入已知值断言渲染 + "不可见即不采样"
      断言（OS 查询函数带调用计数探针）

## 8. 注册接线与集成验收

- [x] 8.1 各模块在装配期注册 section（term-rate、threads/pipeline 两节），
      验证：debug 构建启动后 Debug 菜单列出两节、依次开签正常
- [x] 8.2 release 集成检查：`cargo build --release` + 启动，验证无菜单
      泄漏（无 Debug 菜单）、Edit/Copy 仍在、`cargo test` 全绿
- [x] 8.3 headed 手动验收清单执行并记录：菜单栏出现、Edit/Copy 灰显随
      选区、Debug 菜单开签/聚焦/重开、term-rate 数值与 stderr 行一致、
      线程数随会话增减、关签停采样、cmd-c 快捷键行为与改前一致
      - 记录 (2026-11-23)：用户本机 `cargo run` (debug) headed 验收通过
        — 菜单栏/Edit·Copy/Debug 开签/cmd-c 无异常，term-rate stderr
        数据流正常 (会话期 echo/paint/feed/帧KB 指标活动可见, 与面板同源
        同值)；关签停采样与菜单内容/灰显由无头测试锁定 (close_tab_stops_
        sampling、invisible_stops_sampling、get_menus 断言、release 专项)。
        用户确认无问题。

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied.
- Verify the archived result.
