# Headed 冒烟记录 — solution-overview (tasks 6.1–6.3)

无头层已覆盖等价行为（`solution_row_renders_above_its_project_with_namespaced_id` /
`solution_row_context_menu_dispatches_declared_actions` /
`solution_row_click_toggles_collapse_only`），但 tasks.md 指定的是 **headed**
冒烟 + 记录留存，故单独跑本记录。

**执行环境**：macOS Aqua 会话，仓库根目录。
**执行者**：用户手动（b.f.peng）　**日期**：2026-11-24

---

## 0. 夹具

临时工作区 `/tmp/balabala-smoke/ws`，两个 `simple` solution，已置于历史首位：

| solution path | 解析出的 project |
|---|---|
| `/tmp/balabala-smoke/ws/proj-alpha` | `proj-alpha` |
| `/tmp/balabala-smoke/ws/proj-beta` | `proj-beta` |

两个 solution 的 `register_to_asset_manager` 均为缺省（= enabled），所以资产树
初始应有**两个** project 根。

> 这个夹具刻意让 solution 行与 project 行**同名**（`proj-alpha · simple` vs
> `proj-alpha`）—— 正是 design D2 说的"v1 两者 path 相同、类型标记是唯一
> 视觉区分"那个场景。

历史备份：`workspace_history.json.bak`（同目录）。测完还原。

---

## 6.1 两行 solution 各自带类型标记、位于其 project 之上

1. `cargo run`
2. Launcher 列表第一行 = `/tmp/balabala-smoke/ws`，点它
3. 主窗打开后，左槽自上而下应为：

```
proj-alpha · simple      ← solution 行 (带 · simple 标记)
└─ proj-alpha            ← project 行 (无标记, 同名)
proj-beta · simple       ← solution 行
└─ proj-beta             ← project 行
```

4. **判据**：
   - 两行都含 `· simple`
   - 每个 solution 行**紧贴在**其 project 行上方
   - `proj-alpha · simple` 与 `proj-alpha` 是两行不同的行（不是重复渲染）

记录：＿＿＿＿＿

---

## 6.2 右键 → 服务声明的开关 → 交树收缩/扩张 → 写回文件

1. 仍在该工作区。右键 `proj-beta · simple` 行
2. **预期**：菜单出现两项 —— `注册到资产树`（勾选态 **✓**）、`git 管理`（勾选态 ✓）
   （label 来自服务的 `solution_actions()`，不是视图文案表）
3. 点 `注册到资产树` → 取消勾选
4. **预期**：
   - 菜单重开时该项变为**未勾选**
   - 右侧资产树**少掉** `proj-beta` 这个根（交树收缩 → 资产注册重枚举）
   - 左槽的 `proj-beta · simple` / `proj-beta` 行**仍在**（全量集保留）
5. 再点一次 → 资产树根**回来**
6. 校验写回：`cat /tmp/balabala-smoke/ws/balabala.solutions.json`
   **预期**：`proj-beta` 那项出现 `"register_to_asset_manager": false`（或恢复为 `true`）

记录：＿＿＿＿＿

---

## 6.3 左键折叠/展开 + 重启后恢复展开

1. 左键点 `proj-alpha · simple` 行
2. **预期**：其下 `proj-alpha` 行消失；**再点恢复**
3. 折叠状态下重启（关掉 → `cargo run` → 再选同一工作区）
4. **预期**：`proj-alpha · simple` **恢复为展开**（折叠态易失、不持久化）
5. 顺带确认左键**不**打开任何签、不切 active session、不切 tab

记录：＿＿＿＿＿

---

## 结果

| Task | 结果 | 备注 |
|---|---|---|
| 6.1 两行 solution + 类型标记 + 位置 | **☑ 通过** | 左槽依次 `proj-alpha · simple` → `proj-alpha` → `proj-beta · simple` → `proj-beta`, 两 solution 行带 `· simple` 且与 project 行是不同行 |
| 6.2 右键开关 → 交树收缩/扩张 + 写回 | **☑ 通过** | 菜单 label 来自服务; 切换后 `balabala.solutions.json` 被服务重写, 两个 item 出显式 flag (load 时 `serde(default)` 补真值 → 落盘两个都带, 属预期) |
| 6.3 折叠/展开 + 重启恢复 | **☑ 通过** | 左键折叠隐藏子行、再点恢复; 重启后展开 (易失不持久化); 左键不打开签 |

> 三条全通过 → tasks.md 6.1/6.2/6.3 已勾，可进入 archive。
> 夹具 `/tmp/balabala-smoke` 已删除，历史已还原（3 条有效条目），备份已清。
