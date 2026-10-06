//! SVG 子集解析：path（绝对 M/L/Q/Z）、rect（rx）、linear/radialGradient、
//! stop-opacity、fill/stroke 引用（url(#id) / #hex / none）。
//!
//! 只覆盖 logo.svg 用到的子集；XML 走标签扫描（含注释跳过），不做完整 XML。

use super::{Geom, Grad, GradKind, Paint, Rgba, Shape};

/// 解析属性对（name="value"，兼容单引号；跳过标签名与无值属性）。
fn tag_attrs(tag: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < tag.len() {
        while i < tag.len() && !b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let ns = i;
        while i < tag.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b':') {
            i += 1;
        }
        if i == ns {
            break;
        }
        let name = &tag[ns..i];
        while i < tag.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= tag.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < tag.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= tag.len() {
            break;
        }
        let q = b[i];
        if q != b'"' && q != b'\'' {
            continue;
        }
        i += 1;
        let vs = i;
        while i < tag.len() && b[i] != q {
            i += 1;
        }
        out.push((name, &tag[vs..i]));
        if i < tag.len() {
            i += 1;
        }
    }
    out
}

fn attr<'a>(pairs: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn fnum(s: &str) -> f32 {
    s.trim().parse::<f32>().unwrap_or(0.0)
}

/// objectBoundingBox 口径数值：百分比或 0..1 小数都折算成比例。
fn ffrac(s: &str) -> f32 {
    let s = s.trim();
    match s.strip_suffix('%') {
        Some(p) => fnum(p) / 100.0,
        None => fnum(s),
    }
}

fn parse_color(s: &str) -> Rgba {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let two = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f32;
        if h.len() == 6 {
            return Rgba::new(two(0), two(2), two(4), 255.0);
        }
        if h.len() == 3 {
            let one = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).unwrap_or(0) as f32 * 17.0;
            return Rgba::new(one(0), one(1), one(2), 255.0);
        }
    }
    Rgba::new(0.0, 0.0, 0.0, 255.0)
}

/// 把 `url(#id)` / `#hex` / `none` 解析成 [Paint]。
pub(super) fn parse_paint(v: Option<&str>, grads: &[(String, Grad)], def: Paint) -> Paint {
    let v = match v {
        Some(v) => v.trim(),
        None => return def,
    };
    if v == "none" {
        return Paint::None;
    }
    if let Some(id) = v.strip_prefix("url(#").and_then(|s| s.strip_suffix(')')) {
        let id = id.trim();
        return match grads.iter().find(|(k, _)| k == id) {
            Some((_, g)) => Paint::Grad(g.clone()),
            None => Paint::None,
        };
    }
    Paint::Solid(parse_color(v))
}

/// 二次贝塞尔细分（24 段/条，误差远小于 1 像素）。
fn flatten_q(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32, out: &mut Vec<(f32, f32)>) {
    const N: usize = 24;
    for s in 1..=N {
        let t = s as f32 / N as f32;
        let u = 1.0 - t;
        out.push((
            u * u * x0 + 2.0 * u * t * x1 + t * t * x2,
            u * u * y0 + 2.0 * u * t * y1 + t * t * y2,
        ));
    }
}

/// 解析 path d（仅绝对 M/L/Q/Z），Q 细分成折线；返回闭合子路径。
pub(super) fn parse_path_d(d: &str) -> Vec<Vec<(f32, f32)>> {
    // 分词：命令字母 / 数字。
    let mut tokens: Vec<String> = Vec::new();
    let mut num = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() {
            if !num.is_empty() {
                tokens.push(std::mem::take(&mut num));
            }
            tokens.push(c.to_string());
        } else if c == '-' || c == '+' {
            if !num.is_empty() {
                tokens.push(std::mem::take(&mut num));
            }
            num.push(c);
        } else if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else if !num.is_empty() {
            tokens.push(std::mem::take(&mut num));
        }
    }
    if !num.is_empty() {
        tokens.push(num);
    }

    let mut subs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut i = 0;
    while i < tokens.len() {
        let cmd = tokens[i].chars().next().unwrap_or('M');
        i += 1;
        let mut nums: Vec<f32> = Vec::new();
        while i < tokens.len() {
            match tokens[i].parse::<f32>() {
                Ok(v) => {
                    nums.push(v);
                    i += 1;
                }
                Err(_) => break,
            }
        }
        let mut k = 0;
        match cmd {
            // M 后续坐标对是隐式 L。
            'M' => {
                if nums.len() >= 2 {
                    let (x, y) = (nums[0], nums[1]);
                    if cur.len() >= 2 {
                        subs.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                    cur.push((x, y));
                    cx = x;
                    cy = y;
                    sx = x;
                    sy = y;
                    k = 2;
                }
                while k + 1 < nums.len() {
                    let (x, y) = (nums[k], nums[k + 1]);
                    k += 2;
                    cur.push((x, y));
                    cx = x;
                    cy = y;
                }
            }
            'L' => {
                while k + 1 < nums.len() {
                    let (x, y) = (nums[k], nums[k + 1]);
                    k += 2;
                    cur.push((x, y));
                    cx = x;
                    cy = y;
                }
            }
            'Q' => {
                while k + 3 < nums.len() {
                    let (qx, qy, x, y) = (nums[k], nums[k + 1], nums[k + 2], nums[k + 3]);
                    k += 4;
                    flatten_q(cx, cy, qx, qy, x, y, &mut cur);
                    cx = x;
                    cy = y;
                }
            }
            'Z' => {
                if cur.len() >= 2 {
                    subs.push(std::mem::take(&mut cur));
                }
                cx = sx;
                cy = sy;
            }
            _ => {}
        }
    }
    if cur.len() >= 2 {
        subs.push(cur);
    }
    subs
}

/// 渐变草稿（open 标签收集 stops，close 标签落库）。
type Draft = (String, GradKind, bool, Vec<(f32, Rgba)>);

fn grad_kind(name: &str, pairs: &[(&str, &str)], user_space: bool) -> GradKind {
    // userSpaceOnUse 取绝对数值；objectBoundingBox 取 0..1 比例（百分比折算）。
    let f = |k: &str, d: f32| {
        attr(pairs, k)
            .map(|v| if user_space { fnum(v) } else { ffrac(v) })
            .unwrap_or(d)
    };
    if name == "linearGradient" {
        GradKind::Linear {
            x1: f("x1", 0.0),
            y1: f("y1", 0.0),
            x2: f("x2", 1.0),
            y2: f("y2", 0.0),
        }
    } else {
        GradKind::Radial {
            cx: f("cx", 0.5),
            cy: f("cy", 0.5),
            r: f("r", 0.5),
        }
    }
}

/// 解析整份 SVG：返回（按文档序的图形，viewBox）。
pub(super) fn parse_svg(svg: &str) -> (Vec<Shape>, (f32, f32, f32, f32)) {
    let mut shapes: Vec<Shape> = Vec::new();
    let mut grads: Vec<(String, Grad)> = Vec::new();
    let mut vb = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut draft: Option<Draft> = None;

    let mut i = 0;
    while i < svg.len() {
        let lt = match svg[i..].find('<') {
            Some(p) => i + p,
            None => break,
        };
        if svg[lt..].starts_with("<!--") {
            i = svg[lt..]
                .find("-->")
                .map(|p| lt + p + 3)
                .unwrap_or(svg.len());
            continue;
        }
        let gt = match svg[lt..].find('>') {
            Some(p) => lt + p,
            None => break,
        };
        let tag = &svg[lt + 1..gt];
        i = gt + 1;
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if draft.is_some() && (name == "linearGradient" || name == "radialGradient") {
                if let Some((id, kind, user_space, stops)) = draft.take() {
                    grads.push((
                        id,
                        Grad {
                            kind,
                            user_space,
                            stops,
                        },
                    ));
                }
            }
            continue;
        }
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
            .collect();
        let pairs = tag_attrs(tag);
        match name.as_str() {
            "svg" => {
                if let Some(v) = attr(&pairs, "viewBox") {
                    let n: Vec<f32> = v
                        .split(|c: char| c.is_whitespace() || c == ',')
                        .filter(|s| !s.is_empty())
                        .map(fnum)
                        .collect();
                    if n.len() == 4 {
                        vb = (n[0], n[1], n[2], n[3]);
                    }
                }
            }
            "linearGradient" | "radialGradient" => {
                let id = attr(&pairs, "id").unwrap_or("").to_string();
                let user_space = attr(&pairs, "gradientUnits") == Some("userSpaceOnUse");
                let kind = grad_kind(&name, &pairs, user_space);
                draft = Some((id, kind, user_space, Vec::new()));
            }
            "stop" => {
                if let Some((_, _, _, stops)) = draft.as_mut() {
                    let off = attr(&pairs, "offset").map(ffrac).unwrap_or(0.0);
                    let mut c = attr(&pairs, "stop-color")
                        .map(parse_color)
                        .unwrap_or(Rgba::new(0.0, 0.0, 0.0, 255.0));
                    let so = attr(&pairs, "stop-opacity").map(fnum).unwrap_or(1.0);
                    c.a = 255.0 * so;
                    stops.push((off, c));
                }
            }
            "rect" | "path" => {
                let f = |k: &str| attr(&pairs, k).map(fnum).unwrap_or(0.0);
                let fill = parse_paint(
                    attr(&pairs, "fill"),
                    &grads,
                    Paint::Solid(Rgba::new(0.0, 0.0, 0.0, 255.0)),
                );
                let stroke = parse_paint(attr(&pairs, "stroke"), &grads, Paint::None);
                let stroke_w = f("stroke-width");
                let geom = if name == "rect" {
                    Geom::Rect {
                        x: f("x"),
                        y: f("y"),
                        w: f("width"),
                        h: f("height"),
                        rx: f("rx"),
                    }
                } else {
                    Geom::Path {
                        subs: parse_path_d(attr(&pairs, "d").unwrap_or("")),
                    }
                };
                shapes.push(Shape {
                    geom,
                    fill,
                    stroke,
                    stroke_w,
                });
            }
            _ => {}
        }
    }
    (shapes, vb)
}
