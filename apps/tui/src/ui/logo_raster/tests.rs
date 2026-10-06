//! logo_raster 单测（自超 500 行拆出）。
use super::*;

fn logo_svg() -> String {
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Some(dir) = std::path::Path::new(file!()).parent() {
        cands.push(dir.join("../../assets/logo.svg"));
    }
    cands.push("assets/logo.svg".into());
    cands.push("apps/tui/assets/logo.svg".into());
    for c in &cands {
        if let Ok(s) = std::fs::read_to_string(c) {
            return s;
        }
    }
    panic!("logo.svg 未找到: {cands:?}");
}

#[test]
fn parse_path_m_l_q_z() {
    let subs = parse::parse_path_d("M10 10L20 10Q25 15 20 20Z");
    assert_eq!(subs.len(), 1);
    let s = &subs[0];
    assert_eq!(s[0], (10.0, 10.0), "起点 = M");
    assert!(s.len() > 5, "Q 细分出折线点: {}", s.len());
    // t=0.5 的二次贝塞尔点 = (22.5, 15)，应有采样点落在附近。
    assert!(s
        .iter()
        .any(|p| (p.0 - 22.5).abs() < 0.3 && (p.1 - 15.0).abs() < 0.3));
    // M 多点 = 隐式 L；Z 闭合后可再起新子路径。
    let subs2 = parse::parse_path_d("M0 0L10 0 10 10ZM20 20L30 20 30 30Z");
    assert_eq!(subs2.len(), 2);
    assert_eq!(subs2[1][0], (20.0, 20.0));
}

#[test]
fn fill_evenodd_has_hole() {
    // 回字形：外 100×100、内孔 25..75，偶奇规则应把内孔挖掉。
    let subs = parse::parse_path_d("M0 0L100 0L100 100L0 100Z M25 25L75 25L75 75L25 75Z");
    let m = raster::raster_fill(&subs, 100, 100, 1.0, 1.0);
    assert_eq!(m[50 * 100 + 50], 0, "内孔不填");
    assert_eq!(m[10 * 100 + 50], 1, "上环填充");
    assert_eq!(m[50 * 100 + 10], 1, "左环填充");
    assert_eq!(m[90 * 100 + 90], 1, "右下环填充");
}

#[test]
fn gradient_linear_interp_and_alpha() {
    let g = Grad {
        kind: GradKind::Linear {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 0.0,
        },
        user_space: true,
        stops: vec![
            (0.0, Rgba::new(255.0, 0.0, 0.0, 255.0)),
            (1.0, Rgba::new(0.0, 0.0, 255.0, 0.0)),
        ],
    };
    let mid = g.eval(50.0, 0.0, (0.0, 0.0, 100.0, 100.0));
    assert!(
        (mid.r - 127.5).abs() < 1.0 && (mid.b - 127.5).abs() < 1.0,
        "{mid:?}"
    );
    assert!(
        (mid.a - 127.5).abs() < 1.0,
        "stop-opacity 线性插值: {}",
        mid.a
    );
    // 多 stop：插值停靠在相邻 stop 之间。
    let g2 = Grad {
        kind: GradKind::Linear {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 0.0,
        },
        user_space: true,
        stops: vec![
            (0.0, Rgba::new(0.0, 0.0, 0.0, 255.0)),
            (0.25, Rgba::new(100.0, 100.0, 100.0, 255.0)),
            (1.0, Rgba::new(255.0, 255.0, 255.0, 255.0)),
        ],
    };
    assert!((g2.eval(12.5, 0.0, (0.0, 0.0, 100.0, 100.0)).r - 50.0).abs() < 1.0);
}

#[test]
fn halfblock_compose() {
    // 2×2 像素 → 2×1 格：上红下蓝、上绿下白。
    let img = vec![
        Rgba::new(255.0, 0.0, 0.0, 255.0),
        Rgba::new(0.0, 255.0, 0.0, 255.0),
        Rgba::new(0.0, 0.0, 255.0, 255.0),
        Rgba::new(255.0, 255.0, 255.0, 255.0),
    ];
    let g = grid_from_pixels(&img, 2, 2);
    assert_eq!((g.cols, g.rows), (2, 1));
    assert_eq!(g.cells[0].ch, '▀');
    assert_eq!(g.cells[0].fg, (255, 0, 0));
    assert_eq!(g.cells[0].bg, Some((0, 0, 255)));
    assert_eq!(g.cells[1].fg, (0, 255, 0));
    assert_eq!(g.cells[1].bg, Some((255, 255, 255)));
}

#[test]
fn halfblock_bg_not_painted() {
    // 近背素像素不涂：上下都近背素 → 空格；单侧实 → 半块无 bg。
    let bg = Rgba {
        r: 34.0,
        g: 38.0,
        b: 44.0,
        a: 1.0,
    };
    let img = vec![bg, Rgba::new(255.0, 200.0, 90.0, 255.0), bg, bg];
    let g = grid_from_pixels(&img, 2, 2);
    assert_eq!(g.cells[0].ch, ' '); // 上近下近 → 空
    assert_eq!(g.cells[0].bg, None);
    assert_eq!(g.cells[1].ch, '▀'); // 上实下近 → 上半块（无 bg 不涂）
    assert_eq!(g.cells[1].fg, (255, 200, 90));
    assert_eq!(g.cells[1].bg, None);
}

#[test]
fn render_logo_halfblock_grid() {
    let g = render_logo(&logo_svg(), 60, 30);
    assert_eq!((g.cols, g.rows), (60, 30));
    assert_eq!(g.cells.len(), 60 * 30);
    // 合成规则四态：' ' / '▀' / '▄'（不整面实涂）。
    assert!(g.cells.iter().all(|c| matches!(c.ch, ' ' | '▀' | '▄')));
    assert!(g.cells.iter().any(|c| c.ch == ' '));
}

/// ASCII 亮度预览（--nocapture 看输出；每格一个亮度阶梯字符）。
#[test]
fn preview_ascii() {
    let g = render_logo(&logo_svg(), 60, 30);
    let ramp: Vec<char> = " .:-=+*#%".chars().collect();
    let mut out = String::new();
    for r in 0..g.rows {
        for c in 0..g.cols {
            let cell = &g.cells[r * g.cols + c];
            let lum =
                |t: (u8, u8, u8)| 0.2126 * t.0 as f32 + 0.7152 * t.1 as f32 + 0.0722 * t.2 as f32;
            let l = (lum(cell.fg) + lum(cell.bg.unwrap_or((34, 38, 44)))) / 2.0 / 255.0;
            let idx = (l * (ramp.len() - 1) as f32).round() as usize;
            out.push(ramp[idx.min(ramp.len() - 1)]);
        }
        out.push('\n');
    }
    print!("{out}");
}
