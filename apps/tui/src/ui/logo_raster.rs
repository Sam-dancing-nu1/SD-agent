//! SVG 子集光栅化器（零依赖）：把 logo.svg 渲染成终端半字符网格。
//!
//! 只支持本仓库 logo 用到的子集：`path`（绝对 M/L/Q/Z，含小数，Q 细分）、
//! `rect`（rx 圆角）、`linearGradient` / `radialGradient`（userSpaceOnUse 与
//! objectBoundingBox 两种，多 stop、stop-opacity）、`fill`/`stroke` 的
//! `url(#id)` / `#hex` / `none`。stroke 用位图膨胀近似（半径 = stroke-width/2
//! 按缩放折算）；stroke-linecap/linejoin 圆头影响很小，忽略。
//!
//! 输出：每格一个半块字符 `▀`，fg=上半像素色、bg=下半像素色；透明区域
//! 合成到深色背素 (34, 38, 44)。像素画布取 cols×(rows*2)，正方形 SVG
//! 等比映射（终端字符高≈宽×2）。
//!
//! 注：`render_logo` 的调用方（ui/logo.rs 接线）尚未落地，模块内私有件在
//! 落地前按死代码告警，故在本模块声明 allow(dead_code)；接线后可移除。

#![allow(dead_code)]

mod parse;
mod raster;

use raster::SS;

/// 终端字符格（每格上下两个半像素）。
///
/// bg=None 表示不涂背景（用终端默认背景）：近背素像素输出空格/单半块，
/// 半块路线天然利用透明，不整面实涂（ASCII 视觉设计手册 image-to-ascii
/// §2.1/§2.2 半块要点）。
pub struct LogoCell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    pub bg: Option<(u8, u8, u8)>,
}

/// 半字符网格（cols×rows 个 [LogoCell]）。
pub struct LogoGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<LogoCell>,
}

/// 深色背素（RGB 0..255 浮点）。
const BG: Rgba = Rgba {
    r: 34.0,
    g: 38.0,
    b: 44.0,
    a: 1.0,
};

#[derive(Clone, Copy, Debug)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Rgba {
    const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// src-over 合成（dst 恒为不透明背素/前层结果）。
fn over(dst: &mut Rgba, src: Rgba) {
    let a = src.a.clamp(0.0, 1.0);
    dst.r = src.r * a + dst.r * (1.0 - a);
    dst.g = src.g * a + dst.g * (1.0 - a);
    dst.b = src.b * a + dst.b * (1.0 - a);
}

#[derive(Clone)]
enum GradKind {
    Linear { x1: f32, y1: f32, x2: f32, y2: f32 },
    Radial { cx: f32, cy: f32, r: f32 },
}

#[derive(Clone)]
struct Grad {
    kind: GradKind,
    /// true = userSpaceOnUse（坐标是 SVG 用户坐标）；false = objectBoundingBox
    /// （坐标是包围盒内 0..1 比例，包围盒由所属图形给出）。
    user_space: bool,
    stops: Vec<(f32, Rgba)>,
}

impl Grad {
    fn eval(&self, px: f32, py: f32, bbox: (f32, f32, f32, f32)) -> Rgba {
        let t = match self.kind {
            GradKind::Linear { x1, y1, x2, y2 } => {
                let (x1, y1, x2, y2) = if self.user_space {
                    (x1, y1, x2, y2)
                } else {
                    (
                        bbox.0 + x1 * bbox.2,
                        bbox.1 + y1 * bbox.3,
                        bbox.0 + x2 * bbox.2,
                        bbox.1 + y2 * bbox.3,
                    )
                };
                let (dx, dy) = (x2 - x1, y2 - y1);
                let den = dx * dx + dy * dy;
                if den <= 0.0 {
                    0.0
                } else {
                    ((px - x1) * dx + (py - y1) * dy) / den
                }
            }
            GradKind::Radial { cx, cy, r } => {
                let (cx, cy, rr) = if self.user_space {
                    (cx, cy, r)
                } else {
                    (bbox.0 + cx * bbox.2, bbox.1 + cy * bbox.3, r * bbox.2)
                };
                if rr <= 0.0 {
                    0.0
                } else {
                    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() / rr
                }
            }
        };
        self.stops_at(t)
    }

    /// 沿 stop 列表线性插值（颜色与 alpha 分量分别插值，同 SVG 默认口径）。
    fn stops_at(&self, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let s = &self.stops;
        if s.is_empty() {
            return Rgba::new(0.0, 0.0, 0.0, 0.0);
        }
        if t <= s[0].0 {
            return s[0].1;
        }
        for w in s.windows(2) {
            let (o0, c0) = w[0];
            let (o1, c1) = w[1];
            if t <= o1 {
                let u = if o1 > o0 { (t - o0) / (o1 - o0) } else { 0.0 };
                return Rgba::new(
                    c0.r + (c1.r - c0.r) * u,
                    c0.g + (c1.g - c0.g) * u,
                    c0.b + (c1.b - c0.b) * u,
                    c0.a + (c1.a - c0.a) * u,
                );
            }
        }
        s[s.len() - 1].1
    }
}

enum Paint {
    None,
    Solid(Rgba),
    Grad(Grad),
}

impl Paint {
    fn eval(&self, px: f32, py: f32, bbox: (f32, f32, f32, f32)) -> Rgba {
        match self {
            Paint::None => Rgba::new(0.0, 0.0, 0.0, 0.0),
            Paint::Solid(c) => *c,
            Paint::Grad(g) => g.eval(px, py, bbox),
        }
    }
    fn is_none(&self) -> bool {
        matches!(self, Paint::None)
    }
}

enum Geom {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
    },
    /// 已展平的闭合子路径折线（Q 已细分）。
    Path { subs: Vec<Vec<(f32, f32)>> },
}

struct Shape {
    geom: Geom,
    fill: Paint,
    stroke: Paint,
    stroke_w: f32,
}

impl Shape {
    /// SVG 用户坐标包围盒（objectBoundingBox 渐变用）。
    fn bbox(&self) -> (f32, f32, f32, f32) {
        match &self.geom {
            Geom::Rect { x, y, w, h, .. } => (*x, *y, *w, *h),
            Geom::Path { subs } => {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for s in subs {
                    for p in s {
                        x0 = x0.min(p.0);
                        y0 = y0.min(p.1);
                        x1 = x1.max(p.0);
                        y1 = y1.max(p.1);
                    }
                }
                if x1 < x0 {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    (x0, y0, x1 - x0, y1 - y0)
                }
            }
        }
    }
}

/// 像素图 → 半字符网格（相邻上下两像素合成一格）。
///
/// 合成规则（半块路线利用透明，背景不涂）：
/// 上下都近背素 → ' '（整格空）；仅上实 → '▀'；仅下实 → '▄'；都实 → '▀' 双色。
fn grid_from_pixels(img: &[Rgba], pw: usize, ph: usize) -> LogoGrid {
    let rows = ph / 2;
    let mut cells = Vec::with_capacity(pw * rows);
    for r in 0..rows {
        for c in 0..pw {
            let t = img[(2 * r) * pw + c];
            let b = img[(2 * r + 1) * pw + c];
            let t_solid = !near_bg(t);
            let b_solid = !near_bg(b);
            let cell = match (t_solid, b_solid) {
                (false, false) => LogoCell {
                    ch: ' ',
                    fg: (0, 0, 0),
                    bg: None,
                },
                (true, false) => LogoCell {
                    ch: '▀',
                    fg: rgb8(t),
                    bg: None,
                },
                (false, true) => LogoCell {
                    ch: '▄',
                    fg: rgb8(b),
                    bg: None,
                },
                (true, true) => LogoCell {
                    ch: '▀',
                    fg: rgb8(t),
                    bg: Some(rgb8(b)),
                },
            };
            cells.push(cell);
        }
    }
    LogoGrid {
        cols: pw,
        rows,
        cells,
    }
}

/// 像素是否接近背素（每通道差 < 6 即视为透明背景）。阈值 14 会把图标
/// 底板的暗部渐变当背景滤掉（icon 只剩字形显小），收到 6 让底板显形。
fn near_bg(p: Rgba) -> bool {
    (p.r - BG.r).abs() < 6.0 && (p.g - BG.g).abs() < 6.0 && (p.b - BG.b).abs() < 6.0
}

fn rgb8(c: Rgba) -> (u8, u8, u8) {
    let f = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    (f(c.r), f(c.g), f(c.b))
}

/// 从 SVG 光栅化到 cols×rows 个终端字符格（内部按 2 倍行分辨率取半像素）。
pub fn render_logo(svg: &str, cols: usize, rows: usize) -> LogoGrid {
    let (shapes, vb) = parse::parse_svg(svg);
    let pw = cols.max(1);
    let ph = rows.max(1) * 2;
    let (vw, vh) = (
        if vb.2 > 0.0 { vb.2 } else { 1.0 },
        if vb.3 > 0.0 { vb.3 } else { 1.0 },
    );
    let kx = pw as f32 / vw;
    let ky = ph as f32 / vh;
    let pix = 1.0 / kx.max(ky); // 1 像素对应的 SVG 尺寸（解析 AA 用）
    let mut img = vec![BG; pw * ph];

    for shape in &shapes {
        let bbox = shape.bbox();
        // 每像素覆盖率：rect 走解析式 AA；path 走超采样位图。
        let mut fcov = vec![0.0f32; pw * ph];
        let mut scov = vec![0.0f32; pw * ph];
        match &shape.geom {
            Geom::Rect { x, y, w, h, rx } => {
                let (x0, y0, x1, y1) = (*x, *y, x + w, y + h);
                for py in 0..ph {
                    for px in 0..pw {
                        let sx = vb.0 + (px as f32 + 0.5) / kx;
                        let sy = vb.1 + (py as f32 + 0.5) / ky;
                        let sd = raster::sd_round_rect(sx, sy, x0, y0, x1, y1, *rx);
                        let i = py * pw + px;
                        fcov[i] = (0.5 - sd / pix).clamp(0.0, 1.0);
                        if shape.stroke_w > 0.0 {
                            scov[i] =
                                (0.5 - (sd.abs() - shape.stroke_w / 2.0) / pix).clamp(0.0, 1.0);
                        }
                    }
                }
            }
            Geom::Path { subs } => {
                let (wi, hi) = (pw * SS, ph * SS);
                let (kxi, kyi) = (kx * SS as f32, ky * SS as f32);
                fcov = raster::downsample(&raster::raster_fill(subs, wi, hi, kxi, kyi), pw, ph);
                if shape.stroke_w > 0.0 && !shape.stroke.is_none() {
                    let cl = raster::raster_centerline(subs, wi, hi, kxi, kyi);
                    let rad = shape.stroke_w / 2.0 * kxi.max(kyi);
                    scov = raster::downsample(&raster::dilate(&cl, wi, hi, rad), pw, ph);
                }
            }
        }

        for py in 0..ph {
            for px in 0..pw {
                let i = py * pw + px;
                let sx = vb.0 + (px as f32 + 0.5) / kx;
                let sy = vb.1 + (py as f32 + 0.5) / ky;
                if !shape.fill.is_none() && fcov[i] > 0.0 {
                    let p = shape.fill.eval(sx, sy, bbox);
                    over(
                        &mut img[i],
                        Rgba {
                            a: p.a / 255.0 * fcov[i],
                            ..p
                        },
                    );
                }
                if scov[i] > 0.0 {
                    let p = shape.stroke.eval(sx, sy, bbox);
                    over(
                        &mut img[i],
                        Rgba {
                            a: p.a / 255.0 * scov[i],
                            ..p
                        },
                    );
                }
            }
        }
    }
    grid_from_pixels(&img, pw, ph)
}

#[cfg(test)]
mod tests;
