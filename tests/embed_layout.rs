//! α 渲染管线布局回归：embed ANSI → vt100 → 网格文本（spec: Terminal
//! output rendering 的"输入框贴底"验收项）。
//!
//! 复现验收报告的缺陷：输入框应位于 VT100 网格最下面，而不是第二行。

use std::fs;
use std::io::Write as _;
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use pi_shell_rs::app::embed::{EmbedConfig, EmbeddedApp};

/// 假 pi：记录 cwd 并对每个请求回 success（与 repo A embed 测试同款）。
fn write_stub(dir: &std::path::Path) -> std::path::PathBuf {
    let pwd_file = dir.join("pwd.txt");
    let script = format!(
        "#!/bin/sh\npwd > '{}'\nwhile IFS= read -r line; do\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\n  printf '{{\"type\":\"response\",\"id\":\"%s\",\"success\":true,\"data\":{{}}}}\\n' \"$id\"\ndone\n",
        pwd_file.display()
    );
    let path = dir.join("fake-pi.sh");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn input_box_sits_at_bottom_of_vt100_grid() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("embed-layout-{}-{unique}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stub = write_stub(&dir);

    let app = EmbeddedApp::spawn(EmbedConfig {
        pi_bin: Some(stub.to_str().unwrap().to_string()),
        cwd: Some(dir.clone()),
        cols: 100,
        rows: 30,
        ..Default::default()
    })
    .expect("embed spawn");

    // 收集一段完整帧（首帧 + 后续命令拉取帧），喂给与 GUI 相同的 vt100 解析器
    let mut ansi = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        ansi.extend(app.take_ansi());
        std::thread::sleep(Duration::from_millis(20));
    }
    app.shutdown();
    assert!(!ansi.is_empty(), "no ANSI produced");

    let mut parser =
        vt100::Parser::new(NonZeroU16::new(30).unwrap(), NonZeroU16::new(100).unwrap(), 1000);
    parser.process(&ansi);

    // 转储网格（验收诊断用）
    let screen = parser.screen();
    let mut grid: Vec<String> = Vec::new();
    for row in 0..30 {
        let mut line = String::new();
        for col in 0..100 {
            let cell = screen.cell(row, col);
            line.push_str(cell.map(|c| c.contents()).unwrap_or(""));
        }
        grid.push(line);
    }
    println!("===== vt100 grid (30 rows x 100 cols) =====");
    for (i, line) in grid.iter().enumerate() {
        println!("{i:02}: |{line}|");
    }

    // 输入框边框标题所在行 (标题渲染为 "input(...", 首尾空格会被裁)
    let input_rows: Vec<usize> = grid
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("input("))
        .map(|(i, _)| i)
        .collect();
    println!("input box rows: {input_rows:?}");

    // 断言: 输入框贴在网格底部区域 (最后 6 行内), 而不是顶部
    assert!(
        !input_rows.is_empty(),
        "input box not found in grid; rows dumped above"
    );
    assert!(
        input_rows.iter().all(|&r| r >= 24),
        "input box must sit at the bottom of the grid, found at rows {input_rows:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}
