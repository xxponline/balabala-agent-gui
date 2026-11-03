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
