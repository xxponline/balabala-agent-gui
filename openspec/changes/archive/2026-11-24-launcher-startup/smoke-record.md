# Headed 冒烟记录 — launcher-startup (tasks 7.1–7.3)

无头层已覆盖等价行为（`valid_entry_hands_off_to_main_window` /
`closing_the_launcher_interrupts_startup` / `dead_history_entry_stays_in_launcher`），
但 tasks.md 指定的是 **headed** 冒烟 + 记录留存，故单独跑本记录。

**执行环境**：macOS Aqua 会话，仓库根目录。
**执行者**：用户手动（b.f.peng）　**日期**：2026-11-23

> 执行期间的两条旁证（非判据，但值得留档）：
>
> 1. **7.2 那次日志里零条 `[term-rate]`**，7.1/7.3 则有 —— 该打印由
>    `rate_log::maybe_print()` 发出，只被 `note_render`/`note_feed` 触发，
>    而二者仅在 `AgentTerminal`（主窗 console pane）上发生。这从侧面
>    证实 **Launcher 阶段零其他逻辑**（spec: No other subsystem starts
>    during the Launcher）。
> 2. **7.1 跑完后历史列表头从 `test-solution` 变成 `balabala`** ——
>    `commit_open` 的 `history.push` 真的执行了，说明 `OpenOutcome::Ready`
>    分支端到端跑通，不只是窗口切换。

---

## 0. 准备

```bash
cd /Users/b.f.peng/Documents/Projects/balabala-agent-gui
cargo build
```

历史文件（`directories` 的 `ProjectDirs::from("com.balabala","",…)` 在 macOS
上把 qualifier 也拼进路径）：

```bash
H="$HOME/Library/Application Support/com.balabala.balabala-agent-gui/workspace_history.json"
```

当前条目（执行时若有变化以实际为准）：

| # | 路径 | 状态 |
|---|---|---|
| 1 | `/Users/b.f.peng/Documents/Projects/test-solution` | 有效 |
| 2 | `/Users/b.f.peng/Documents/Projects/balabala` | 有效 |
| 3 | `/Users/b.f.peng/Documents/Projects/balabala-agent-gui` | 有效 |

---

## 7.1 启动出 Launcher → 选历史条目 → 主窗打开且 workspace 就位

1. `cargo run`
2. **预期 A**：只出现一个 **480×560、标题 `balabala`** 的小窗；
   标题下依次是 `balabala` 字样、历史条目列表（绝对路径，最近在上）、
   `open workspace…`、`new workspace…` 两个按钮。
   **此时不应存在** 1000×600 标题 `pi agent shell` 的主窗。
3. 点任意一个**有效**历史条目（如 `…/balabala-agent-gui`）。
4. **预期 B**：小窗消失 → 主窗 `pi agent shell` 出现；
   左槽出现 project 行（说明 solutions 已加载、workspace 就位）。
5. 记录：＿＿＿＿＿

**通过判据**：A 与 B 都成立，且小窗关闭后进程**没有**退出。

---

## 7.2 启动出 Launcher → 关闭 → 进程退出、无残留窗口

1. 确认没有残留：`pkill -f balabala-agent-gui`（无输出即干净）
2. `cargo run &`
3. 等 Launcher 出现（约 2–3 秒）
4. `pgrep -fl balabala-agent-gui` → **应有输出**（进程在）
5. 点 Launcher 左上角红色关闭按钮（⌘W 亦可）
6. 等 2 秒，`pgrep -fl balabala-agent-gui` → **应无输出**
7. 记录：＿＿＿＿＿

**通过判据**：关窗后进程退出、无残留窗口。
（这验的是 `QuitMode::LastWindowClosed` —— Launcher 是唯一窗口，
关掉即最后一个。）

---

## 7.3 启动出 Launcher → 点死条目 → 就地报错、留在 Launcher、进程未退

历史里目前**没有**死条目，先临时注入一条（测完还原）：

```bash
# 备份
cp "$H" "$H.bak"

# 注入一条不存在的路径到列表头
python3 - "$H" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
dead = "/tmp/balabala-dead-entry-does-not-exist"
if dead not in d["recent"]:
    d["recent"].insert(0, dead)
json.dump(d, open(p, "w"), indent=2, ensure_ascii=False)
print("已注入:", dead)
PY
```

1. `cargo run &`
2. **预期 A**：Launcher 列表**第一行**是 `/tmp/balabala-dead-entry-does-not-exist`
3. 点这一行
4. **预期 B**：
   - 标题下方出现**红色**错误文字，含 `does not exist`
   - **Launcher 窗口仍在**（不关闭）
   - 进程仍在（`pgrep -fl balabala-agent-gui` 有输出）
   - **没有**出现 `pi agent shell` 主窗
5. 点一个有效条目 → 主窗正常打开（证明错误不锁死 Launcher）
6. 还原历史：

```bash
mv "$H.bak" "$H"
```

7. 记录：＿＿＿＿＿

**通过判据**：B 的四项全成立，且第 5 步能恢复。

---

## 结果

| Task | 结果 | 备注 |
|---|---|---|
| 7.1 启动 → 选历史 → 主窗就位 | **☑ 通过** | 只出 480×560 `balabala` 小窗无主窗；点 `…/balabala` 后小窗消失、主窗出现、左槽有 project 行、进程未退；历史 push 已验证 |
| 7.2 关 Launcher → 进程退出 | **☑ 通过** | 关窗后 `pgrep` 无输出，无残留窗口；日志零 `[term-rate]` |
| 7.3 死条目就地报错、留在 Launcher | **☑ 通过** | 注入 `/tmp/balabala-dead-entry-does-not-exist` 于列表头 → 点击后红字 `does not exist`、Launcher 仍在、进程仍在、无主窗；再点有效条目正常进主窗 |

> 7.3 的死条目已按步骤 5 还原（历史回到 3 条全有效、备份已删）。

三条全通过 → tasks.md 7.1/7.2/7.3 已勾，可进入 archive。
