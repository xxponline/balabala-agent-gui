# Design

## Context

- 中槽开签已有统一后端：`TerminalApp::open_object` 查路径 dedup 表 →
  `add_panel_view(Center)` 尾插，handler 走静态注册表 `HANDLERS`
  （`src/property_detail_manager/mod.rs`）。console 恒第一签由尾插 +
  `closable=false` 保证（specs/center-tabs）。
- `rate_log`（`crates/agent-service/src/terminal/mod.rs` 内私有模块）是
  process 全局原子计数，`maybe_print` 每秒**破坏性**汇总（swap 清零）后
  `eprintln` —— 单写者，不能直接插第二个读者。
- 项目从未调用过 `cx.set_menus`：现有快捷键（cmd-c / cmd-shift-m）全靠
  keymap 活着，与菜单无关；菜单栏是净新增。
- gpui-fast 的菜单链路已验证：菜单项点击 → `on_app_menu_action` 回调 →
  `cx.dispatch_action` → 焦点窗口 action 树或 `cx.on_action` 全局监听；
  `on_validate_app_menu_command` → `is_action_available` 实现自动灰显；
  keymap 反查 `bindings_for_action` 自动给菜单项标快捷键。
- `gpui_kit::actions!` + `cx.bind_keys` 已在用（`ToggleMdView`）。
- 线程数要 OS 真值（选项 a）：线程大头在 pi-shell-rs（git 依赖，无计数
  接口），估算值不可信；`sysinfo` 0.31.4 无 macOS/Windows 线程数 API（已查证死路）。

## Goals / Non-Goals

**Goals:**
- debug-only 的 section 注册表 + 中槽开签 + 菜单入口，骨架先被两个 section 跑通。
- 菜单栏作为独立常驻功能立起来（release 也有价值：Edit/Copy 可见性）。
- 采样零成本原则：不打开签 = 不采样、不注册代码。

**Non-Goals:**
- 最近关闭栈、终端 Paste 链路（缺口已知，另立 change）。
- contracts/dylib 侧的调试注册通路（范围 A：进程内 GUI 模块）。
- 服务状态页（既有 deferral 不变）。
- 环境变量/运行时开关（`cfg(debug_assertions)` 一刀切）。

## Decisions

### D1: 范围 A —— 纯进程内注册表，数据驱动通路不建

注册 = GUI 进程内模块在启动期把 `DebugSection { id, title, factory }` 塞进
注册表。`service-plugin` 的依赖铁律（plugin → contracts + core，不碰 GUI）
决定了 dylib 侧永远不能传 trait 对象，那是另一个数量级的设计（描述 +
action 消息的协议）。**备选**：数据驱动协议统一进程内外 —— 现在不做，
等有真实需求再立 change。

### D2: section = Entity，不是 render 闭包

`fn(&mut Context) -> Div` 拿不到自己的状态/事件；首批 section 就需要
"数字每秒刷新"和（将来）"点按钮触发动作"。Entity 能 `cx.new`、订阅、
持 `FocusHandle`。**备选**：闭包更轻 —— 立刻撞墙，不值。

### D3: 平行开签入口，不动 `open_object`

debug section 的身份是 `id: &'static str`，不是 `PathBuf`，也不是
`AssetKind` —— 塞进 `OpenObject` 要泛化身份语义，会动 specs/center-tabs
"绝对路径去重"这条既有 requirement。**做法**：平行入口复用同一套
`add_panel_view(Center)` 尾插 + 独立 dedup 表（key = section id 字符串），
console 恒第一签的不变量天然保持。**备选**：泛化 `OpenObject` 身份为
任意 key —— 改动面大、波及文件开签测试，收益为零。

### D4: 菜单栏 = OS 原生优先，Win/Linux 窗口内兜底

macOS 走 `cx.set_menus`（真系统菜单栏，快捷键自动带出，零布局改动）；
Win/Linux 第一版同样先试 `set_menus`（gpui-fast-windows/linux 有实现），
若实测不理想再上 gpui-component 的 `AppMenuBar`（读
`GlobalState::set_app_menus`，渲染进窗口、挂在 `TerminalApp::render`
的 dock_area 之上 —— `app.rs:1038` 注释已预留该容器位）。两边读同一份
action 注册，内容一致。**备选**：只做窗口内 AppMenuBar —— 放弃 macOS
原生菜单的免费快捷键可见性。

### D5: `cfg(debug_assertions)` 掐两处，无环境变量

release 编译期抹掉：(1) `src/debug/` 注册表为空表、section 代码不进
二进制；(2) 菜单构建函数整块不产出 Debug 子菜单。**备选**：env 开关
双保险 —— 引入"忘了设"的测试矩阵，debug 构建默认全开已够用；要隐藏
某个 section 是注册方自己的事（不注册即可）。

### D6: 菜单栏常驻，Debug 子菜单才受 cfg

release 菜单提供标准件（Edit/Copy 接现有 `TerminalCopy`，灰显由
`is_action_available` 白捡）。Paste 链路缺口已确认（无 cmd-v 绑定、无
paste→send_text 路径），**本期不接**，Edit 菜单暂只放 Copy。**备选**：
整个菜单栏 debug-only —— release 里 macOS 无菜单栏体验残废，且浪费了
已验证的 action 链路。

### D7: rate_log 报告槽（单写者双出口）

`maybe_print` 每秒汇总时，在 `eprintln` 之外把同一份汇总写进
`static LATEST: Mutex<Option<Report>>`；GUI 只读该槽，计数器仍由
rate_log 独占（swap 语义不变）。eprintln 保留 —— stderr 与面板是同一
数据源的两个出口，不会漂。`Report` 是普通 struct，release 里写槽照常
（几十字节），不加 cfg。**备选**：GUI 非破坏快照直读计数器 —— 要改
EchoWindow 清零语义、动热路径，为一个只读面板不值。

### D8: 跨 crate 取数走通路 1（process 全局静态 + `pub fn`）

首批只有 `dropped_frames` 需要跨 crate（服务 actor 手里，GUI 够不着）：
在 agent-service 侧改为全局原子计数 + `pub fn` 读取。**备选**：
- 契约消息 ask（走 bus）—— debug 数据每秒一问一回复太重，且消息归属
  规则要求进 contracts crate，边界情况不值得开；
- 构造注入 `Arc<Mutex<Stats>>` —— AgentService 是 manager 内部 factory
  构造的，GUI 拿不到句柄，要改装配。
其余指标（`manager.pending_count()`、GUI listener 积压、`sessions_view`）
GUI 手里直读，零通路。

### D9: 线程数走 OS 真值，三平台

```
 macOS   libc::proc_pidinfo(PROC_PIDTASKINFO) -> proc_taskinfo.pti_threadnum
 Windows CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD) -> 数本 pid 线程
 Linux   读 /proc/self/status 的 "Threads:" 行
```

各约 15/35/10 行，失败一律返回 `None` → UI 显示 unavailable。依赖：
`libc`、`windows-sys` 0.61 已在 Cargo.lock（传递依赖），提升为 target
专属直接依赖，无新增下载面。**备选**：
- `sysinfo` crate —— 0.31.4 无 macOS/Windows 线程数 API，死路；
- `ntapi::SYSTEM_PROCESS_INFORMATION.NumberOfThreads` —— 要分配大缓冲
  解析系统信息，复杂度翻倍；
- sessions × 5 估算 —— 调试面板显示估算值没有意义。

**成本注记**：Windows 侧真机验证需要 Windows 环境（项目已有先例，
shell.rs 的 pi.cmd 实测）；编译验证本机 `rustup target add` +
`cargo check` 即可（check 不链接）。

### D10: 采样时机 = section 可见时

两个 section 都只在页签可见时按秒采样（读 `LATEST` / 查 OS / 读计数），
关闭或切走即停。OS 查询（尤其 Windows Toolhelp32 是系统级快照）在 1Hz
下开销可忽略，但"不打开 = 零开销"是硬约束（specs/debug-console 已写入）。

### D11: 菜单重建时机

注册表在启动期填充（`TerminalApp::new` 装配阶段），菜单在装配完成后
构建一次；运行期若 section 注册变化（首批不会有），按
specs/app-menu-bar 的重建 requirement 整根 `set_menus` 重推。动态内容
（将来若有）每次变都重设整根菜单 —— 已知代价，接受。

## Risks / Trade-offs

- [Windows Toolhelp32 / macOS proc_pidinfo 在本机无法真机验证] → 编译期
  `cargo check` 交叉目标兜底；真机验证作为 tasks 里的显式验收项，挂到
  已有的 Windows 实测流程上。
- [Win/Linux 菜单栏 headed 验收待真机 (task 6.4 已知待验项)] → 本机仅
  macOS, 按 task 6.4 退路记录。代码级已核: gpui-pre 与 gpui-fast 的
  windows/linux 四份 `set_menus` 实现**全部仅存储** — 全生态
  `get_menus()` 唯一消费方是 `App::get_menus`, Windows 侧零
  HMENU/AppendMenu 调用, Linux 侧 `set_dock_menu` 仍 `todo(linux)` →
  原生菜单栏大概率不出现, D4 的窗口内 `AppMenuBar` 兜底大概率必需
  (通路在位: `gpui_kit::component::menu::AppMenuBar` 读
  `gpui_kit::base::GlobalState::{app_menus,set_app_menus}`, 项目默认
  component feature 已开; 挂载点 = `TerminalApp::render` 根容器注释处 —
  D4 所引 app.rs:1038 已漂至 ~1051)。待验: 真机 set_menus 呈现、菜单可点、
  Copy 行为与 macOS 一致; 交叉 check 亦受阻 (Windows: gpui-pre 构建脚本
  缺 llvm-rc, 同 task 1.1; Linux: x86_64-unknown-linux-gnu target std 未装)。
- [菜单栏净新增，可能影响现有快捷键] → 菜单项走 action 分发，与 keymap
  是同一 action；不绑新快捷键、不改现有 `bind_keys`，回归面 = 菜单出现
  后 cmd-c 行为不变（有测试覆盖 selection copy）。
- [`maybe_print` 加写槽引入锁] → 槽锁只在每秒一次的汇总路径上，
  `note_feed/note_render` 热路径不碰它；`LATEST` 写入用 try_lock 或
  沿用 ECHO 同款 poisoning 容忍。
- [全局静态让测试互相污染] → 与 rate_log 现状同款（已有教训）：
  debug 快照的测试按"读前后差值"断言，不清全局；section 测试注入
  本地 Report。
- [Debug section 忘记 cfg 泄漏进 release] → 模块级 `#![cfg(debug_assertions)]`
  + 菜单构建函数的 cfg 块，tasks 里加 release 构建检查项。

## Migration Plan

无数据迁移、无部署序列。单 change 内落地：骨架（注册表/开签/菜单）→
rate_log 报告槽 → 两个 section → 三平台线程数。回滚 = revert 整个
change（新增为主，触碰的既有路径只有 `maybe_print` 一处且行为向后兼容）。

## Open Questions

- Win/Linux 首版用 `set_menus` 还是直接上窗口内 `AppMenuBar`：D4 定了
  "先试 set_menus，不理想再兜底"，具体判定留给 tasks 的 headed 验收。
- Edit 菜单除 Copy 外还要不要 Window/Help 等标准件：不影响 specs（spec
  只要求"至少一个菜单"和 Copy 行为），实现时按平台惯例补。
