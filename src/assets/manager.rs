//! `AssetManager` — 右槽数据的服务层 (design D1/D5/D6)。
//!
//! 持有注册集 (solution 注册的 Project) 与三态 [`AssetState`] + 代数;
//! 枚举链在一次 spawn 内跑完 (Project → Module → 文件), **原子 land**。
//! 通知纪律: 本实体不直接 notify — 注册由调用方 (app 钩子) 收尾转发,
//! 异步落地经 `app.notify_panels` (内部 defer, 不撞面板 lease)。
//!
//! 守卫 (spec: O(1) / In-flight): 数据变更通知不进入本实体 —
//! 面板只做代数相等比对; 在途任务只被 refresh/注册替换 (supersede)。

#[cfg(test)]
use std::cell::Cell;
use std::path::PathBuf;
#[cfg(test)]
use std::rc::Rc;

use gpui_kit::*;

use crate::app::TerminalApp;
use crate::assets::state::{AssetEffect, AssetKind, AssetMenu, find_kind};
use crate::property_detail_manager::OpenObject;

use crate::assets::state::AssetState;
use super::traits::{Enumerator, EnumeratorOwner};

/// 右键功能 id (spec: Right-click context menu)。
pub const FUNC_COPY_PATH: &str = "copy-path";
pub const FUNC_REVEAL: &str = "reveal";

pub struct AssetManager {
    /// 通知枢纽 (落地后经 app.notify_panels 转发)。
    app: WeakEntity<TerminalApp>,
    /// 工作区配置目录 — 规则读取点 (spec: re-read on refresh);
    /// None = 从未注册。
    ws_dir: Option<PathBuf>,
    /// 注册集 (保持 solution 序; 身份 = `owner.root()`) —
    /// 资产侧只见 trait, 不见 Project (design D3: Level 2)。
    owners: Vec<Box<dyn EnumeratorOwner + Send>>,
    /// 三态视图 (design D5)。
    view: AssetState,
    /// 视图代数: 每次失效 (注册/刷新入口) 与每次落地各 +1 —
    /// 面板以相等比对做 O(1) 失效检测 (spec: O(1) when nothing changed)。
    generation: u64,
    /// 在途枚举任务: drop = 取消 (refresh/换注册集即 supersede)。
    in_flight: Option<Task<()>>,
    /// 枚举启动计数 (测试缝: 在途不重启 / 空注册不跑枚举)。
    #[cfg(test)]
    enumerate_starts: Rc<Cell<u64>>,
    /// 视图拉取计数 (测试缝: O(1) 通知守卫)。
    #[cfg(test)]
    view_pulls: Rc<Cell<u64>>,
}

impl AssetManager {
    pub fn new(app: WeakEntity<TerminalApp>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
            ws_dir: None,
            owners: Vec::new(),
            view: AssetState::Empty,
            generation: 0,
            in_flight: None,
            #[cfg(test)]
            enumerate_starts: Rc::new(Cell::new(0)),
            #[cfg(test)]
            view_pulls: Rc::new(Cell::new(0)),
        }
    }

    /// 拉视图 (面板唯一数据入口)。计数为 O(1) 守卫的测试观测面:
    /// 代数未变的通知不得触发本方法 (spec: … constant work)。
    pub fn state(&self) -> AssetState {
        #[cfg(test)]
        self.view_pulls.set(self.view_pulls.get() + 1);
        self.view.clone()
    }

    /// 视图代数 (面板 O(1) 比对用; spec: Data-change notification is O(1))。
    pub fn view_generation(&self) -> u64 {
        self.generation
    }

    /// 右键菜单 (spec: Right-click context menu — file or directory row;
    /// Project 是分组头, 不在其列; 功能由 manager 提供, 面板落地)。
    pub fn menu(&self, item_id: &str) -> Vec<AssetMenu> {
        match find_kind(&self.view, item_id) {
            Some(kind) if kind.is_menu_row() => vec![
                AssetMenu {
                    id: FUNC_COPY_PATH.to_string(),
                    label: "复制路径".to_string(),
                },
                AssetMenu {
                    id: FUNC_REVEAL.to_string(),
                    label: "在 Finder 显示".to_string(),
                },
            ],
            _ => Vec::new(),
        }
    }

    /// 执行菜单功能 → 效果意图 (面板落地; design D2)。
    /// id 不在视图 / 非菜单行 → no-op。
    pub fn execute(&self, item_id: &str, func_id: &str) -> Vec<AssetEffect> {
        let Some(kind) = find_kind(&self.view, item_id) else {
            return Vec::new(); // 树中不存在 → no-op
        };
        if !kind.is_menu_row() {
            return Vec::new();
        }
        match func_id {
            FUNC_COPY_PATH => vec![AssetEffect::CopyText(item_id.to_string())],
            FUNC_REVEAL => vec![AssetEffect::RevealPath(PathBuf::from(item_id))],
            _ => Vec::new(),
        }
    }

    /// 左键默认打开 (spec: Left-click open hook): 查视图得 kind,
    /// 仅 File 产出 `OpenObject` (目录/模块行由面板切展开, 无打开意图)。
    pub fn open_object_for(&self, item_id: &str) -> Option<OpenObject> {
        match find_kind(&self.view, item_id)? {
            AssetKind::File => Some(OpenObject {
                path: PathBuf::from(item_id),
                kind: AssetKind::File,
            }),
            _ => None,
        }
    }

    /// 全量注册 (workspace 打开/切换; design D6): 替换注册集。
    /// 先清场到 Empty (spec: 切换不闪旧树 / no stale rows) 再异步枚举;
    /// 零注册 → 停在 Empty 且**不跑枚举** (spec: zero-project … no
    /// enumeration runs)。
    pub fn register(
        &mut self,
        ws_dir: PathBuf,
        owners: Vec<Box<dyn EnumeratorOwner + Send>>,
        cx: &mut Context<Self>,
    ) {
        self.ws_dir = Some(ws_dir);
        self.owners = owners;
        self.in_flight = None; // supersede 旧在途
        self.set_view(AssetState::Empty);
        if self.owners.is_empty() {
            return;
        }
        self.spawn_enumerate(cx);
    }

    // 增量注册已随 create ask 流退役 (change solution-service 5.2:
    // add-solution 走全量替换重注册 — register 即增量语义, 不重启)。

    /// 显式刷新 (⟳): 重读规则 + supersede 在途; 代数入口 +1
    /// (spec: explicit refresh 是缓存失效输入之一)。
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.owners.is_empty() {
            return; // 零注册无物可枚举
        }
        self.generation += 1;
        self.spawn_enumerate(cx);
    }

    /// 立即改视图 (同步路径): 代数 +1。通知由调用方收尾。
    fn set_view(&mut self, view: AssetState) {
        self.view = view;
        self.generation += 1;
    }

    /// 一次 spawn 内跑完整条枚举链, 末尾主线程一次 swap (原子 land)。
    fn spawn_enumerate(&mut self, cx: &mut Context<Self>) {
        if self.ws_dir.is_none() {
            return; // 从未注册 (defensive)
        }
        // 过滤供给在 owner 侧 (design D2/D8): 主线程同步构造枚举执行体
        // — owner 不入 spawn (design D2), 执行体 (Send) 丢后台执行。
        let enumerators: Vec<Box<dyn Enumerator>> =
            self.owners.iter().map(|o| o.enumerator()).collect();
        let app = self.app.clone();
        #[cfg(test)]
        self.enumerate_starts.set(self.enumerate_starts.get() + 1);

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let bg = cx.background_executor();
            let outcome = bg.spawn({
                let bg = bg.clone();
                async move {
                    // 单一求值权威在扫描期 (design D1): 枚举产物即视图,
                    // land 前不再有后处理 pass。
                    let mut nodes = Vec::with_capacity(enumerators.len());
                    for en in enumerators {
                        let node = en.enumerate(bg.clone()).await?;
                        nodes.push(node);
                    }
                    // 单一求值权威在扫描期 (design D1): 枚举产物即视图,
                    // land 前不再有后处理 pass。
                    Ok::<Vec<crate::assets::state::AssetNode>, String>(nodes)
                }
            })
            .await;
            // 回主线程: 换 view + 代数 +1, 再经 app 枢纽转发通知
            this.update(cx, |m, _cx| {
                m.view = match outcome {
                    Ok(nodes) => AssetState::Forest { nodes },
                    Err(message) => AssetState::Degraded { message },
                };
                m.generation += 1;
            })
            .ok();
            app.update(cx, |app, cx| app.notify_panels(cx)).ok();
        });
        self.in_flight = Some(task);
    }

    /// 枚举启动计数 (测试缝)。
    #[cfg(test)]
    pub(crate) fn enumerate_starts_handle(&self) -> Rc<Cell<u64>> {
        self.enumerate_starts.clone()
    }

    /// 视图拉取计数 (测试缝: O(1) 守卫 — 代数未变的通知不得拉视图)。
    #[cfg(test)]
    pub(crate) fn view_pulls_handle(&self) -> Rc<Cell<u64>> {
        self.view_pulls.clone()
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
