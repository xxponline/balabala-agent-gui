# Design

## Context

动机与范围见 proposal.md（Why / What Changes）。这里只记**约束现状** —— 全部
来自代码级核查，是本设计的硬边界：

- **两份互不相交的菜单存储**：
  - `cx.set_menus(menus)` → 平台侧 `RefCell<Vec<OwnedMenu>>`（Windows 仅存储，
    零渲染；macOS 原生渲染；无头测试经 `App::get_menus` 读它）。
  - `GlobalState::set_app_menus(owned)` → `gpui-component` 的
    `AppMenuBar::reload()` 读它。**全依赖树零调用方**。
- **`AppMenuBar` 从未被实例化过**（`gpui-component` 内只有它自己的焦点单测）。
  根节点 `div().relative().size_full()` = `w:100% h:100%`
  （`gpui-pre-macros/styles.rs`：`prefix:"size"` × `suffix:"full"` →
  `relative(1.)`）。
- **`AppMenuBar::new()` 内部立刻 `reload()`** 读 GlobalState；`reload()` 无条件
  `selected_index = None; action_context = None`。
- **弹出菜单打开时抢焦点**（`build_popup_menu` 里 `focus_handle.focus()`），且
  `AppMenuBar` **没有 open 钩子** —— `on_will_open_app_menu` 只被平台原生菜单
  调用，组件内零调用方。
- **`gpui-component` 全库 0 处 `is_action_available`**；`PopupMenu` 只读
  `OwnedMenuItem::Action.disabled` 静态字段，`activate()` 经 `is_clickable()`
  拦掉 disabled 项。
- **Windows 修饰键**：`platform` = `VK_LWIN/VK_RWIN`
  （`gpui-pre-windows/events.rs:1788`）；`KeyBinding::new` 走无 OS 的
  `Keystroke::parse`，遇 `secondary` 会 `unwrap()` panic。
- **`AgentTerminal.selection` 是 `pub`**，变化处均 `cx.notify()`；notify 经
  `mark_view_dirty` + `view_path_reversed` 向上标脏，祖先链含 `TerminalApp`。
- **`TerminalApp::render` 是 memoized 的**（`view.rs` 按 bounds/mask/style +
  `dirty_views` 判定复用），不是每帧必跑。

## Goals / Non-Goals

**Goals:**

- Windows/Linux 上菜单栏真正出现在窗口内、dock 之上，且 dock 拿到剩余全部空间。
- Copy 灰显随选区变化，下次开菜单即生效，无需重启。
- 快捷键按平台解析（Windows = Ctrl+C），菜单上标注的即实际生效的那个。
- 注册表与选区两类变化走**同一条**重建通路。
- 现有 4 个 `get_menus()` 无头测试不回归。

**Non-Goals:**

- 原生 Win32 菜单栏（`SetMenu` + WNDPROC 子类化）。
- 焦点参与灰显判定（见 D2）。
- `cmd-c` / `cmd-shift-m` 之外的键位梳理。
- Paste 链路、最近文件等其它菜单项。

## Decisions

### D1: 先喂 GlobalState，再建 AppMenuBar 实例

`AppMenuBar::new()` 内部立刻 `reload()` 读 `GlobalState::app_menus()`。顺序反了
首帧就是一条空菜单栏，且没有任何报错提示。因此 `menu::install()` 内固定为：

```
set_app_menus(build().into_iter().map(|m| m.owned()).collect())
        ↓ 必须先
AppMenuBar::new(cx)   → 内部 reload() 读到已就位的数据
```

`cx.set_menus(build())` **保留**（macOS 原生呈现 + 现有 `get_menus()` 测试）。

**备选**：只喂一处、让 macOS 也读 GlobalState —— 否决。macOS 原生菜单走的是
平台侧存储，改它要动原生渲染路径，风险远大于收益，且会让 4 个现有测试全失效。

### D2: 灰显只看选区，焦点不参与

`disabled = !selection.is_some()`，在 `TerminalApp::render` 内读 `pub selection`
比对缓存 `bool`，**翻转才**走重建。

**为什么不碰焦点**：焦点感知在窗口内路径上会自杀 —

```
点 Edit → set_selected_index 记下 action_context(仍在终端)
       → 下一帧 build_popup_menu → focus() 抢走焦点
       → 此刻评估 availability = selection && focus_in_terminal = false
       → reload() → selected_index = None → 菜单刚开就自关
```

根因是「焦点在终端」在菜单打开期间**恒为假**（菜单自己持有焦点），而
`AppMenuBar::reload()` 无条件清 `selected_index`。macOS 原生经
`on_validate_app_menu_command` 在**开菜单前**求值所以无此问题；窗口内路径没有
open 钩子，拿不到那个时机。

**备选 (b) 焦点感知 + 开合守卫** —— 否决：`selected_index` / `action_context`
均为私有，外部读不到开合状态，只能靠鼠标位置或焦点归属猜测，脆且不可测。

**备选 (c) 自写菜单栏**（不用 `AppMenuBar`，自己写标题按钮 + 复用
`PopupMenu`，在开合那刻求值 items）—— 否决：等于重写 AppMenuBar 的键盘导航/
滚动/开合，为一个灰显场景付出不成比例的工作量。

**代价（已接受）**：焦点在侧栏 + 有选区时菜单显示可用，点下去
`dispatch_confirm_action` 恢复 `action_context`（侧栏焦点）后派发，路径上无
`TerminalCopy` 监听 → 静默无操作。macOS 原生此时会灰显。已在 spec delta 中
显式写成「availability 由选区单独决定，焦点不参与」，把该偏差转成契约。

### D3: 重建 = 两步，且必须比对翻转

统一重建函数：

```
set_app_menus(build().into_iter().map(|m| m.owned()).collect())
menu_bar.update(|m, cx| m.reload(cx))
```

**必须比对翻转**：`reload()` 会重建全部 `AppMenu` 实体并清 `selected_index`，
每帧无脑推 = 每帧重建实体 + 菜单一开就关。因此调用方只在 `bool` 翻转时调。

**两个触发方共用此通路**：
- ② 选区翻转 —— 在 `TerminalApp::render` 内比对（依赖 notify 向上标脏使
  `TerminalApp::render` 重跑）。
- ④ 注册表变化 —— `debug::register()` 之后显式调用。

**备选**：给 `debug::register` 加回调参数注入重建 —— 否决。`register` 是装配期
自由函数、被大量测试直接调用，改签名波及面大；注册表目前只在装配期
（`install` 之前）写入，测试侧直接调重建函数即可。

### D4: 快捷键改 `KeyBinding::load("secondary-...", ..., cx)`

`KeyBinding::new` 走 `Keystroke::parse`（`os = None`），`secondary` 分支需要 OS
否则返回 `Err` → `new` 内 `.unwrap()` **panic**。必须换 `KeyBinding::load`，
它用 `parse_with_os(source, cx.platform_info().os)`：macOS → `platform`
（= Cmd，语义不变），Windows/Linux → `control`（= Ctrl）。

`cmd-shift-m`（ToggleMdView）同理改 `secondary-shift-m`。

**备选**：`#[cfg(windows)]` 分别绑 `ctrl-c` / `cmd-c` —— 否决。两份条件编译的
绑定串是重复源，且 `secondary` 正是为此设计的别名。

### D5: 布局用两个 wrapper，不改 dock 内部

```
div().size_full().flex().flex_col()
  ├─ div().h(px(?)).w_full().flex_shrink_0()   ← 菜单栏 wrapper（定高）
  │    └─ AppMenuBar  (size_full → 100% of wrapper)
  └─ div().flex_1().min_h(px(0.))              ← dock wrapper
       └─ DockArea  (size_full → 100% of wrapper)
```

两个 `size_full` 兄弟直接并列会各要 100% 高 → 总高 200%，菜单栏被压成半窗。
`min_h(px(0.))` 是项目既有教训（`assets/panel.rs:350` 注释「flex 子项收缩
(pane 尺寸教训)」）。

**高度取值待真机**：`Button::small` = `size_6` = 24px，`AppMenu` 又叠
`.py_0p5()`，初值取 28px，headed 验收时目视调整。

**备选**：改 `DockArea` 根节点样式 —— 否决。那是 `gpui-base` 的代码，改不了也
不该 fork。

### D6: 平台门控只挡「挂载」，不挡「喂数据」

`AppMenuBar` 的**挂载**用 `#[cfg(not(target_os = "macos"))]`（macOS 走原生，
窗口内再挂一条会双份）。**喂 GlobalState 与 `set_menus` 不做门控** —— 无害且
让 macOS 侧数据保持单源。

**代价**：渲染级新测试必须同样 `cfg(not(target_os = "macos"))`，否则 macOS 上
找不到 `Edit` 文本会挂。

## Risks / Trade-offs

- [Windows 上原生菜单栏确实零渲染，本次靠代码级核查而非实测确认] → 已核
  `gpui-pre-windows` 与 `gpui-fast-windows` 两份 `set_menus` 全部仅存储、
  `HMENU`/`CreateMenu`/`AppendMenu`/`SetMenu`/`WM_COMMAND` 零命中；
  headed 验收作为 tasks 显式项，挂到既有 Windows 实测流程。
- [`Win+C 是否真被系统热键吃掉` 只有代码级推断] → **两条均真机实测**
  (task 4.4, 2026-11-24, Windows 10/11, debug 构建 `balabala-agent-gui.exe`，
  sentinel 前后夹测)：
  - ✅ **Ctrl+C 复制生效** — sentinel `W44-BEFORE` → 按键后变为用户实际
    选中的文本；真机链路 OS → gpui → keymap → `TerminalCopy` → 剪贴板走通，
    4.1 的 `secondary-c` → Ctrl 判定坐实；
  - ✅ **Win+C 不复制** — 用户明确确认已按下，sentinel `W44-BEFORE-WINC`
    完好、应用 PID 存活、日志无 panic → **未产生任何剪贴板写入**。
  4.4 原文的条件分支「若 Win+C 竟能触发则复核 4.1」**未触发**，无需复核。
  ⚠️ **归因仍未二分**：「系统热键吃掉」与「到达应用但 `modifiers` 不匹配」
  产生相同的可观察结果（都不写剪贴板）；本轮未记录是否弹出 Copilot/
  Widgets 面板。**但两者对正确性等价** —— 无论哪种 Win+C 都不触发本应用
  复制，且键位判定由 task 4.3 无头断言（`control && !platform` +
  `should_match` 逐字段比对）**构造性**独立保证，不依赖本归因。
  剩余不确定性属外部行为，不影响本 change 正确性，并入 task 6.2 一并观察。
  📌 **方法论教训**：sentinel 必须伴随「操作者确认已执行」—— 「无写入」
  无法区分「没按」与「按了没生效」，首轮就是把前者当成了后者。
- [菜单栏吃掉约 28px 高度，网格行数变化] → 网格测试的 `exp_rows` 从实际
  `bounds.size.height` 反算，自洽；唯一绝对断言 `width < 800` 是宽度，不受
  横向菜单影响。已扫全部 `texts.contains` 断言，`Edit`/`Debug`/`Copy` 与
  `+ agent`、`过滤:` 均不撞。
- [新增菜单文本进入帧文本，可能影响依赖文本序列的断言] → 未发现
  `assert_eq!(texts, ...)` 整组比较；负向断言只有 `+ agent` 与 `过滤:`，不撞。
- [焦点不参与灰显 → 焦点在侧栏时菜单显示可用但点击静默无操作] → 已接受并
  写入 spec delta（D2）；`handle_copy` 本身无选区即返回，不会误写剪贴板。
- [渲染级测试在 macOS 上会因挂载门控找不到文本] → 测试同样
  `cfg(not(target_os = "macos"))`；`get_menus()` 级测试保持跨平台。
- [`reload()` 重建实体，若在菜单打开期被调用会关掉菜单] → D3 强制比对翻转；
  选区在鼠标位于菜单上时不会变化，天然规避。
- [`TerminalApp::render` 是 memoized 的，未必每次选区变化都重跑] → 依赖
  notify 向上标脏使祖先 `TerminalApp` 进 `dirty_views`；若实测不触发，退路是
  在选区变化处显式 `cx.notify()` 于 `TerminalApp`（tasks 中留验证项）。
- [本机 `cargo test` 基线本身不绿，「全绿」不可达] → **验收基准改为「相对
  基线零新增失败」**。Windows 上既有 19 个失败（已 stash 改动跑基线对照，
  失败集合逐条 IDENTICAL），三类根因均与本 change 无关：
  **A 类（5 个，`test_sessions.rs:211`）** —— `write_stub` 产出的
  `fake-pi.sh` 是 `#!/bin/sh` 脚本，Windows 无法 spawn（`%1 is not a valid
  Win32 application`，可执行位也只在 `#[cfg(unix)]` 设置）；
  **B 类（7 个，`gpui-kit test.rs:142`）** —— 测试用
  `format!("cell-{}/stub-dir", ws.display())` 拼 ElementId 写死 `/`，
  Windows 实际路径分隔符为 `\`，`find` 找不到；
  **C 类（7 个）** —— `启动恢复未落定` 超时、`try_find_by_label("top.rs")`、
  solutions json 断言等，部分疑似 A 类下游。
  修复 Windows 测试可移植性是**正交的既有问题，不在本 change 范围**；
  本 change 的验证改为：`cargo test` 失败集合与基线 diff 为空 + 新增测试通过。
  ⚠️ 1.1 点名的 `app::tests::adaptive_grid_dispatch` 属 A 类，本机不可执行。
- [macOS 侧编译在本机不可验（task 2.2 第三条验收）] →
  `cargo check --target x86_64-apple-darwin` 卡在**依赖**上，未走到本项目
  代码：`onig_sys`（`gpui-kit` 的 `syntect` feature 拉入的 C 依赖）的
  build script 报 `cc-rs: failed to find tool "cc"` —— 本机 `cc`/`gcc`/`clang`
  均不存在（PATH 里有 MSYS2 但 `/ucrt64/bin` 为空）。即便装上 clang，
  交叉编译到 `x86_64-apple-darwin` 还需 **Apple SDK**，Windows 上不可得。
  **结构性论证**：本 change 的 `menu_bar` 字段与挂载**两平台源码同构、
  无 cfg 分叉**（唯一 cfg 在 `menu.rs::install` 内：非 mac 建实例 / macOS
  返回 `None`），故 task 2.2 担心的「字段/挂载缺失导致 macOS 编译失败」
  在构造上不可能发生；但「macOS 整体编译通过」**无编译证据，只有论证**。
  与 2.4 / 4.4 / 6.2 / 6.3 同类：留待 macOS 真机（task 6.3）一并验证。
  **已知代价**：因不采用字面 cfg，编译器不会强制字段/挂载/赋值三处
  cfg 一致 —— 日后若有人在 macOS 路径误加代码，需靠 review 而非编译器拦截。
- [`PopupMenu` 内菜单项的 `disabled` 不进 a11y 快照, 无头测试无法直接
  断言弹出菜单里的灰显] → **`gpui-component` 的观测盲区, 非本 change 缺陷**：
  `MenuItemElement::render` (menu/menu_item.rs) 只写 `role`/`aria_label`/
  `aria_selected`/`aria_toggled`, **从不把 `disabled` 写进 a11y node**
  (`self.disabled` 只控 hover/click 分支与 `muted_foreground` 前景色) ——
  故 `find_by_label("Copy").disabled()` 恒为 `None`, 与灰不灰无关（视觉上
  仍会灰, 只是拿不到观测位）。**任务 3.2 的断言口径因此分两半**：
  (a) 菜单真的开了 + Copy 项在场 → popup a11y (`find_by_label`);
  (b) 灰显值 → `GlobalState` 的 `OwnedMenuItem::Action.disabled`, 它被
  `PopupMenu::with_menu_items` **逐字**拷进 popup 项, 即渲染输入源。
  后续若要改回直接断言弹出菜单, 需先给 gpui-component 补
  `aria-disabled`。
- [Debug 注册表与 GlobalState 均为**进程全局**, 并行测试会互相
  append → 全等断言会 flake] → 已在 3.1 / 3.2 的测试里改用**前缀断言**
  (`starts_with`) —— `register` 只追加或同 id 原地覆盖, 已有项的相对顺序
  恒定, 前缀在并发下成立; spec 要的也是「已有项不被打乱」而非「总数不变」。
  `Edit` 菜单（静态 Copy）与菜单名序列不受影响, 保留全等断言。

## Migration Plan

无数据迁移。单 change 内落地：布局 wrapper → GlobalState 喂食 + 实例挂载 →
选区比对重建 → 快捷键 `load` → 重建通路接注册表。回 revert 即回到「Windows
无菜单栏」现状（新增为主，`bind_keys` 一处是行为修正）。

## Open Questions

- 菜单栏高度取值（28px vs 30px vs 主题相关）—— 不影响 spec 与任务拆分，
  headed 验收时目视定。
- 是否顺带在菜单加 Window/Help 等标准件 —— 不影响 spec（spec 只要求「至少
  一个菜单」与 Copy 行为），本次不做。
