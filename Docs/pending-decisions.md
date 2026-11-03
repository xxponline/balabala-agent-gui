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

**当前选择**: sidebar::render 返回一个 `SidebarAction` struct, app.rs 在 ui()
里用一长串 `if let ... = action.xxx` 匹配并调用相应方法。

**为什么这样选**: MVP 阶段直通, 不引入事件总线 / 命令模式。

**未来需要重新评估的信号**:
- action 种类继续膨胀 (现在已经 6 种), if 链可读性下降
- 想给某些 action 加全局快捷键 / 撤销 / 重做
- 想做命令面板 (Ctrl+Shift+P 那种)

**要做的事**:
- 把每个 action 抽象成一个 `Command` trait 对象
- 在 app 层维护一个 `CommandQueue` 或 `EventBus`
- sidebar 只负责发出事件, 不直接触发状态变更

## rfd 同步调用阻塞主线程

**当前选择**: 用 `rfd::FileDialog::pick_folder()` 同步调用, 直接在 egui 的
ui() 里阻塞等用户选完。

**为什么这样选**: MVP 阶段最简, 用户主动点的按钮, 阻塞期间用户本来就在
等对话框。

**未来需要重新评估的信号**:
- macOS 上某些版本会卡窗口 /  beach ball
- 想在对话框打开期间让后台 pty 继续渲染
- pick_folder() 被绑到非用户手势的代码路径上 (比如自动恢复)

**要做的事**:
- 切到 `rfd::AsyncFileDialog`
- 把 Future 句柄存在 app 状态里, 每帧 poll

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

**当前选择**: 两步操作 — 点 sidebar 切 selection, 再点 pane 区域才拿
键盘焦点 (IME 才激活)。

**为什么这样选**: MVP 阶段信任 egui 的 focus 系统, 不自己跟踪
"当前哪个 pane 有 focus"。

**未来需要重新评估的信号**:
- 用户抱怨 "切了 agent 还要再点一下才能打字", 体验差
- 想做 keyboard-only 导航 (Tab 切 agent)

**要做的事**:
- sidebar 点 agent 行时, 调 `ctx.memory_mut(|m| m.focused = Some(frame_id))`
- 或者用 `response.request_focus()` 的等价 API
- 跟 InteractiveFrame 的全局 ID 协调好
