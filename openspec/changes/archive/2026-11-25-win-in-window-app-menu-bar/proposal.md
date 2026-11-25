# Proposal

## Why

`add-debug-console` 立菜单栏时走的是 design D4 的「OS 原生优先」：全工程只有
`cx.set_menus(build())` 一行，把呈现交给平台。而 Windows 后端
（`gpui-pre-windows` / `gpui-fast-windows`）的 `set_menus` **只把菜单存进一个
`RefCell` 就结束了** —— 两份实现里 `HMENU` / `CreateMenu` / `AppendMenu` /
`SetMenu` / `WM_COMMAND` 全部 0 命中，`get_menus()` 在整个依赖树里的唯一消费者
是 `App::get_menus`，也就是我们的无头测试。结果是：**Windows 上一个菜单像素都
不显示，而 4 个菜单测试全绿** —— 它们断言的是存储，不是呈现。

这一点在 `add-debug-console` 的 design Risks 里已被代码级读出（「原生菜单栏
大概率不出现，D4 的窗口内 `AppMenuBar` 兜底大概率必需」），task 6.4 当时因
无 Windows 机器按方案 B 记成「已知待验项」。现在真机在 Windows 上，这一项到验
了，验出的结果就是不显示。

同时暴露两个 spec 已经要求、但从未被任何平台真正兑现的缺口：

- **快捷键在 Windows 上是坏的**：`KeyBinding::new("cmd-c", ...)` 走无 OS 的
  `Keystroke::parse`，`cmd` 解析成 `platform` 修饰键 = `VK_LWIN/VK_RWIN`。所以
  Windows 上 Copy 绑的是 **Win+C**（Win11 全局热键，八成进不了应用），而
  Ctrl+C 因 `handle_key_down` 对 control 早退既不转发会话也不触发 action ——
  **Copy 两侧都按不出来**。菜单栏一旦显示，还会把这个假快捷键标给用户看。
- **菜单内容重建从未有过触发路径**：`app-menu-bar` 第 3 条要求内容变化后重建，
  但全工程只有 `menu.rs:53` 一处 `set_menus`，`debug::register()` 改完注册表
  什么都不做 —— macOS 也一样缺。

## What Changes

- **窗口内 `AppMenuBar` 兜底（D4 的既定退路，Windows/Linux）**：喂
  `GlobalState::set_app_menus` → 建 `AppMenuBar` 实例 → 挂进
  `TerminalApp::render` 的 dock 之上。macOS 继续走原生 `set_menus`，`set_menus`
  调用保留（macOS 呈现 + 现有 4 个 `get_menus()` 测试都靠它）。
- **布局硬性改动**：`AppMenuBar` 根节点是 `size_full`（`w:100% h:100%`），
  直接塞进现有 `flex_col` 会与同样 `size_full` 的 `dock_area` 各要 100% 高 →
  菜单栏被压成半窗。必须加两个 wrapper：菜单栏定高 + `flex_shrink_0`，dock 换
  `flex_1().min_h(px(0.))`。
- **Copy 灰显**：`disabled = !selection.is_some()`，在 `TerminalApp::render`
  里比对 bool 翻转、翻了才重建菜单。**只看选区，焦点不参与判定**（见下）。
- **快捷键改 `secondary-`**：`cmd-c` → `KeyBinding::load("secondary-c", ..., cx)`
  （`KeyBinding::new` 走的 parse 无 OS，遇 `secondary` 会 panic，必须换 `load`），
  `cmd-shift-m` 同理。macOS 语义不变（`secondary` → platform），Windows 变成
  Ctrl+C。
- **菜单重建通路**：统一的 `set_app_menus` + `AppMenuBar::reload()` 两步重建，
  供 ②（选区翻转）与 ④（注册表变化）共用。
- **明确不在范围**：原生 Win32 菜单栏（`SetMenu` + WNDPROC 子类化，要和 gpui
  抢窗口过程，风险不成比例）；焦点参与灰显判定（理由见下）；`cmd-c` 之外的
  其它键位；Paste 链路。

## Capabilities

### New Capabilities

（无 —— 窗口内兜底、灰显、重建都属 `app-menu-bar` 既有职责范围。）

### Modified Capabilities

- `app-menu-bar`:
  1. 第 1 条「非原生平台渲染在窗口内」补一条 **Windows 真机** 的显式 scenario
     —— 现有两条 scenario 只断言「菜单在场」，被 `get_menus()` 存储断言满足，
     从未覆盖呈现。
  2. 第 2 条「available **in the current context**」**收窄**为「随选区」并写明
     焦点不参与判定 —— 窗口内弹出菜单在打开期间自行持有焦点，开合期求值可用性
     会判成不可用，而 `AppMenuBar::reload()` 无条件清 `selected_index`，等于
     菜单一开就自关。macOS 原生经 `on_validate_app_menu_command` 在**开菜单前**
     求值所以没这问题；窗口内路径没有 open 钩子，拿不到那个时机。
  3. 第 3 条「内容变化后重建」补一条**触发者**的明确性（注册表变更与选区翻转
     共用同一条重建通路）。
- `agent-shell`: 「Copy selection with **Cmd+C**」的键位措辞改为**平台主修饰键**
  （macOS = Cmd，Windows/Linux = Ctrl）。行为修正而非行为变更：macOS 不变，
  Windows 从「绑 Win+C 且按不出来」变为「绑 Ctrl+C 且可用」。**BREAKING**
  （仅 Windows/Linux 的键位语义）。

## Impact

- **代码**：
  - `src/menu.rs` —— `install()` 增加 `GlobalState::set_app_menus` + 建
    `AppMenuBar` 实例；抽出统一的重建函数。
  - `src/app.rs` —— `TerminalApp` 增加 `menu_bar` 字段；`render()` 根容器加两个
    wrapper 并挂菜单栏；渲染期比对选区翻转触发重建；`bind_keys` 两处改
    `KeyBinding::load`。
  - `src/menu_tests.rs` —— 现有 4 个 `get_menus()` 测试保持绿（`set_menus` 保留）；
    新增**渲染级**断言（帧文本含 `Edit`/`Debug`），需 `cfg` 门控。
- **依赖**：无新增。`gpui-component`（`component` feature 默认开）与
  `gpui-base::GlobalState` 均已在依赖树中。
- **测试风险已评估**：
  - 新增菜单文本是 `Edit`/`Debug`/`Copy`，扫过全部 `texts.contains` 断言，
    与 `+ agent`、`过滤:` 均不撞。
  - 网格 bounds 测试的 `exp_rows` 从实际 `bounds.size.height` 反算，自洽；
    唯一绝对断言 `width < 800` 是宽度，不受横向菜单影响。
  - ⚠️ 渲染级新测试必须 `cfg(not(target_os = "macos"))` —— 窗口内菜单栏只在
    非 mac 挂载，macOS 上跑同一份测试会找不到 `Edit` 文本。
- **spec**：`app-menu-bar` 三处 delta、`agent-shell` 一处 delta。
- **验收**：菜单栏高度取值需真机目视（`Button::small` = `size_6` = 24px，
  `AppMenu` 又叠 `.py_0p5()`）；「Win+C 是否真被系统吃掉」需真机按键确认 ——
  该条目前只有代码级推断。
