# Pending Decisions

记录当前 demo 阶段先简化、但未来需要重新评估的设计权衡。

## 多 agent 的 IO 调度

**当前选择**: 每 entity 一个 reader 线程 + 无界 std mpsc channel。

**为什么这样选**: 简单, 与现有 demo 模式一致, N 个 agent 复制 N 份即可。

**未来需要重新评估的信号**:
- agent 数量到 30+ 时线程数是否成为问题
- pty 输出狂喷 (例如 `cat bigfile`) 时无界 channel 是否内存爆炸
- 是否值得引入 mio / epoll 统一 poll 所有 pty master fd

**备选方案**:
- B: 单 reader 线程用 mio / epoll 统一 poll 所有 pty (portable-pty 未直接暴露 raw fd, 复杂度高)
- C: crossbeam 有界 channel + 背压 / 丢老策略

## 终端尺寸 (resize) 同步

**当前选择**: 写死 100 cols x 30 rows, 不响应窗口大小变化。

**为什么这样选**: MVP 阶段优先打通多 agent + 渲染 + 输入链路,
resize 需要字体 metrics + SIGWINCH 通知, 是独立的工作块。

**未来需要重新评估的信号**:
- 用户想跑 `vim` / `htop` / `less` 这种全屏程序时, 屏幕会糊
- 窗口被拖大拖小后, 终端内容不跟着变

**要做的事**:
- 用 `ctx.fonts()` 算出 monospace 字体的 char width / line height
- 从 main pane 的 available_size 反推 rows / cols
- 调 `parser.set_size(rows, cols)` + `pty.resize(PtySize{..})`
- 注意: 只在尺寸变化时通知 pty, 别每帧都发 SIGWINCH

## Agent 关闭的优雅退出

**当前选择**: 直接 `child.kill()` (SIGKILL), 不做优雅退出。

**为什么这样选**: MVP 阶段简化, SIGKILL 立刻释放资源。

**未来需要重新评估的信号**:
- pi agent 正在写文件 / 跑长任务时被强杀, 可能丢数据
- 用户期望 Ctrl+D / exit 能让 agent 自己收尾

**要做的事**:
- 关闭时先向 pty 写 `\x04` (Ctrl+D) 或 `exit\r`
- 等 N 秒看子进程是否自己退出
- 没退出再 fallback 到 SIGKILL

## 键盘输入范围 (Ctrl 组合键)

**当前选择**: 只转发可打印字符 + Enter / Backspace / Tab / 方向键,
不处理 Ctrl+C / Ctrl+D / Ctrl+R 等带修饰键的控制字符。

**为什么这样选**: MVP 阶段最小链路, 修饰键判断增加代码复杂度。

**未来需要重新评估的信号**:
- pi agent 生成长回复时, 用户想 Ctrl+C 中断
- 用户想 Ctrl+R 翻历史 / Ctrl+D 退出当前 session

**要做的事**:
- 在 `Event::Key` 分支里检查 `ui.input(|i| i.modifiers.ctrl)`
- 把 Ctrl+字母 映射成对应的 ASCII 控制字符 (Ctrl+A=0x01 ... Ctrl+Z=0x1a)
- 注意区分 Cmd+C (复制) 和 Ctrl+C (中断), 取决于平台

## 统一 sidebar 事件处理器

**当前选择 (2026-11-06 更新, change `port-ui-to-gpui-kit`)**: 回调式分发 —
gpui 版 sidebar 渲染时经 `cx.listener` 给每个控件挂闭包, 点击经 entity
update 直接进入 `TerminalApp` 的方法 (design D7); `SidebarAction` struct
已退役, egui 时代的 `if let` 链随之消失。不引入事件总线 / 命令模式的
取舍不变 (理由同下)。

**未来需要重新评估的信号**:
- action 种类继续膨胀 (现在已经 6 种), if 链可读性下降
- 想给某些 action 加全局快捷键 / 撤销 / 重做
- 想做命令面板 (Ctrl+Shift+P 那种)

**要做的事**:
- 把每个 action 抽象成一个 `Command` trait 对象
- 在 app 层维护一个 `CommandQueue` 或 `EventBus`
- sidebar 只负责发出事件, 不直接触发状态变更

## rfd 文件夹选择 (原: 同步调用阻塞主线程)

**状态: 已解决** (2026-11-06, change `port-ui-to-gpui-kit`)。

- 同步版在 gpui 下会崩溃: 模态循环阻塞期间监听器仍持有 App 可变借用,
  50ms drain 循环的 `weak.update` 重入借用 → RefCell panic (实测)。
- 已切到 `rfd::AsyncFileDialog` + `cx.spawn`, Future 句柄存于 shell 的
  `_dialog_task` (design D8); 对话框打开期间主线程照常跑 drain 循环。
- 通则: gpui 事件监听器内禁止任何阻塞调用 (design D3)。
- 原记录的 "要做的事" (切 AsyncFileDialog + 存 Future 句柄) 已完成;
  gpui 原生 `prompt_for_paths` 仍是可选的后续替换 (D8)。

## 持久化路径选择 (项目本地 vs 用户目录)

**当前选择**: `.workspaces.json` 写在项目根目录下。

**为什么这样选**: 这是 demo / 开发期工具, 用户从哪启动 cargo run,
workspaces.json 就在哪, 心智最简单。

**未来需要重新评估的信号**:
- 把这个 app 发布给其他用户用 (他们不会 clone 源码)
- 用户期望在不同项目里启动同一个 app 都能看到自己的 workspace 列表

**要做的事**:
- 引入 `directories` crate 拿平台特定的配置目录
  (macOS: `~/Library/Application Support/pi-agent-shell/`)
- 写一个一次性迁移: 如果项目根有旧的 `.workspaces.json`, 复制到新位置
- 之后读新位置, 删项目根的旧文件

## IME preedit 在终端里的预览

**当前选择**: 只转发 `ImeEvent::Commit(s)` (上屏后的最终字符串),
不转发 `Preedit` (候选词预览) 和 `DeleteSurrounding`。

**为什么这样选**: MVP 阶段先让中文能进 pty, 预览是体验优化。

**未来需要重新评估的信号**:
- 用户抱怨打字时终端里看不到候选词预览, 体验跟原生终端差太多
- 想做光标位置感知 (DeleteSurrounding 要知道光标在哪)

**要做的事**:
- 在 vt100 网格底部 (或光标下方) 画一个临时预览行
- 监听 `Preedit { text, active_range_chars }` 事件, 实时更新预览
- Commit 时把预览行清掉, 把最终文本写到主网格
- DeleteSurrounding 要反查当前光标位置, 调 parser 删字符

## 一步切 pane focus (点 sidebar 直接拿焦点)

**状态: 已解决** (2026-11-06, change `port-ui-to-gpui-kit`)。

- 一步式模型落地: 点 sidebar agent 行 → 同时聚焦 `pane_focus:
  FocusHandle`, 键盘/IME 即刻生效, 无需再点面板。
- 聚焦态可视化: 终端面板框蓝 (聚焦) / 灰 (失焦)。
- specs delta 已同步 ("Sidebar selection grants keyboard and IME focus
  in one step")。
- 原两步方案 (egui `InteractiveFrame` + `memory.focused`) 已随 egui 删除。
- 仍待未来: keyboard-only 导航 — sidebar 尚未进 Tab 序, Tab 目前只在
  终端面板内被转发 (Terminal context 绑定)。

## Streaming 活动指示 (badge 视觉反馈)

**当前选择**: 不做 — 维持 spec 三态 (starting / running / exited)。

**背景 (2026-11-09, change `split-ui-modules-dockarea` 6.1 验收)**:
用户报告 "badge 在 streaming 期间没有实时变动"。诊断结论: **badge 无缺陷** —
- 创建后即绿: pi 首帧输出极快 (<100ms), 黄点 starting 一闪而过, 肉眼即"直接变绿";
- streaming 期间不变: running → running **本来就没有状态变化**, spec 无此态;
- quit 变红 (exited): 实时 ✓ (无头回归测试 `status_badge_refreshes_via_drain_loop`
  验证全链: drain → notify 转发 → 面板重渲)。

**未来需要重新评估的信号**:
- 用户明确想要 streaming 时的视觉反馈 (如"让绿点知道它在干活")
- 多 agent 并行时需要区分"哪个在流式输出"

**要做的事** (若立项):
- spec delta: agent-shell 的 status badge requirement 扩展一个活动态
  (或独立 capability), 定义视觉语义 (脉冲/闪烁/透明度呼吸)
- 实现点: 与 drain 循环联动 (有帧消费 -> 临时活动态 -> 超时回落 running)
- 注意: 活动态是**纯视觉**, 不改变三态语义与 quit 行为

**备选方案**:
- A: 绿点脉冲动画 (透明度呼吸, 有帧消费时启动, idle 停)
- B: 状态点旁小 spinner 字符 (网格内不可行, 只能 sidebar 侧)
- C: badge 数字/计数 (如本次会话 token 流量) — 依赖 pi 状态快照

## pi-shell-rs 不处理 Tab 键 (原生 TUI 功能在 embed 路径缺失)

**状态**: 待查 (2026-11-09, change `split-ui-modules-dockarea` 6.1 的 F3 诊断发现)。

**现象**: Tab 键经完整链路送达后**零反应** — 光标不动、整屏内容不变;
同一 `inject_key` 通道的 Left/Right/Backspace 均有反应。诊断复现:
`cargo test diagnostic_per_commit -- --ignored --nocapture` (key Tab 段)。

**应用侧已全部证实无责**:
- 真机: `[F3] TerminalTab action fired -> send_key(Tab)` ×3 (绑定截获路径通);
- CI: `sidebar_select_grants_one_step_focus` 断言3 (`tab_forwards` 计数) 守卫
  spec "Tab SHALL be delivered" 转发边界;
- 通道: 同 `embed.inject_key` 的其他键全部正常送达并生效。

**关键线索 (用户)**: **原生 TUI 的 Tab 是有对应功能的** → 问题大概率在
pi-shell-rs 的 key 事件封装/注入层, 而非 pi 本身。

**未来需要重新评估的信号**:
- 修好后: embed 路径 Tab 行为应与原生 TUI 一致 (可跑 diagnostic 对比)

**要做的事** (排查方向, 按嫌疑排序):
- `event.rs` 的 `key_intent` 分类 (event.rs:200-206): `UiEvent::Key(Tab)` 是否
  被分类成被丢弃/降级的 intent (已有 `KeyIntent::SendFollowUp` 等分类器)
- embed 事件循环消费 `UiEvent::Key` 的路径 (embed.rs:140 入队后): 注入 TUI
  时是否过滤了 Tab / 非文本键
- 对照原生模式 (crossterm 直通) 与 embed 模式的按键处理差异

**备选方案**:
- A: pi-shell-rs 修复 Tab 注入 (首选, 保持与原生一致)
- B: 若 embed 场景确不适用 Tab → 文档化已知限制, spec 交付边界不变
  (应用侧已送达, 属 pi-shell 行为)

## 文件树过滤的配置界面 (含 agent-shell 键盘条款修订)

**状态**: 待立项 (2026-11-09, change `right-dock-file-tree`)。

**当前选择**: 过滤规则纯配置驱动 — `.workspaces.json` 新开 `file_tree`
区段 (按 workspace 路径键控, gitignore 式模式行数组), v1 右 dock **无任何
可聚焦文本输入**, ⟳ 刷新时重读生效。

**为什么这样选**: `agent-shell` 两条键盘契约是绝对条款 ("仅 macOS 应用失焦
才不转发键盘/IME"), dock 内一旦有输入控件, 打字即违反 spec。配置文件绕开了
这个问题, 且配置语义随数据源走 (i 定案)。

**未来需要重新评估的信号**:
- 用户想要即输即筛的体验 (改配置文件 + 点刷新太绕)

**要做的事** (配置界面立项时):
- spec: `agent-shell` 的 Session selection focus model 与 Keyboard input
  forwarding 两条 MUST 开一个明确例外 — "明确获得焦点的 dock 交互控件
  (当前即右 dock 过滤框) 可接收键盘/IME, 直到其失焦"; 补对应场景
- 面板侧: 过滤输入框持焦期间, 树行点击仍即时交回终端 (纪律不变)

## 文件树的自动刷新 (焦点重扫 / FS 监听)

**状态**: 待立项 (2026-11-09, change `right-dock-file-tree` 5.x)。

**当前选择**: 三个低频触发 — 首次加载 (根路径出现) / 切换活动 workspace
(根路径变化) / 手动 ⟳ 按钮; 同 workspace 切 agent 不重扫。

**未来需要重新评估的信号**:
- 用户频繁外部建文件后抱怨"要点刷新才能看到"
- 编辑器内改文件, 右树长期陈旧

**要做的事** (按成本排序):
- b: 窗口重获焦点时自动重扫 (零依赖, `did_become_key` 订阅 + 防抖)
- c: notify crate 实时监听 — 需防抖、目录移动事件归并、大树 diff
- 注意: 重扫 = 全量重建权威树, 展开态由面板集合贴回 (design D4) 不丢

## 文件树的惰性扫描 (大仓)

**状态**: 待立项 (2026-11-09, change `right-dock-file-tree` design D6)。

**当前选择**: 全量递归 + 排除表 (`.git node_modules target build dist
.DS_Store`) + 后台线程。

**未来需要重新评估的信号**:
- 排除表挡不住的超大仓 (数据目录 / 未排除的生成物) 刷新明显卡顿
- 用户要求浏览被排除目录 (如按需展开 node_modules)

**要做的事**:
- 首展才扫该目录 (惰性) — 需解决两件事: `is_folder()` 对未扫目录为 false
  (空 children = 假文件, 面板 dir_ids 集合已兜图标, 但语义要补)、展开事件
  驱动的异步插入 + 树重建 (展开态靠面板集合保留)
- 或: 扫描深度上限 + 超限目录显示"展开以加载"

## 右键 "在 Finder 显示" 的跨平台

**状态**: 待立项 (2026-11-09, change `right-dock-file-tree` 4.2)。

**当前选择**: macOS only — `open -R <path>` 子进程 (spec 场景即 Finder);
`cfg(not(target_os = "macos"))` 分支为空操作。

**未来需要重新评估的信号**:
- app 要发到 Windows / Linux 用户手里

**要做的事**:
- Windows: `explorer /select,"<path>"`
- Linux: 桌面环境差异大 (nautilus -select 等), 需定义降级 (如打开父目录)
- 落地点: `reveal_in_finder` 的 cfg 分支 (src/tree/mod.rs)

