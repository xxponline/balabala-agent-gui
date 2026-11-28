# Tasks

## 1. 偏好存储 (src/theme_preference.rs)

- [x] 1.1 `ThemePreference {System,Light,Dark}` 枚举 + `resolve(WindowAppearance)
      -> ThemeMode` + 版本化 JSON 存取 (`load_at`/`save_at`/`default_path`,
      对齐 `workspace/history.rs` 形态)；测试：round-trip 三态各一次、
      文件缺失/损坏/版本不符/未知值 → 皆回落 `System`
      验证：`cargo test theme_preference` 全绿

## 2. 启动 apply + 订阅门控

- [x] 2.1 `apply_theme_preference(pref, window_opt, cx)` 助手（pinned →
      `Theme::change`，System → `sync_system_appearance`）；`main.rs` 在
      `gpui_kit::init` 后、`launcher::open` 前读默认路径并 apply（顺带修
      白色 Launcher 缺口）；测试：注入 pinned `dark` 文件启动 →
      `Theme::global(cx).mode == Dark`（测试平台外观是 Light，钉住态
      可观测）；无文件启动 → 跟随外观（既有 `theme_follows_window_appearance_
      on_startup` 不改动继续通过）
      验证：上述新测试 + 既有测试双绿
- [x] 2.2 `TerminalApp` 增偏好字段（路径可注入，`history_path` 同款
      纪律）；`observe_window_appearance` handler 门控：System 才
      sync + `push_tui_theme`，pinned 忽略；`TerminalApp::new` 内按偏好
      apply（真窗口下 System 用 `window.appearance()`）；测试：pinned 态
      走外观变化回调后 mode 不变，System 态解析为外观值
      验证：门控测试绿

## 3. 菜单 View > Theme

- [x] 3.1 `SetThemeMode` action（derive `Action`）；`menu::build` 追加
      `View > Theme` 三项（`checked` = 当前偏好恰一项），参数贯穿
      `feed_global`/`install`/`rebuild_menus`；`build_launcher` 不动；
      测试：build 产出含 View 子菜单且恰一项勾选（分别钉三偏好各断言
      一次）、Launcher 菜单无 View 项
      验证：菜单测试绿
- [x] 3.2 切换 handler（`app.on_action` + `cx.defer`，Debug 菜单同款
      防 update-lease 坑）：更新字段 → apply → 落盘（失败仅日志）→
      `push_tui_theme`（活会话重注入，spec req5）→ `rebuild_menus`（刷
      勾选）；测试：切换后文件内容、`Theme::global(cx).mode`、push 消息
      （既有 push 断言形态）、下次 build 勾选位落新项四项齐断
      验证：切换测试绿

## 4. 集成与真机验证

- [x] 4.1 全量 `cargo test --workspace` 与基线 diff 为空（当前 macOS
      基线 0 failed）
      验证：命令输出全绿
- [x] 4.2 macOS 真机冒烟：原生菜单 View > Theme 勾选位渲染（源码已证
      `setState`，真机目视）、切换即时生效含活会话配色、重启后偏好
      恢复、**启动即见存储偏好**（白色 Launcher 缺口已修）、钉住态下
      翻转系统外观应用不动；验证：用户目视 + 记录于本 tasks
      **验收记录（2026-11-25, macOS 27.2.1 arm64, 用户真机目视）**：
      用户反馈“功能上没有问题”（清单 A–E 全过：勾选位渲染/切换即时
      生效含活会话/钉住持守/回跟随吸附/重启恢复/首帧深色 Launcher/
      不回归，日志仅常态 term-rate 无 panic）；目视中发现**既有**
      遗留问题：solution-overview hover 提示字体恒黑、dark 模式不可见
      → 根因 `TextTooltip` 缺 text_color（浮层不吃应用根默认色），
      记入组 5 本 change 内修复
- [x] 4.3 Windows 真机冒烟：窗口内菜单栏勾选位渲染（源码已证
      check icon 映射）、切换/恢复/门控行为同 macOS；验证：用户目视 +
      记录于本 tasks
      **验收记录（2026-11-28, Windows, 用户真机目视）**：清单 A–F
      全过：View > Theme 随主窗出现且恰一勾（Launcher 无 View）、切
      换即时生效含活会话配色且下次打开勾跳新项、重启恢复且启动即见存
      储偏好（首帧 Launcher 色正确）、System 态翻转系统外观吸附、pin
      态持守（设置 → 个性化 → 颜色）、无关重建后勾选保持、原生标题栏
      跟系统外观（DWM 设计行为，观察项非回归）；日志仅常态 term-rate
      无 panic

## 5. 冒烟发现的既有 theming 缺口

- [x] 5.1 `TextTooltip` 补 `.text_color(cx.theme().foreground)`：tooltip 走
      独立浮层、不吃应用根（`gpui-component root.rs`）的默认文字色 →
      恒黑，dark 模式深底黑字不可见；违反现有 theming req2（chrome
      surfaces theme-consistent / “legible in only one theme”），spec 无需
      delta；gpui 自家 Tooltip 带 `popover_foreground` 是先例，全仓唯一
      自绘 tooltip，solution-overview 三行共用一处。验证：
      `cargo test --workspace` 全绿 + 用户暗色模式目视复查 tooltip 可读
      **验收记录（2026-11-25, macOS, 用户目视）**：暗色下 hover
      solution-overview 行，提示文字浅色可读 ✓（“Mac 已经正常”）。
      全量测试：修复后 7 轮全量跑 6 绿 1 偶发（首跑 2 个帧泵时序测试
      `tree_adapter_roundtrip`/`right_click_shows_asset_menu` 并行负载
      挤压，单跑均过、断言逻辑无关、非本 change 回归，已判非阻塞）

## Workflow follow-up

- 归档：`openspec archive "add-theme-preference" --yes`（会把 MODIFIED
  theming req1 合入主 spec、ADDED app-menu-bar View 菜单要求追加），
  归档前先确认 4.2/4.3 真机记录已写入。
- 归档后 `openspec validate` 复核主 spec。
