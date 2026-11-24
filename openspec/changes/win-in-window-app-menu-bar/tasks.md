# Tasks

## 1. 布局骨架：菜单栏容器与 dock 让位

- [x] 1.1 在 `TerminalApp::render` 根容器加两个 wrapper —— 菜单栏位
      `h(px(28)).w_full().flex_shrink_0()`、dock 位 `flex_1().min_h(px(0))`
      包住现有 `dock_area`，验证：`cargo check --all-targets` 通过，且
      `cargo test` 失败集合与既有基线 diff 为空（零新增失败；基线 19 个
      既有失败的三类根因见 design Risks，其中 `adaptive_grid_dispatch`
      属 A 类本机不可执行）
- [x] 1.2 用 `gpui_kit::test` 无头窗口断言 dock 仍占满 wrapper 内剩余空间
      （菜单栏 wrapper 存在时 `terminal-grid` bounds 高度 < 窗口高度且
      > `MIN_GRID_ROWS * line_h`），验证：新增测试通过，且失败集合与基线
      diff 为空（原有网格 bounds 测试不因本改动新增失败）

## 2. 窗口内菜单栏挂载（spec: Application menu bar is always present）

- [x] 2.1 `menu::install` 内**先** `GlobalState::set_app_menus(build().into_iter().map(|m| m.owned()).collect())`
      **再**建 `AppMenuBar` 实例并存入 `TerminalApp` 字段（D1 顺序），保留
      现有 `cx.set_menus(build())`，验证：`cargo test menu` 现有 4 个
      `get_menus()` 测试仍全绿
- [x] 2.2 把 `AppMenuBar` 实体挂进 1.1 的菜单栏 wrapper（D6：macOS 不
      挂第二条）。**机制偏离任务原文**：未在挂载处加 cfg，改为在
      `menu.rs::install` 创建处门控（非 mac 建实例 / macOS 返回 `None`），
      挂载处用 `.children(self.menu_bar.clone())` —— 平台决策单点、cfg 由
      5 处降为 2 处，行为等价；理由与已知代价见 design Risks。验证：
      `cargo build --target x86_64-pc-windows-msvc` 与 `cargo build` 均通过；
      **macOS 侧编译本机不可验**（`onig_sys` build script 缺 `cc`，且需
      Apple SDK），以结构性论证替代 —— 字段与挂载两平台源码同构无 cfg，
      「缺失」构造上不可能发生；完整验证归 task 6.3
- [x] 2.3 新增渲染级断言（`cfg(not(target_os = "macos"))`）：帧内 `Edit`
      菜单在场（debug 构建另含 `Debug`、release 反向断言缺席），且
      `Edit` 落在 `dock-slot` 之上。**取法偏离任务原文**：改用
      `find_by_label`（a11y label）而非 `texts()` —— Button 的 label 走
      a11y 路径，`texts()` 只收 `ObservedText` 绘的字串，实测 `Edit` 不在
      其中；与 `assets/panel_tests` 的 `try_find_by_label` 同款取法。
      验证：该测试在 Windows 通过、在 macOS 被 cfg 跳过；对应 spec 场景
      *Windowed menu bar is visible above the dock on Windows*
- [x] 2.4 headed 验收：Windows 真机跑 debug 构建，确认菜单栏可见可点、
      `Edit` 菜单能开、dock 未被挤出窗口（spec 场景 *Windowed menu bar
      leaves the dock its full remaining space*）。
      **验收记录（2026-11-24, Windows 10/11 真机, debug 构建
      `target/debug/balabala-agent-gui.exe`）**：用户目视确认 —— 菜单栏在
      窗口顶部、含 `Edit` 与 `Debug`、`Edit` 可点开（含 `Copy` 项）、左中右
      三槽完整未被挤出窗口；进程正常渲染（`render/s` 持续 >0），关窗干净
      退出，日志无 panic/error。菜单栏高度 28px 初值目视可接受（design D5
      Open Question 就此收口）。

## 3. Copy 灰显随选区（spec: Copy item enabled state follows selection changes）

- [x] 3.1 抽出统一重建函数（D3：`set_app_menus` + `menu_bar.reload()` 两步），
      验证：单元测试直接调用两次，断言 `GlobalState::app_menus()` 内容更新且
      第二次调用不改变已有项的顺序
- [x] 3.2 `TerminalApp` 缓存 `copy_enabled: bool`，在 `TerminalApp::render` 内
      读 `terminal.selection.is_some()` 比对，**翻转才**调 3.1 的重建函数，
      验证：新增无头测试 —— 无选区开菜单 Copy 灰显、注入选区后重渲染再开
      菜单 Copy 可用、清选区后再次灰显，且全程 `Edit`/`Debug` 项顺序不变。
      **观测口径偏离任务原文**：灰显值读 `GlobalState` 的 `disabled` 源
      （`with_menu_items` 逐字拷进 popup，即渲染输入）而非 popup 项的
      `.disabled()` —— 后者恒 `None`，因 `MenuItemElement::render` 不把
      `disabled` 写进 a11y node（gpui-component 观测盲区，非本 change 缺陷）。
      「菜单真的开了 + Copy 项在场」仍由 popup a11y 断言（`find_by_label`，
      未开会 panic）。**顺序断言改前缀**（Debug 注册表进程全局，并行测试
      会 append）—— 详见 design Risks 末两条。
- [x] 3.3 验证 notify 向上标脏确实使 `TerminalApp::render` 在选区变化后重跑
      （design Risks 末条），验证：新增测试在选区变化后**不调 `render()`、
      不开菜单**（`click_target` 内部自带 `render_frame`，一点菜单就掩盖被测
      机制），仅 `run_until_parked`，断言 `copy_disabled` 已翻转 —— **通过，
      未触发 fallback**，无需在选区变化处补 `TerminalApp` 显式 `notify`。
      `run_until_parked` 只跑 dispatcher 不绘制，画面推进只能来自 notify 标脏
      后的重渲，故该断言确实证明祖先标脏链有效。
- [x] 3.4 焦点不参与判定的负向断言（spec 场景 *Focus outside the console does
      not disable Copy*）：有选区、焦点移至侧栏后开菜单，Copy 仍可用，
      验证：新增测试通过。**含反向对照** —— 焦点移出后
      `window.is_action_available(&TerminalCopy)` 必须报 `false`（Copy 监听
      挂在 terminal pane 上，焦点不在其派发路径内）；两条一起断言才区分
      「焦点真的不参与」与「断言空转」。代价已由 design D2 接受并写入。

## 4. 快捷键按平台解析（spec: Copy selection with Cmd+C）

- [x] 4.1 `bind_keys` 中 `cmd-c` 改 `KeyBinding::load("secondary-c", ...)`
      （`new` 遇 `secondary` 会 panic，必须用 `load`），验证：
      `cargo build` 通过且 `cargo test` 中复制相关测试全绿。
      **抽出 `platform_binding(&App, keys, action, context)`** 统一承载
      `load` 调用（建参数需 `&App`、`bind_keys` 需 `&mut App`，故先在块内
      建数组再 bind，避开同一次调用的 `&cx`/`&mut cx` 重叠）。
      **测试侧配套改动**（4.1 的直接后果，非新范围）：
      `property_detail_manager` 三处 `press("cmd-c"/"cmd-shift-m")` 改为
      `press("secondary-c"/"secondary-shift-m")` —— `press` 内部走
      `parse_with_os`，与绑定同源；否则 Windows 上 `cmd-c` 解析成 Win 键
      与新绑定不匹配（改后这两个测试从绿 → 失败，已复绿）。
- [x] 4.2 `cmd-shift-m`（ToggleMdView）同改 `secondary-shift-m`，验证：
      md 查看切换的既有测试通过（`md_toggle_action_dispatches_in_place` ok，
      同走 4.1 的 `platform_binding`，无单独实现）。
- [x] 4.3 断言键位按平台解析：无头测试读 keymap 中 `TerminalCopy` 的绑定，
      macOS 断言 `platform` 修饰键、Windows 断言 `control` 修饰键，验证：
      `cargo build` + `cargo build --target x86_64-pc-windows-msvc` 均通过，
      测试在本机通过（对应 spec 场景 *Shortcut resolves to Ctrl+C on Windows* /
      *Shortcut resolves to Cmd+C on macOS*）。
      **验证方式偏离任务原文**：未跑两个 target 各一遍 —— 本机 host 即
      `x86_64-pc-windows-msvc`（两遍实为同一）；macOS target 无法交叉执行
      test（task 2.2 同款 Apple SDK 限制）。改用**单测试 + `cfg!` 分支**
      （编译期定值）覆盖两平台，语义等价且免重复。另：读 `inner().modifiers`
      而非 `modifiers()` —— 后者 Windows 上返回 **display** modifiers，与参与
      匹配的 GPUI 表示可能差 shift。
- [x] 4.4 真机按键确认：Windows 上 Ctrl+C 复制生效、Win+C 不复制，
      验证：记录实测结果；若 Win+C 竟能触发，回头复核 4.1 的修饰键判定。
      **实测记录（2026-11-24, Windows 10/11, debug 构建, sentinel 前后夹测）**：
      - Ctrl+C 复制生效 ✓ — sentinel `W44-BEFORE` → 变为用户实际选中的
        文本（` thinking:high`），真机链路 OS→gpui→keymap→`TerminalCopy`→剪贴板走通；
      - Win+C 不复制 ✓ — 用户确认已按下，sentinel `W44-BEFORE-WINC` 完好、
        应用 PID 存活、日志无 panic → 无任何剪贴板写入；
      - 条件分支「若 Win+C 竟能触发则复核 4.1」**未触发**，无需复核。
      **归因（系统热键 vs modifiers 不匹配）未二分** —— 两者可观察结果相同，
      本轮未记录是否弹出 Copilot/Widgets 面板；但对正确性等价且键位判定
      由 4.3 构造性保证，故不阻断。并入 6.2 观察。
      📌 **方法论教训**：sentinel 需配「操作者确认已执行」—— 首轮就是把
      「无写入」当成了「按了被拒」，实际是没按；已写入 design Risks。

## 5. 重建通路接注册表（spec: Menu content rebuild on change）

- [x] 5.1 `debug::register()` 之后（装配期与测试侧）走 3.1 的同一重建函数，
      验证：新增测试 —— 启动后注册新 section，重渲染后 `Debug` 菜单列出该项，
      且 `Edit` 等既有项顺序不变（spec 场景 *Registered section appears in the
      menu* / *Rebuild preserves unchanged items*）。**补的是 3.1 未覆盖的
      后半段链路**：3.1 断到 `GlobalState` 为止，本测试一路打到 popup 渲染
      （`GlobalState` → `reload` → `AppMenu` 快照 → `with_menu_items` → a11y）。
      **含负向段**：`AppMenu.menu` 是 `reload()` 时刻的快照，故注册后未
      rebuild 时 popup 必不含新项 —— 证明 rebuild 必需而非碰巧；rebuild 后
      `reload()` 关掉开着的 popup（design D3 记录的行为），重开即见新项。
      **helper 泛化**：`open_edit_menu` 抽出 `open_menu_at(cx, handle, ix)`
      供复用（scope 靠被观测 Button 路径祖先解析，非 AppMenu 根 div 自身）。
- [x] 5.2 确认装配期顺序不回归：`register_builtin()` 仍在 `menu::install`
      之前，验证：`cargo test` 失败集合与基线 diff 为空，debug 构建 `Debug`
      菜单含 term-rate 与 threads/pipeline 两项。
      **实测**：装配顺序 `register_builtin`(@292) → `menu::install`(@298)
      未因本 change 移位；`builtin_sections_listed_and_open_in_turn`（走
      `open_app` 真实装配链）通过；全量 diff 与基线 IDENTICAL。

## 6. 集成验收

- [x] 6.1 全量回归：`cargo test`（debug）与 `cargo build --release`，验证：
      两者均通过，且 `cargo test` 失败集合与基线 diff 为空（基线数字见下
      实测段与 design Risks —— 集成 `92cdc9e` 后为 22 个既有失败）；
      release 二进制菜单无 `Debug` 项（既有字符串
      验收不变）。**实测（2026-11-24）**：与并行 change `92cdc9e` merge 后
      基线已变（19 → **22**，远程重构改测试名，旧基线不可 diff），改用
      **独立 worktree 取 `origin/master` 真基线**对照：debug
      `124 passed / 22 failed` → `132 / 22`（+8 = 本 change 新测试）、
      release `109 / 22` → `115 / 22`（+6，另 2 个 debug-only），两 profile
      失败集均 **IDENTICAL**；`cargo build --release` 通过；release 字符串
      验收 `release_menu_has_no_debug_and_keeps_edit_copy` 通过；零 warning。
      📌 **踩到的坑（已解决）**：headed 应用若仍在运行，会占用
      `target/debug/balabala-agent-gui.exe` → cargo 报 `failed to remove file`
      (os error 5) 且 **`cargo test` 静默不跑测试**（exit=101、无
      `test result` 行）。判据：**看 exit code 与 `test result` 行数，别只看
      diff** —— 空失败集可能意味着根本没跑。跑前先 `taskkill` 应用。
      🔧 **release 侧新测试已修**：`copy_disabled_follows_selection` 原硬断言
      `["Edit","Debug"]` 在 release 失败（design D5: Debug 菜单整块
      `cfg(debug_assertions)`）；改为**按 build profile 给期望序列**，
      而非 cfg 掉整个测试 —— 灰显属 design D6「Edit/Copy 常驻」，release
      也必须覆盖。
- [x] 6.2 Windows 真机整体验收：菜单栏可见、Copy 灰显随选区翻转、Ctrl+C
      复制生效、debug 菜单开签正常，验证：逐条对照 spec 场景记录结论，
      并把 2.4 / 4.4 的观察合并成一份验收记录。
      **验收记录（2026-11-24, Windows 10/11, debug 构建 PID 15616，
      观察者 = 用户真机目视，我方整理）**：
      - **A** 无选区 → Edit → Copy 灰 ✓ (*Copy disabled without a selection*)
      - **B** 拖选 → Edit → Copy 亮 ✓ (*Copy enabled with an active selection*)
      - **C** 清选区 → Edit → Copy 又灰 ✓ (*Copy item enabled state follows
        selection changes*) —— 3.2 的翻转链路首次真机确认
      - **D** Debug → term-rate → 中槽开签 ✓ (*Debug section tabs open through
        the section entry point*)，console 仍为第一签
      - 复核项：菜单栏在顶含 Edit/Debug（2.4）、Ctrl+C 复制生效（4.4）、
        dock 三槽完整未被挤出（2.4）✓
      - 运行期日志无 panic/error。
      **记录口径说明**：观察者是用户（我无法目视 GUI），我方负责逐项对照
      spec 场景与整理；非我方独立复现。
- [ ] 6.3 macOS 侧确认不回归：原生菜单栏仍出现、Cmd+C 复制不变、窗口内
      菜单栏未被挂载（无双份菜单），验证：真机目视 + `cargo test` 失败集合
      与基线 diff 为空。
      **状态：待 macOS 真机补做，change 暂不归档**（用户选择：稍后补）。
      本机为 Windows，**无法执行**——与 task 2.2 的 macOS 交叉编译限制同源
      （无 Apple SDK）。风险评估：本 change **未改动 macOS 路径的行为** —
      `menu::install` 的 macOS 分支只多了 `feed_global`（GlobalState 在
      macOS 无消费者，因 `menu_bar` 恒 `None` 不挂载）、返回值仍为 `None`；
      `set_menus` 调用原样保留；`secondary-c` 在 macOS 解析为 `platform`
      （⌘）与原 `cmd-c` **等价**。故回归风险低，但**无编译/运行证据**，
      属结构性论证（同 2.2 处理）。

## Workflow follow-up

- **归档前置条件：task 6.3 尚未完成**（待 macOS 真机补做）—— 本轮**不归档**。
  6.3 完成后：`openspec archive`，确认归档后主 spec 含本次三处
  `app-menu-bar` delta 与一处 `agent-shell` delta。
- 6.3 执行时的检查点：macOS 原生菜单栏仍出现、Cmd+C 复制不变、
  窗口内菜单栏**未**被挂载（无双份菜单）、`cargo test` 失败集合与
  macOS 侧基线 diff 为空。
- 附带可选观察（不阻塞归档）：Win+C 在 Win11 是否弹 Copilot/Widgets
  面板 —— design Risks 已记为外部行为、不影响正确性，与本 change
  无关。
