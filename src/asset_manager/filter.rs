//! gitignore 式过滤引擎（design D5）——spec 承诺的子集：
//! `*`（不跨 `/`）/ `?` / `**` / 路径模式（相对 workspace 根）/
//! 后写覆盖先写 / `!` 前缀反悔。**零新增 crate**（proposal: 零依赖）：
//! 手写段匹配 + `**` 递归，不引 globset/regex——字符类 `[]`、转义等
//! 超出 spec 子集的 gitignore 特性**有意不实现**（design Risks）。
//!
//! 目录类行不参与求值（结构保留——对 gitignore 的有意偏离），由
//! [`filter_assets`] 保证只对文件行调用本引擎。
//! 模式语义 (asset-manager-right-dock): **相对各 Project 根** —
//! 由 land 路径逐 project 传入自己的 `root_id` (spec: … relative to
//! each project's root)。

use super::dto::AssetNode;

/// 编译后的单条规则。
struct Rule {
    /// `!` 前缀：命中即重新纳入。
    negated: bool,
    /// 含 `/`（去尾斜杠后）或以 `/` 开头 → 锚定整路径；
    /// 否则匹配路径的**任意单段**（gitignore basename 语义）。
    anchored: bool,
    /// 锚定规则的路径段（段内支持 `*`/`?`，段级支持 `**`）；
    /// 非锚定规则恒为单段。
    segs: Vec<String>,
}

impl Rule {
    fn compile(raw: &str) -> Option<Rule> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let (negated, body) = match trimmed.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, trimmed),
        };
        // 尾斜杠 = 目录前缀语义: 文件行按前缀命中 ("build/" 隐藏 build 下的文件)
        let body = body.strip_suffix('/').unwrap_or(body);
        if body.is_empty() {
            return None; // 空规则 ("/"、单独 "!"、全空白) — 跳过
        }
        let anchored = body.starts_with('/') || body.contains('/');
        let body = body.strip_prefix('/').unwrap_or(body);
        Some(Rule {
            negated,
            anchored,
            segs: body.split('/').map(str::to_string).collect(),
        })
    }

    /// 单段 glob 匹配（`*` 不跨段、`?` 单字符；经典回溯）。
    fn matches_segment(pattern: &str, text: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let t: Vec<char> = text.chars().collect();
        let (mut pi, mut ti) = (0usize, 0usize);
        let mut star: Option<usize> = None;
        let mut mark = 0usize;
        while ti < t.len() {
            if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < p.len() && p[pi] == '*' {
                star = Some(pi);
                mark = ti;
                pi += 1;
            } else if let Some(s) = star {
                pi = s + 1;
                mark += 1;
                ti = mark;
            } else {
                return false;
            }
        }
        while pi < p.len() && p[pi] == '*' {
            pi += 1;
        }
        pi == p.len()
    }

    /// 多段匹配（支持 `**` 跨段）。
    fn matches_segments(pat: &[String], txt: &[&str]) -> bool {
        match pat.split_first() {
            None => txt.is_empty(),
            Some((head, rest)) if head == "**" => {
                (0..=txt.len()).any(|skip| Self::matches_segments(rest, &txt[skip..]))
            }
            Some((head, rest)) => match txt.split_first() {
                None => false,
                Some((first, trest)) => {
                    Self::matches_segment(head, first) && Self::matches_segments(rest, trest)
                }
            },
        }
    }

    fn matches(&self, rel_path: &str) -> bool {
        let txt: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        if self.anchored {
            Self::matches_segments(&self.segs, &txt)
        } else {
            // 无 `/` 的模式匹配任意深度的任意段 (basename 语义)
            txt.iter().any(|seg| Self::matches_segment(&self.segs[0], seg))
        }
    }
}

/// 编译好的规则集（`view()` 每次求值编译一份，避免逐文件重复编译）。
pub struct Ruleset {
    rules: Vec<Rule>,
}

impl Ruleset {
    pub fn compile(rules: &[String]) -> Ruleset {
        Ruleset {
            rules: rules.iter().filter_map(|r| Rule::compile(r)).collect(),
        }
    }

    /// gitignore 求值：**后写覆盖先写**，`!` 命中即重新纳入；
    /// 无任何命中 = 默认全显。
    pub fn is_visible(&self, rel_path: &str) -> bool {
        let mut visible = true;
        for rule in &self.rules {
            if rule.matches(rel_path) {
                visible = rule.negated;
            }
        }
        visible
    }
}

/// 对一棵资产子树按 `root_id`（Project 根的绝对路径）求值:
/// 文件行按 [`Ruleset`] 过滤, 目录类行 (Project/Module/Dir) 恒显
/// (spec: Rules SHALL apply to file rows only)。
///
/// 在 land 路径调用 (结果随视图固化 — 数据通知零重过滤)。
pub fn filter_assets(node: &AssetNode, root_id: &str, rules: &Ruleset) -> AssetNode {
    let rel_of = |id: &str| -> String {
        id.strip_prefix(root_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(str::to_string)
            .unwrap_or_else(|| id.to_string())
    };
    let children = node
        .children
        .iter()
        .filter(|child| child.kind.is_folder() || rules.is_visible(&rel_of(&child.id)))
        .map(|child| filter_assets(child, root_id, rules))
        .collect();
    AssetNode {
        id: node.id.clone(),
        label: node.label.clone(),
        kind: node.kind,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ruleset(rules: &[&str]) -> Ruleset {
        let owned: Vec<String> = rules.iter().map(|s| s.to_string()).collect();
        Ruleset::compile(&owned)
    }

    #[test]
    fn glob_empty_rules_show_everything() {
        let r = ruleset(&[]);
        assert!(r.is_visible("main.rs"));
        assert!(r.is_visible("src/deep/file.txt"));
        let blank = ruleset(&["", "  "]);
        assert!(blank.is_visible("main.rs"));
    }

    #[test]
    fn glob_hide_matching_log() {
        let r = ruleset(&["*.log"]);
        assert!(!r.is_visible("app.log"), "根层 .log 应隐藏");
        assert!(!r.is_visible("src/deep/app.log"), "任意深度 .log 应隐藏");
        assert!(r.is_visible("main.rs"), "非匹配文件应显示");
    }

    #[test]
    fn glob_deny_all_reinclude() {
        let r = ruleset(&["*", "!*.rs"]);
        assert!(r.is_visible("main.rs"), "`!` 反悔应纳入 .rs");
        assert!(r.is_visible("src/lib.rs"), "任意深度 .rs 应纳入");
        assert!(!r.is_visible("main.txt"), "其余文件应被 `*` 隐藏");
    }

    #[test]
    fn glob_last_match_wins() {
        let r = ruleset(&["*", "!*.rs", "*_test.rs"]);
        assert!(r.is_visible("foo.rs"), "`!*.rs` 纳入且未被后续规则命中 → 显示");
        assert!(!r.is_visible("foo_test.rs"), "最后命中者 `*_test.rs` 覆盖 → 隐藏");
        assert!(!r.is_visible("bar.txt"), "`*` 仍隐藏非 rs");
    }

    #[test]
    fn glob_path_pattern_anchored() {
        let r = ruleset(&["src/*.rs"]);
        assert!(!r.is_visible("src/app.rs"), "锚定路径直接子项应隐藏");
        assert!(r.is_visible("src/nested/app.rs"), "`*` 不跨 `/` → 更深层不命中");
        assert!(r.is_visible("tests/app.rs"), "其他根不命中");
    }

    #[test]
    fn glob_deep_path_any_depth() {
        let r = ruleset(&["**/*.rs"]);
        assert!(!r.is_visible("a/b/c.rs"), "`**` 跨段 → 任意深度 .rs 隐藏");
        assert!(r.is_visible("a/b/c.txt"), "非 .rs 显示");
    }

    #[test]
    fn glob_trailing_slash_is_dir_prefix() {
        let r = ruleset(&["build/"]);
        assert!(!r.is_visible("pkg/build/out.js"), "尾斜杠 = 目录前缀语义");
        assert!(r.is_visible("pkg/out.js"));
    }

    // ===== filter_assets (spec: File-row filtering relative to project roots) =====

    use crate::asset_manager::{AssetKind, AssetNode};

    fn file(id: &str) -> AssetNode {
        AssetNode {
            id: id.to_string(),
            label: id.rsplit('/').next().unwrap().to_string(),
            kind: AssetKind::File,
            children: Vec::new(),
        }
    }

    fn dir(id: &str, children: Vec<AssetNode>) -> AssetNode {
        AssetNode {
            id: id.to_string(),
            label: id.rsplit('/').next().unwrap().to_string(),
            kind: AssetKind::Dir,
            children,
        }
    }

    /// 合成一棵 `<root>/src/a.rs` + `<root>/docs/b.md` 的目录树。
    fn fixture(root: &str) -> AssetNode {
        dir(
            root,
            vec![
                dir(&format!("{root}/src"), vec![file(&format!("{root}/src/a.rs"))]),
                dir(&format!("{root}/docs"), vec![file(&format!("{root}/docs/b.md"))]),
            ],
        )
    }

    #[test]
    fn filter_assets_keeps_folders_hides_matching_files() {
        let r = ruleset(&["*.md"]);
        let out = filter_assets(&fixture("/p1"), "/p1", &r);
        // 目录行恒显 (含各层), 匹配文件隐藏, 非匹配文件显示
        assert_eq!(out.children.len(), 2, "目录行不参与求值");
        let docs = &out.children[1];
        assert!(docs.children.is_empty(), "b.md 被过滤");
        let src = &out.children[0];
        assert_eq!(src.children.len(), 1, "a.rs 不匹配 *.md → 在");
    }

    #[test]
    fn filter_assets_deny_all_with_reinclude_and_last_match() {
        let r = ruleset(&["*", "!*.rs", "*_test.rs"]);
        let out = filter_assets(&fixture("/p1"), "/p1", &r);
        let src = &out.children[0];
        assert_eq!(src.children.len(), 1, "*.rs 重包含, 但 *_test.rs 后写胜出");
        assert_eq!(src.children[0].label, "a.rs");
        assert!(out.children[1].children.is_empty(), "deny-all 段非 .rs 全隐");
    }

    #[test]
    fn filter_assets_evaluates_each_project_root_separately() {
        // 同一条相对规则分别对两个 Project 根求值 (spec: patterns
        // evaluated per project root — 两侧各自的 src 命中)
        let r = ruleset(&["src/**"]);
        for root in ["/p1", "/p2"] {
            let out = filter_assets(&fixture(root), root, &r);
            let src = out.children.iter().find(|n| n.label == "src").unwrap();
            assert!(
                src.children.is_empty(),
                "{root}: src 子树相对自身根命中规则 → 隐藏"
            );
            let docs = out.children.iter().find(|n| n.label == "docs").unwrap();
            assert_eq!(docs.children.len(), 1, "{root}: 非 src 文件不受影响");
        }
    }

    #[test]
    fn filter_assets_empty_rules_show_everything() {
        let r = ruleset(&[]);
        let out = filter_assets(&fixture("/p1"), "/p1", &r);
        assert_eq!(out.children[0].children.len(), 1);
        assert_eq!(out.children[1].children.len(), 1);
    }
}
