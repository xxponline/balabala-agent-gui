//! debug-console change task 1.2: 调试面板模块骨架 (design D1/D2/D5)。
//!
//! **整模块 `#![cfg(debug_assertions)]` 门控**：release 构建中本模块
//! 编译为空, 注册表/section 代码不进二进制 (spec: Debug sections register
//! only in debug builds)。范围 = GUI 进程内模块 (design D1), 不提供
//! 跨进程/插件边界的注册通路。
//!
//! 身份登记 [`DebugSection`] 记 `id`/`title`/`factory`；渲染体是 Entity
//! (design D2), 开签入口 (task 5.1) 经工厂创建后包进
//! [`panel::DebugSectionPanel`]。`TerminalApp` 的开签/摘表方法也实现在
//! 本模块 (task 5.1) — 避免 `crate::debug` 的 cfg 门控泄进 `app.rs`。

#![cfg(debug_assertions)]
// 消费方: 开签 5.x/菜单 6.x/注册 8.1 (注册表与开签已接通)
#![allow(dead_code)]

use std::sync::{Mutex, OnceLock};

use gpui_kit::base::dock::PanelId;
use gpui_kit::component::dock::{DockPlacement, panel_handle};
use gpui_kit::{AnyView, App, AppContext, Context, Window};

use crate::app::TerminalApp;

pub mod panel;
pub mod term_rate;
pub mod thread_count;
pub mod threads_pipeline;

/// section 视图工厂: 创建渲染体 Entity 并类型擦除为 [`AnyView`]
/// (design D2: section = Entity, 能持状态/订阅/响应点击)。
/// 首参 = section id (工厂内留作可见性门控键, task 7.1)。
pub type SectionFactory = fn(&'static str, &mut Context<TerminalApp>) -> AnyView;

/// 一个调试 section 的登记项。
// 不派生 PartialEq/Eq: factory 是 fn 指针, 比较不可靠 (rustc 警告) 且无消费方;
// 身份相等一律比 `id` (register/section_by_id 同款)。
#[derive(Clone, Debug)]
pub struct DebugSection {
    /// 去重身份 (specs/center-tabs: section id, 独立于文件路径身份)。
    pub id: &'static str,
    /// 菜单与页签的显示标题。
    pub title: &'static str,
    /// 打开签时创建 section 视图 (task 5.1)。
    pub factory: SectionFactory,
}

/// 进程内注册表 (装配期写入, 菜单构建/开签入口读取)。
fn registry() -> &'static Mutex<Vec<DebugSection>> {
    static REGISTRY: OnceLock<Mutex<Vec<DebugSection>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// 登记一个 section (装配期调用)。
///
/// 同 `id` 重复注册 = 后者覆盖前者 (幂等, 菜单不产生重复项)。
pub fn register(section: DebugSection) {
    let mut regs = registry().lock().expect("debug section registry poisoned");
    match regs.iter_mut().find(|s| s.id == section.id) {
        Some(existing) => *existing = section,
        None => regs.push(section),
    }
}

/// 当前全部已注册 section (按注册序) — 菜单构建 (task 6.2) 的数据源。
pub fn sections() -> Vec<DebugSection> {
    registry()
        .lock()
        .expect("debug section registry poisoned")
        .clone()
}

/// 按 id 查登记项 (开签入口 task 5.1 消费)。
pub fn section_by_id(id: &str) -> Option<DebugSection> {
    sections().into_iter().find(|s| s.id == id)
}

/// 装配期注册内置 sections (task 8.1, design D11: 注册表在
/// `TerminalApp::new` 装配阶段填充, 菜单在装配完成后构建一次)。
/// 幂等 — `register` 按 id 去重, 测试多开窗口重复调无碍。
pub fn register_builtin() {
    register(DebugSection {
        id: "term-rate",
        title: "term-rate",
        factory: term_rate::factory,
    });
    register(DebugSection {
        id: "threads-pipeline",
        title: "threads/pipeline",
        factory: threads_pipeline::factory,
    });
    // change add-lua-language-service (design D1): view 住 crate, 这里 3 行
    // 接线 — 无 TerminalApp 依赖, 工厂参数忽略即为 `_`。
    register(DebugSection {
        id: "lua-index",
        title: "lua-index",
        factory: |_, cx| cx.new(|cx| lua_language_service::LuaIndexView::new(cx)).into(),
    });
}

/// 菜单项 action (task 6.2 产出 section 项 / 6.3 分发接线): 点击 Debug
/// 菜单项 → 打开对应 section 的签 (走 5.1 [`TerminalApp::open_debug_section`] 入口)。
///
/// 动态 section id → 带载荷; `no_json` → 不经 JSON 构建 (菜单直接构造
/// 实例, 也不需要 serde/schemars)。类型随本模块 `cfg(debug_assertions)`
/// 门控 — release 二进制不进 action 名字符串 (task 6.2 字符串验收)。
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = debug_console, no_json)]
pub struct OpenDebugSection {
    /// 目标 section id (与注册表/开签 dedup 表同身份)。
    pub id: String,
}

// ---------- task 5.1: section 开签平行入口 ----------
// 实现在 debug 模块内 (而非 app.rs): 方法体引用 `crate::debug`,
// 跟着模块的 `#![cfg(debug_assertions)]` 一起在 release 消失。

impl TerminalApp {
    /// section 开签平行入口 (specs/center-tabs delta: Debug section tabs
    /// open through the section entry point)。dedup key = section id
    /// (独立于 `open_tabs` 的文件路径身份 — 同字符串互不干扰):
    /// 命中 `select_panel` 聚焦既有签; 未命中查注册表建签尾插
    /// (`add_panel_view(Center)` 尾插, console 恒 index 0);
    /// 未登记 id → no-op (与 unclaimed object 同语义)。
    pub fn open_debug_section(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(&panel) = self.debug_tabs.get(id) {
            self.dock_area
                .update(cx, |area, cx| area.select_panel(panel, window, cx));
            // 库的 select/add 不内建聚焦 (open_object 同款) — 显式交焦
            self.focus_active_center_panel(window, cx);
            return;
        }
        let Some(section) = section_by_id(id) else {
            return; // 未登记: 不可见 (spec: 与 unclaimed object 同语义)
        };
        let view = (section.factory)(section.id, cx);
        let app = cx.weak_entity();
        let panel = cx.new(|cx| panel::DebugSectionPanel::new(
            section.title,
            view,
            app,
            cx,
        ));
        let panel_id = PanelId::from(panel.entity_id());
        self.dock_area.update(cx, |area, cx| {
            // 尾插 + activate (InsertTarget::Tabs{ix:None}) — console 保 index 0
            area.add_panel_view(
                panel_handle(panel),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
        });
        self.debug_tabs.insert(id.to_string(), panel_id);
        self.focus_active_center_panel(window, cx);
    }

    /// 签关闭回调 (`DebugSectionPanel::on_removed`): 从 section dedup
    /// 摘除, 同 section 后续可重开 (specs/center-tabs: Closing a tab
    /// removes its entry)。
    pub fn debug_tab_removed(&mut self, panel_id: PanelId) {
        self.debug_tabs.retain(|_, id| *id != panel_id);
    }

    /// section 可见性 (design D10 采样门控, task 7.1): dedup 有登记
    /// **且** 中心槽当前激活签 = 该 section 的 panel。关签 (dedup 已摘)
    /// 或切走 (激活别的签) → false → section 采样醒时跳过读取
    /// (7.1/7.2: 不可见即不采样)。只读两处全局状态、不持 lease
    /// (`&App` 调用面 — 与 kick 的 `weak.update(cx, …)` 同款, 采样
    /// 任务里拿不到 `&mut Context<TerminalApp>`)。
    pub fn is_section_visible(&self, cx: &App, id: &str) -> bool {
        let Some(&panel) = self.debug_tabs.get(id) else {
            return false; // 未开/已关 — dedup 摘除即不可见
        };
        let area = self.dock_area.read(cx);
        let Some(tree) = area.layout(DockPlacement::Center) else {
            return false; // 中心槽缺席 (窗口拆除中) — 视同不可见
        };
        let mut visible = false;
        tree.root().walk(&mut |node| {
            if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels, active_ix } = node.kind() {
                if panels.get(active_ix) == Some(&panel) {
                    visible = true;
                }
            }
        });
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册语义测试不打开签 — 工厂占位 (开签走 panel_tests)。
    fn stub_factory(_: &'static str, _: &mut Context<TerminalApp>) -> AnyView {
        unimplemented!("注册表测试不创建视图")
    }

    fn sec(id: &'static str) -> DebugSection {
        DebugSection {
            id,
            title: id,
            factory: stub_factory,
        }
    }

    #[test]
    fn register_appends_and_dedups_by_id() {
        register(sec("test::append-a"));
        register(sec("test::append-b"));
        register(sec("test::append-a")); // 同 id 覆盖, 不产生第三项
        let regs = sections();
        let a = regs.iter().filter(|s| s.id == "test::append-a").count();
        assert_eq!(a, 1, "同 id 只留一项");
        assert!(regs.iter().any(|s| s.id == "test::append-b"));
    }

    #[test]
    fn section_by_id_finds_registered_section() {
        register(sec("test::lookup"));
        assert_eq!(section_by_id("test::lookup").map(|s| s.title), Some("test::lookup"));
        assert!(section_by_id("test::definitely-missing").is_none(), "未登记 → None");
    }

    /// change add-lua-language-service 3.2: 装配期注册含 `lua-index`
    /// (spec: Debug menu SHALL list a lua-index debug section — 注册表是
    /// 菜单唯一数据源, 注册在场 ⇔ 菜单列出)。register 幂等, 直调无碍。
    #[test]
    fn register_builtin_includes_lua_index() {
        register_builtin();
        let sec = sections()
            .into_iter()
            .find(|s| s.id == "lua-index")
            .expect("register_builtin 须登记 lua-index (3.2)");
        assert_eq!(sec.title, "lua-index", "标题与 spec 的节名一致");
        assert!(
            section_by_id("lua-index").is_some(),
            "section_by_id 可查 (开签入口消费)"
        );
    }
}

#[cfg(test)]
#[path = "lua_index_tests.rs"]
mod lua_index_tests;
