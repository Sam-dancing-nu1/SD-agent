//! 光栅化原语：偶奇扫描线填充、中心线位图、圆膨胀（stroke 近似）、
//! 子像素降采样、圆角矩形 SDF。全部作用在 0/1 位图与覆盖率数组上。

/// 内部超采样倍数（1 像素 = SS×SS 子像素）。
pub(super) const SS: usize = 4;

/// 偶奇规则扫描线填充（含内孔）。kx/ky = SVG→像素比例，像素中心取 +0.5。
pub(super) fn raster_fill(
    subs: &[Vec<(f32, f32)>],
    w: usize,
    h: usize,
    kx: f32,
    ky: f32,
) -> Vec<u8> {
    let mut m = vec![0u8; w * h];
    let mut edges: Vec<[f32; 4]> = Vec::new();
    for s in subs {
        let n = s.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let (x0, y0) = s[i];
            let (x1, y1) = s[(i + 1) % n];
            if y0 != y1 {
                edges.push([x0, y0, x1, y1]);
            }
        }
    }
    let mut xs: Vec<f32> = Vec::new();
    for iy in 0..h {
        let y = (iy as f32 + 0.5) / ky;
        xs.clear();
        for e in &edges {
            let (x0, y0, x1, y1) = (e[0], e[1], e[2], e[3]);
            if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                xs.push(x0 + (y - y0) * (x1 - x0) / (y1 - y0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < xs.len() {
            let (a, b) = (xs[i] * kx, xs[i + 1] * kx);
            for ix in 0..w {
                let c = ix as f32 + 0.5;
                if c >= a && c <= b {
                    m[iy * w + ix] = 1;
                }
            }
            i += 2;
        }
    }
    m
}

/// 把折线（含闭合回边）画成 1 像素宽的中心线位图。
pub(super) fn raster_centerline(
    subs: &[Vec<(f32, f32)>],
    w: usize,
    h: usize,
    kx: f32,
    ky: f32,
) -> Vec<u8> {
    let mut m = vec![0u8; w * h];
    for s in subs {
        let n = s.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let (x0, y0) = s[i];
            let (x1, y1) = s[(i + 1) % n];
            let (ax, ay) = (x0 * kx, y0 * ky);
            let (bx, by) = (x1 * kx, y1 * ky);
            let steps = (((bx - ax).powi(2) + (by - ay).powi(2)).sqrt() * 2.0)
                .ceil()
                .max(1.0) as usize;
            for st in 0..=steps {
                let t = st as f32 / steps as f32;
                let ix = (ax + (bx - ax) * t) as i32;
                let iy = (ay + (by - ay) * t) as i32;
                if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                    m[iy as usize * w + ix as usize] = 1;
                }
            }
        }
    }
    m
}

/// 圆形核位图膨胀（stroke 加粗近似：半径 = stroke-width/2 折算像素数）。
pub(super) fn dilate(m: &[u8], w: usize, h: usize, radius: f32) -> Vec<u8> {
    if radius <= 0.0 {
        return m.to_vec();
    }
    let r = radius + 0.5;
    let ri = r.ceil() as i32;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            if m[y * w + x] == 0 {
                continue;
            }
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    if (dx * dx + dy * dy) as f32 > r * r {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        out[ny as usize * w + nx as usize] = 1;
                    }
                }
            }
        }
    }
    out
}

/// SS×SS 子像素位图降采样成 0..1 覆盖率。
pub(super) fn downsample(m: &[u8], pw: usize, ph: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; pw * ph];
    let inv = 1.0 / (SS * SS) as f32;
    for py in 0..ph {
        for px in 0..pw {
            let mut s = 0u32;
            for dy in 0..SS {
                for dx in 0..SS {
                    s += m[(py * SS + dy) * pw * SS + (px * SS + dx)] as u32;
                }
            }
            out[py * pw + px] = s as f32 * inv;
        }
    }
    out
}

/// 圆角矩形有符号距离（负 = 内部）。解析式 AA 覆盖率 = 0.5 - sd/像素尺寸。
pub(super) fn sd_round_rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let r = r.min((x1 - x0).min(y1 - y0) / 2.0);
    let qx = (x - (x0 + x1) / 2.0).abs() - (x1 - x0) / 2.0 + r;
    let qy = (y - (y0 + y1) / 2.0).abs() - (y1 - y0) / 2.0 + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.min(0.0).max(qy.min(0.0)) - r
}
