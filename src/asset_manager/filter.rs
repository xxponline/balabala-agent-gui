//! gitignore式过滤引擎(design D4/D5)——spec 承诺的子集:
//! `*`(不跨 `/`)/ `?` / `**` / 路径模式(相对模块根)/ 后写覆盖先写 /
//! `!` 前缀反悔,外加 gitignore **格式层**:行首 `#` 注释、空行、
//! CRLF、行尾空白、BOM、尾斜杠 = 只匹配目录(MUSTBEDIR)、裸词 =
//! 任意深度 basename(NODIR)。**零新增 crate**(proposal: 零依赖):
//! 手写段匹配,不引 globset/regex——字符类 `[]`、转义等超出 spec
//! 子集的特性**有意不实现**(design Non-Goals)。
//!
//! [`FileFilter`] 是枚举期注入的过滤契约(design D2/D3):枚举器对
//! 每个条目依次求值,**任一命中即排除、目录命中即减支**(无序 OR,
//! 无跨 filter 反悔)。首批实现:[`GitIgnoreStyleFilter`]、
//! [`DefaultExcludesFilter`]。
//! 求值:倒序首命中即定(对齐 git `last_matching_pattern_from_list`,
//! dir.c:1428);规则 flag 对齐 `parse_path_pattern`(dir.c:702)。
//! 模式语义:相对各模块根(design D7 逐层 `rel_prefix`)——
//! 单一求值权威在扫描期(design D1,后处理 pass 已删除)。
use super::enumerator::EXCLUDES;

/// 枚举期过滤契约(design D3):`rel_path` 用 `/` 分隔、相对枚举根;
/// `is_dir` 来自 lstat(符号链接 = 文件)。对象安全、`Send`(可入
/// background spawn),无状态、可注入多个(design D2)。
pub trait FileFilter: Send {
    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool;
}

/// 编译后的单条规则。
struct Rule {
    /// `!` 前缀:命中即重新纳入。
    negated: bool,
    /// 尾斜杠 → 只匹配目录 (PATTERN_FLAG_MUSTBEDIR)。
    must_be_dir: bool,
    /// 无 `/` → 只比 basename (PATTERN_FLAG_NODIR, 任意深度的条目自身)。
    /// 其补面即锚定语义(含 `/` 或首字符 `/` → 整路径锚定), 单存此 flag。
    no_dir: bool,
    /// 锚定规则的路径段(段内支持 `*`/`?`,段级支持 `**`);
    /// 非锚定规则恒为单段。
    segs: Vec<String>,
}

impl Rule {
    /// 单行编译:剥 `\r` + 裁行尾空白(git `trim_trailing_spaces`;
    /// 转义空格属非目标)。空行/行首 `#` 由 [`GitIgnoreStyleFilter::compile`] 跳过。
    fn compile(raw: &str) -> Option<Rule> {
        let trimmed = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = trimmed.trim_end();
        if trimmed.is_empty() {
            return None;
        }
        let (negated, body) = match trimmed.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, trimmed),
        };
        // 尾斜杠 = 目录语义 (MUSTBEDIR), 从 pattern 中去掉
        let must_be_dir = body.ends_with('/');
        let body = body.strip_suffix('/').unwrap_or(body);
        if body.is_empty() {
            return None; // 空规则 ("/"、单独 "!"、全空白) — 跳过
        }
        let no_dir = !body.contains('/');
        let body = body.strip_prefix('/').unwrap_or(body);
        Some(Rule {
            negated,
            must_be_dir,
            no_dir,
            segs: body.split('/').map(str::to_string).collect(),
        })
    }

    /// 单段 glob 匹配(`*` 不跨段、`?` 单字符;经典回溯)。
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

    /// 多段匹配(支持 `**` 跨段)。
    fn matches_segments(pat: &[String], txt: &[&str]) -> bool {
        match pat.split_first() {
            None => txt.is_empty(),
            Some((head, rest)) if head == "**" => {
                if rest.is_empty() {
                    // 末位 `**` 只命中“里面的一切” — 须消费 ≥1 段
                    // (spike: `src/**` 不命中 `src` 自身, 与 git
                    // fnmatch 模型一致; 中段 `**` 仍可吃零段)
                    !txt.is_empty()
                } else {
                    (0..=txt.len()).any(|skip| Self::matches_segments(rest, &txt[skip..]))
                }
            }
            Some((head, rest)) => match txt.split_first() {
                None => false,
                Some((first, trest)) => {
                    Self::matches_segment(head, first) && Self::matches_segments(rest, trest)
                }
            },
        }
    }

    fn matches(&self, rel_path: &str, is_dir: bool) -> bool {
        if self.must_be_dir && !is_dir {
            return false; // MUSTBEDIR: 文件直接跳过该规则 (dir.c:1428)
        }
        let txt: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        if txt.is_empty() {
            return false;
        }
        if self.no_dir {
            // NODIR: 只比 basename — 任意深度的条目自身
            return Self::matches_segment(&self.segs[0], txt[txt.len() - 1]);
        }
        Self::matches_segments(&self.segs, &txt)
    }
}

/// 编译好的 gitignore 规则集(design D4):编译一次、逐条目复用。
pub struct GitIgnoreStyleFilter {
    rules: Vec<Rule>,
}

impl GitIgnoreStyleFilter {
    /// 从行数组编译(workspace `file_tree` 配置即行数组)。
    /// 跳过:空行、行首 `#`(注释只认行首)、首行 UTF-8 BOM(git
    /// `add_patterns_from_buffer`, dir.c:1231)。
    pub fn compile(lines: &[String]) -> GitIgnoreStyleFilter {
        let mut rules = Vec::new();
        for (i, raw) in lines.iter().enumerate() {
            let mut line = raw.as_str();
            if i == 0 {
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rule) = Rule::compile(line) {
                rules.push(rule);
            }
        }
        GitIgnoreStyleFilter { rules }
    }

    /// 从整段忽略文本编译(按行拆)。
    /// (bin crate 无外部使用者, 仅测试消费 — 放行未用。)
    #[allow(dead_code)]
    pub fn from_text(text: &str) -> GitIgnoreStyleFilter {
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        Self::compile(&lines)
    }
}

impl FileFilter for GitIgnoreStyleFilter {
    /// gitignore 求值:**倒序首命中即定**(对齐 dir.c:1428);
    /// 命中 `!` → 重新纳入;无任何命中 = 放行。
    fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        for rule in self.rules.iter().rev() {
            if rule.matches(rel_path, is_dir) {
                return !rule.negated;
            }
        }
        false
    }
}

/// 默认排除表 filter(design D5):包 [`EXCLUDES`],按 rel 末段 label
/// 判断、文件/目录同判。与用户规则 OR 组合:用户 filter 的 `!` 无法
/// 跨层反悔,`!node_modules` 救不回 — 与原硬检查逐条目等价。
pub struct DefaultExcludesFilter;

impl FileFilter for DefaultExcludesFilter {
    fn is_ignored(&self, rel_path: &str, _is_dir: bool) -> bool {
        let label = rel_path.rsplit('/').next().unwrap_or(rel_path);
        EXCLUDES.contains(&label)
    }
}

/// 枚举链携带的过滤配方(design D2 修订):`Clone` 逐层转发
/// (便宜),`FileEnumerator` 构造时据此**构建并保存** filter 列表。
#[derive(Clone, Debug)]
pub struct FilterSpec {
    /// workspace `file_tree` 行数组(gitignore 格式)。
    pub rules: Vec<String>,
}

impl FilterSpec {
    pub fn new(rules: Vec<String>) -> Self {
        Self { rules }
    }

    /// 构建 filter 列表(design D5: 组合写死 — 默认排除第一员、
    /// 用户规则其次;未来新增 filter 类型在此扩)。
    pub fn build(&self) -> Vec<Box<dyn FileFilter>> {
        vec![
            Box::new(DefaultExcludesFilter),
            Box::new(GitIgnoreStyleFilter::compile(&self.rules)),
        ]
    }
}

impl Default for FilterSpec {
    fn default() -> Self {
        Self { rules: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitignore(rules: &[&str]) -> GitIgnoreStyleFilter {
        let owned: Vec<String> = rules.iter().map(|s| s.to_string()).collect();
        GitIgnoreStyleFilter::compile(&owned)
    }

    #[test]
    fn glob_empty_rules_show_everything() {
        let r = gitignore(&[]);
        assert!(!r.is_ignored("main.rs", false));
        assert!(!r.is_ignored("src/deep/file.txt", false));
        let blank = gitignore(&["", "  "]);
        assert!(!blank.is_ignored("main.rs", false));
    }

    #[test]
    fn glob_hide_matching_log() {
        let r = gitignore(&["*.log"]);
        assert!(r.is_ignored("app.log", false), "根层 .log 应隐藏");
        assert!(
            r.is_ignored("src/deep/app.log", false),
            "任意深度 .log 应隐藏 (NODIR basename)"
        );
        assert!(!r.is_ignored("main.rs", false), "非匹配文件应显示");
    }

    #[test]
    fn glob_deny_all_reinclude() {
        let r = gitignore(&["*", "!*.rs"]);
        assert!(!r.is_ignored("main.rs", false), "`!` 反悔应纳入 .rs");
        assert!(!r.is_ignored("src/lib.rs", false), "任意深度 .rs 应纳入");
        assert!(r.is_ignored("main.txt", false), "其余文件应被 `*` 隐藏");
    }

    #[test]
    fn glob_last_match_wins() {
        let r = gitignore(&["*", "!*.rs", "*_test.rs"]);
        assert!(!r.is_ignored("foo.rs", false), "`!*.rs` 纳入且未被后续规则命中 → 显示");
        assert!(
            r.is_ignored("foo_test.rs", false),
            "最后命中者 `*_test.rs` 覆盖 → 隐藏"
        );
        assert!(r.is_ignored("bar.txt", false), "`*` 仍隐藏非 rs");
    }

    #[test]
    fn glob_path_pattern_anchored() {
        let r = gitignore(&["src/*.rs"]);
        assert!(r.is_ignored("src/app.rs", false), "锚定路径直接子项应隐藏");
        assert!(
            !r.is_ignored("src/nested/app.rs", false),
            "`*` 不跨 `/` → 更深层不命中"
        );
        assert!(!r.is_ignored("tests/app.rs", false), "其他根不命中");
    }

    #[test]
    fn glob_deep_path_any_depth() {
        let r = gitignore(&["**/*.rs"]);
        assert!(r.is_ignored("a/b/c.rs", false), "`**` 跨段 → 任意深度 .rs 隐藏");
        assert!(!r.is_ignored("a/b/c.txt", false), "非 .rs 显示");
    }

    /// 末位 `**` 须消费 ≥1 段 (design D4; spike 对照真 git check-ignore)。
    #[test]
    fn glob_terminal_double_star_requires_descendant() {
        let r = gitignore(&["src/**"]);
        assert!(!r.is_ignored("src", true), "spike: `src/**` 不命中 `src` 自身");
        assert!(!r.is_ignored("src", false), "同名文件也不命中");
        assert!(r.is_ignored("src/a.rs", false), "命中里面的一切");
        assert!(r.is_ignored("src/deep", true), "任意深度的后代命中");
        let mid = gitignore(&["a/**/b"]);
        assert!(
            mid.is_ignored("a/b", false),
            "中段 `**` 吃零段 (git: zero or more directories)"
        );
    }

    /// MUSTBEDIR: 尾斜杠只命中目录,同名文件不中 (dir.c:702)。
    #[test]
    fn glob_trailing_slash_matches_dirs_only() {
        let r = gitignore(&["build/"]);
        assert!(r.is_ignored("build", true), "尾斜杠命中目录");
        assert!(r.is_ignored("pkg/build", true), "任意深度目录命中");
        assert!(!r.is_ignored("build", false), "同名文件不中 (MUSTBEDIR)");
        assert!(
            !r.is_ignored("pkg/build/out.js", false),
            "文件行不中 — 由父目录减支承担 (2.2)"
        );
    }

    /// NODIR: 裸词只比 basename (任意深度的条目自身, 非路径包含)。
    #[test]
    fn glob_nodir_matches_basename_only() {
        let r = gitignore(&["vendor"]);
        assert!(r.is_ignored("vendor", true), "裸词命中目录");
        assert!(r.is_ignored("src/vendor", true), "任意深度目录");
        assert!(r.is_ignored("vendor", false), "命中同名文件");
        assert!(
            !r.is_ignored("pkg/vendor/x.txt", false),
            "文件行自身不中 — 由父目录减支承担"
        );
        assert!(!r.is_ignored("vendorx", false), "basename 段精确匹配 (非后缀)");
    }

    /// spike 事实 2 (真 git check-ignore 对照): `*` + `!*.rs` —
    /// 目录被 `*` 命中即剪枝, 嵌套 .rs 的救回因此永不可达。
    #[test]
    fn spike_deny_all_nested_rescue_blocked_by_dir_prune() {
        let r = gitignore(&["*", "!*.rs"]);
        assert!(r.is_ignored("src", true), "spike: 目录 src 被 `*` 命中 → 剪枝触发");
        assert!(!r.is_ignored("main.rs", false), "spike: 根层 .rs 被 `!` 救回");
        assert!(
            !r.is_ignored("src/a.rs", false),
            "直接求值可救回 — 但目录已剪, 枚举中永不会被问到 (整支缺席由 2.2 钉)"
        );
    }

    /// spike 事实 3: `build/` + `!build/keep.txt` — 剪枝后救回不可达。
    #[test]
    fn spike_reinclude_beneath_excluded_dir_never_reached() {
        let r = gitignore(&["build/", "!build/keep.txt"]);
        assert!(r.is_ignored("build", true), "spike: build 目录被 `build/` 命中 → 剪枝");
        assert!(
            !r.is_ignored("build/keep.txt", false),
            "直接求值可救回 — 剪枝后永不会被问到 (整支缺席由 2.2 钉)"
        );
    }

    /// gitignore 格式层: BOM / 行首 `#` 注释 / 空行 / 全空白行 / CRLF
    /// (design Goals: 格式层完整复刻)。
    #[test]
    fn from_text_parses_gitignore_format_lines() {
        let text = "\u{feff}# 注释行\r\n\r\n   \n*.log\r\nbuild/\n";
        let r = GitIgnoreStyleFilter::from_text(text);
        assert!(r.is_ignored("a.log", false), "CRLF 行解析生效");
        assert!(r.is_ignored("build", true), "尾斜杠行生效");
        assert!(!r.is_ignored("main.rs", false), "注释/空行/空白行不产生规则");
    }

    // ===== DefaultExcludesFilter (design D5) =====

    #[test]
    fn default_excludes_match_label_at_any_depth() {
        let f = DefaultExcludesFilter;
        assert!(f.is_ignored("node_modules", true), "目录命中");
        assert!(f.is_ignored("src/deep/node_modules", true), "任意深度命中");
        assert!(f.is_ignored(".DS_Store", false), "文件形态命中");
        assert!(f.is_ignored("src/.git", true), ".git 目录");
        assert!(!f.is_ignored("src/main.rs", false), "非排除项放行");
        assert!(!f.is_ignored("mynode_modules", false), "label 精确匹配, 非子串");
        assert!(!f.is_ignored("build.rs", false), "label 不等于排除名则放行");
    }
}
