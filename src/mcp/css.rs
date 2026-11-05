//! CSS ↔ node styles for the MCP tools.
//!
//! Agents describe styles the way Paper's MCP server expects them: React-style camelCase CSS
//! property maps (`{"backgroundColor": "#fff", "padding": "20px"}`) or inline `style="..."`
//! attributes. This module maps the subset that has a Sketch equivalent onto `Style`,
//! `TextStyle`, `AutoLayout` and `LayoutChild`, reports the rest as ignored, and turns nodes
//! back into CSS for `get_computed_styles` / `get_jsx`.
//!
//! The report is what an agent relies on to know its design landed, so a property here must
//! either work or come back in `ignored`. Never accept one and draw something else.

use crate::geom::v2;
use crate::model::rough::{FillStyle, Rough};
use crate::model::{
    Align, AutoLayout, BlendMode, BorderSide, Color, CssAngle, Dim, Direction, Distribute, GridPos,
    GridSpec, ImageMode, LayoutMode, Node, NodeKind, Overflow, Paint, ScrollPosition, SelfAlign,
    Shadow, Sizing, Stop, TextAlign,
};
use serde_json::{Map, Value};

/// An ordered list of `(camelCaseProperty, value)` pairs.
pub type Styles = Vec<(String, String)>;

/// `background-color` → `backgroundColor`; camelCase passes through.
pub fn camel(prop: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in prop.trim().chars() {
        if c == '-' {
            up = !out.is_empty();
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse an inline `style` attribute (`a: b; c: d`), respecting parentheses and quotes.
pub fn parse_inline(s: &str) -> Styles {
    let mut out = Vec::new();
    for decl in split_top(s, ';') {
        if let Some((k, v)) = decl.split_once(':') {
            let (k, v) = (k.trim(), v.trim().trim_end_matches("!important").trim());
            if !k.is_empty() && !v.is_empty() {
                out.push((camel(k), v.to_string()));
            }
        }
    }
    out
}

/// A JSON style object (`{"padding": 8, "color": "#000"}`) as styles. Numbers become px for
/// lengths and stay unitless for the few unitless properties.
pub fn from_json(v: &Value) -> Result<Styles, String> {
    let Some(m) = v.as_object() else {
        return Err("styles must be an object".into());
    };
    Ok(m.iter()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => {
                    let unitless = matches!(
                        camel(k).as_str(),
                        "opacity"
                            | "fontWeight"
                            | "lineHeight"
                            | "flexGrow"
                            | "flexShrink"
                            | "zIndex"
                            | "roughness"
                            | "seed"
                            | "bowing"
                    );
                    if unitless {
                        n.to_string()
                    } else {
                        format!("{n}px")
                    }
                }
                Value::Bool(b) => b.to_string(),
                Value::Null => return None,
                other => other.to_string(),
            };
            Some((camel(k), s))
        })
        .collect())
}

/// Split on `sep` outside parentheses and quotes.
pub fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut cur) = (0i32, None::<char>, String::new());
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, c) if c == sep && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split on whitespace outside parentheses (`1px solid rgb(0, 0, 0)` → 3 parts).
pub fn split_ws(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ------------------------------------------------------------------------------ values

/// A CSS length in px. `em` is relative to `font_size`; percentages are returned as `None`
/// (use [`percent`]).
pub fn length(s: &str, font_size: f64) -> Option<f64> {
    let s = s.trim();
    let num = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    if s == "0" {
        return Some(0.0);
    }
    if let Some(t) = s.strip_suffix("px") {
        return num(t);
    }
    if let Some(t) = s.strip_suffix("rem") {
        return num(t).map(|v| v * 16.0);
    }
    if let Some(t) = s.strip_suffix("em") {
        return num(t).map(|v| v * font_size);
    }
    if let Some(t) = s.strip_suffix("pt") {
        return num(t).map(|v| v * 4.0 / 3.0);
    }
    num(s)
}

pub fn percent(s: &str) -> Option<f64> {
    s.trim()
        .strip_suffix('%')
        .and_then(|t| t.trim().parse::<f64>().ok())
        .map(|v| v / 100.0)
}

/// 1–4 value shorthand (padding, border-radius): top/tl, right/tr, bottom/br, left/bl.
pub fn four(s: &str, font_size: f64) -> Option<[f64; 4]> {
    let v: Vec<f64> = split_ws(s)
        .iter()
        .map(|p| length(p, font_size))
        .collect::<Option<_>>()?;
    Some(match v.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d, ..] => [*a, *b, *c, *d],
        [] => return None,
    })
}

/// `auto`, a length or a percentage.
pub fn dim(s: &str, font_size: f64) -> Option<Dim> {
    let s = s.trim();
    if s == "auto" {
        return Some(Dim::Auto);
    }
    match percent(s) {
        Some(p) => Some(Dim::Pct(p)),
        None => length(s, font_size).map(Dim::Px),
    }
}

/// 1–4 value shorthand that allows `auto` and percentages (margin).
fn four_dims(s: &str, font_size: f64) -> Option<[Dim; 4]> {
    let v: Vec<Dim> = split_ws(s)
        .iter()
        .map(|p| dim(p, font_size))
        .collect::<Option<_>>()?;
    Some(match v.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d, ..] => [*a, *b, *c, *d],
        [] => return None,
    })
}

/// A CSS `font-weight` as a number.
pub fn font_weight(v: &str) -> Option<u16> {
    match v.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" | "bolder" => Some(700),
        "lighter" => Some(300),
        n => n
            .parse::<f64>()
            .ok()
            .filter(|w| (1.0..=1000.0).contains(w))
            .map(|w| w as u16),
    }
}

/// Whether a `font-family` list resolves to the one bundled family. The generic sans-serif
/// names count, since Inter is what a sans-serif means here.
pub fn is_inter(v: &str) -> bool {
    let first = split_top(v, ',').first().map_or(String::new(), |f| {
        f.trim_matches(['"', '\'']).to_ascii_lowercase()
    });
    matches!(
        first.as_str(),
        "inter" | "inter var" | "sans-serif" | "system-ui" | "ui-sans-serif" | "inherit" | ""
    )
}

/// Apply CSS `text-transform` to characters one at a time, so text spans keep their offsets.
pub fn transform_text(chars: &mut [char], mode: &str) {
    let mode = mode.trim().to_ascii_lowercase();
    let mut word_start = true;
    for c in chars.iter_mut() {
        let up = c.to_uppercase().next().unwrap_or(*c);
        let low = c.to_lowercase().next().unwrap_or(*c);
        match mode.as_str() {
            "uppercase" => *c = up,
            "lowercase" => *c = low,
            "capitalize" if word_start => *c = up,
            _ => {}
        }
        word_start = c.is_whitespace();
    }
}

/// A `box-shadow` value; `none` gives an empty list.
pub fn shadows(v: &str, font_size: f64) -> Option<Vec<Shadow>> {
    if v.trim().eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    split_top(v, ',')
        .iter()
        .map(|layer| {
            let (mut lens, mut col, mut inset) = (Vec::new(), None, false);
            for p in split_ws(layer) {
                if p.eq_ignore_ascii_case("inset") {
                    inset = true;
                } else if let Some(l) = length(&p, font_size) {
                    lens.push(l);
                } else {
                    col = Some(color(&p)?);
                }
            }
            if !(2..=4).contains(&lens.len()) {
                return None;
            }
            Some(Shadow {
                x: lens[0],
                y: lens[1],
                blur: lens.get(2).copied().unwrap_or(0.0).max(0.0),
                spread: lens.get(3).copied().unwrap_or(0.0),
                color: col.unwrap_or(Color::BLACK),
                inset,
            })
        })
        .collect()
}

/// A `transform` made only of pixel translations, as an x and y shift.
pub fn translation(v: &str, font_size: f64) -> Option<[f64; 2]> {
    if v.trim().eq_ignore_ascii_case("none") {
        return Some([0.0; 2]);
    }
    let mut out = [0.0; 2];
    for f in split_ws(v) {
        let (name, args) = f.split_once('(')?;
        let args: Vec<f64> = split_top(args.strip_suffix(')')?, ',')
            .iter()
            .map(|a| length(a, font_size))
            .collect::<Option<_>>()?;
        match (name.to_ascii_lowercase().as_str(), args.as_slice()) {
            ("translate", [x]) | ("translatex", [x]) => out[0] += x,
            ("translate", [x, y]) => {
                out[0] += x;
                out[1] += y;
            }
            ("translatey", [y]) => out[1] += y,
            ("translate3d", [x, y, z]) if *z == 0.0 => {
                out[0] += x;
                out[1] += y;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// A `grid-column` or `grid-row` value: `2`, `1 / 3`, `span 2`, `1 / span 2`.
fn grid_line(v: &str) -> Option<[GridPos; 2]> {
    let one = |s: &str| -> Option<GridPos> {
        let s = s.trim().to_ascii_lowercase();
        if s == "auto" {
            Some(GridPos::Auto)
        } else if let Some(n) = s.strip_prefix("span") {
            n.trim().parse().ok().map(GridPos::Span)
        } else {
            s.parse().ok().map(GridPos::Line)
        }
    };
    let (a, b) = v.split_once('/').unwrap_or((v, "auto"));
    Some([one(a)?, one(b)?])
}

const NAMED: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("red", 0xff0000),
    ("green", 0x008000),
    ("lime", 0x00ff00),
    ("blue", 0x0000ff),
    ("yellow", 0xffff00),
    ("orange", 0xffa500),
    ("purple", 0x800080),
    ("pink", 0xffc0cb),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("lightgray", 0xd3d3d3),
    ("lightgrey", 0xd3d3d3),
    ("darkgray", 0xa9a9a9),
    ("darkgrey", 0xa9a9a9),
    ("silver", 0xc0c0c0),
    ("navy", 0x000080),
    ("teal", 0x008080),
    ("aqua", 0x00ffff),
    ("cyan", 0x00ffff),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("maroon", 0x800000),
    ("olive", 0x808000),
    ("brown", 0xa52a2a),
    ("indigo", 0x4b0082),
    ("violet", 0xee82ee),
    ("gold", 0xffd700),
    ("coral", 0xff7f50),
    ("salmon", 0xfa8072),
    ("tomato", 0xff6347),
    ("crimson", 0xdc143c),
    ("skyblue", 0x87ceeb),
    ("steelblue", 0x4682b4),
    ("royalblue", 0x4169e1),
    ("dodgerblue", 0x1e90ff),
    ("tan", 0xd2b48c),
    ("beige", 0xf5f5dc),
    ("ivory", 0xfffff0),
    ("whitesmoke", 0xf5f5f5),
    ("gainsboro", 0xdcdcdc),
    ("slategray", 0x708090),
    ("darkslategray", 0x2f4f4f),
    ("forestgreen", 0x228b22),
    ("seagreen", 0x2e8b57),
    ("mintcream", 0xf5fffa),
    ("lavender", 0xe6e6fa),
    ("rebeccapurple", 0x663399),
];

fn channel(s: &str, scale: f64) -> Option<f64> {
    let s = s.trim();
    match percent(s) {
        Some(p) => Some(p * scale),
        None => s.parse::<f64>().ok(),
    }
}

fn alpha(s: Option<&String>) -> Option<f64> {
    match s {
        None => Some(1.0),
        Some(a) => channel(a, 1.0),
    }
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn func_args(s: &str) -> Option<(String, Vec<String>)> {
    let open = s.find('(')?;
    let name = s[..open].trim().to_ascii_lowercase();
    let inner = s[open + 1..].strip_suffix(')')?;
    // Both `rgb(1, 2, 3, .5)` and `rgb(1 2 3 / .5)`.
    let parts: Vec<String> = if inner.contains(',') {
        inner.split(',').map(|p| p.trim().to_string()).collect()
    } else {
        let (main, a) = match inner.split_once('/') {
            Some((m, a)) => (m, Some(a.trim().to_string())),
            None => (inner, None),
        };
        let mut v: Vec<String> = main.split_whitespace().map(String::from).collect();
        v.extend(a);
        v
    };
    Some((name, parts))
}

fn hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = h.rem_euclid(360.0) / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |t: f64| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

fn oklab_to_srgb(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let lin = [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3,
    ];
    lin.map(|c| {
        if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    })
}

/// Parse a CSS color: hex, `rgb[a]()`, `hsl[a]()`, `oklch()`, `oklab()`, named colors and
/// `transparent`.
pub fn color(s: &str) -> Option<Color> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if lower == "transparent" || lower == "none" {
        return Some(Color::TRANSPARENT);
    }
    if let Some(h) = s.strip_prefix('#') {
        let h: String = match h.len() {
            3 | 4 => h.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => h.to_string(),
            _ => return None,
        };
        return Color::from_hex(&format!("#{h}"));
    }
    if let Some((_, rgb)) = NAMED.iter().find(|(n, _)| *n == lower) {
        return Some(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8));
    }
    let (name, a) = func_args(&lower)?;
    let rgb = match name.as_str() {
        "rgb" | "rgba" if a.len() >= 3 => [
            channel(&a[0], 255.0)? / 255.0,
            channel(&a[1], 255.0)? / 255.0,
            channel(&a[2], 255.0)? / 255.0,
        ],
        "hsl" | "hsla" if a.len() >= 3 => {
            let h = a[0].trim_end_matches("deg").parse::<f64>().ok()?;
            let sat = percent(&a[1]).or_else(|| a[1].parse::<f64>().ok().map(|v| v / 100.0))?;
            let l = percent(&a[2]).or_else(|| a[2].parse::<f64>().ok().map(|v| v / 100.0))?;
            hsl(h, sat, l)
        }
        "oklch" if a.len() >= 3 => {
            let l = channel(&a[0], 1.0)?;
            let c = channel(&a[1], 0.4)?;
            let h = a[2]
                .trim_end_matches("deg")
                .parse::<f64>()
                .ok()?
                .to_radians();
            oklab_to_srgb(l, c * h.cos(), c * h.sin())
        }
        "oklab" if a.len() >= 3 => oklab_to_srgb(
            channel(&a[0], 1.0)?,
            channel(&a[1], 0.4)?,
            channel(&a[2], 0.4)?,
        ),
        _ => return None,
    };
    let al = alpha(a.get(3))?;
    Some(Color::rgba(
        to_u8(rgb[0]),
        to_u8(rgb[1]),
        to_u8(rgb[2]),
        to_u8(al),
    ))
}

/// `rgba()` text for a color (hex when opaque).
pub fn color_css(c: Color) -> String {
    if c.a == 255 {
        c.to_hex()
    } else {
        format!(
            "rgba({}, {}, {}, {})",
            c.r,
            c.g,
            c.b,
            (c.a as f64 / 255.0 * 1000.0).round() / 1000.0
        )
    }
}

/// `linear-gradient(...)` / `radial-gradient(...)` as a paint.
pub fn gradient(s: &str) -> Option<Paint> {
    let (name, _) = func_args(s)?;
    let open = s.find('(')?;
    let inner = s[open + 1..].trim().strip_suffix(')')?;
    let args = split_top(inner, ',');
    let mut stops_src: &[String] = &args;
    let mut angle = 180.0f64; // to bottom
    let mut to_corner = None;
    if name == "linear-gradient" {
        let first = args.first()?.to_ascii_lowercase();
        if let Some(d) = first.strip_suffix("turn") {
            angle = d.trim().parse::<f64>().ok()? * 360.0;
            stops_src = &args[1..];
        } else if let Some(d) = first.strip_suffix("rad") {
            angle = d.trim().parse::<f64>().ok()?.to_degrees();
            stops_src = &args[1..];
        } else if let Some(d) = first.strip_suffix("deg") {
            angle = d.trim().parse().ok()?;
            stops_src = &args[1..];
        } else if let Some(dir) = first.strip_prefix("to ") {
            let (mut x, mut y) = (0.0f64, 0.0f64);
            for w in dir.split_whitespace() {
                match w {
                    "top" => y = -1.0,
                    "bottom" => y = 1.0,
                    "left" => x = -1.0,
                    "right" => x = 1.0,
                    _ => {}
                }
            }
            angle = x.atan2(-y).to_degrees();
            to_corner = Some((x as i8, y as i8));
            stops_src = &args[1..];
        }
    } else if name == "radial-gradient" {
        if args
            .first()
            .is_some_and(|a| color(&split_ws(a)[0]).is_none())
        {
            stops_src = &args[1..];
        }
    } else {
        return None;
    }
    let n = stops_src.len();
    if n == 0 {
        return None;
    }
    let mut stops = Vec::new();
    for (i, st) in stops_src.iter().enumerate() {
        let parts = split_ws(st);
        let c = color(parts.first()?)?;
        let pos = parts.get(1).and_then(|p| percent(p)).unwrap_or(if n == 1 {
            0.0
        } else {
            i as f64 / (n - 1) as f64
        });
        stops.push(Stop { pos, color: c });
    }
    if stops.len() == 1 {
        stops.push(stops[0]);
        stops[1].pos = 1.0;
    }
    Some(if name == "linear-gradient" {
        let r = angle.to_radians();
        let d = v2(r.sin() * 0.5, -r.cos() * 0.5);
        Paint::Linear {
            stops,
            from: v2(0.5 - d.x, 0.5 - d.y),
            to: v2(0.5 + d.x, 0.5 + d.y),
            css: Some(match to_corner {
                Some((x, y)) => CssAngle::To(x, y),
                None => CssAngle::Deg(angle),
            }),
        }
    } else {
        Paint::Radial {
            stops,
            center: v2(0.5, 0.5),
            radius: v2(0.5, 0.5),
        }
    })
}

fn gradient_css(p: &Paint) -> Option<String> {
    let stops = |s: &[Stop]| {
        s.iter()
            .map(|s| format!("{} {}%", color_css(s.color), (s.pos * 100.0).round()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match p {
        Paint::Linear {
            stops: s,
            css: Some(CssAngle::To(x, y)),
            ..
        } => {
            let side = |v: i8, neg: &'static str, pos: &'static str| match v {
                0 => "",
                v if v < 0 => neg,
                _ => pos,
            };
            let dir = [side(*y, "top", "bottom"), side(*x, "left", "right")]
                .iter()
                .filter(|w| !w.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            Some(format!("linear-gradient(to {dir}, {})", stops(s)))
        }
        Paint::Linear {
            stops: s,
            css: Some(CssAngle::Deg(a)),
            ..
        } => Some(format!(
            "linear-gradient({}deg, {})",
            a.rem_euclid(360.0).round(),
            stops(s)
        )),
        Paint::Linear {
            stops: s, from, to, ..
        } => {
            let a = (to.x - from.x)
                .atan2(-(to.y - from.y))
                .to_degrees()
                .rem_euclid(360.0);
            Some(format!("linear-gradient({}deg, {})", a.round(), stops(s)))
        }
        Paint::Radial { stops: s, .. } => Some(format!("radial-gradient({})", stops(s))),
        _ => None,
    }
}

/// `url(...)` target.
pub fn url(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix("url(")?.strip_suffix(')')?;
    Some(inner.trim().trim_matches(['"', '\'']).to_string())
}

// ------------------------------------------------------------------------------ apply

/// Where the node sits, for properties whose meaning depends on the parent.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// The parent's auto layout (None = free positioning or page root).
    pub parent_layout: Option<AutoLayout>,
    /// The parent's size (for percentages and right/bottom).
    pub parent_size: Option<(f64, f64)>,
}

/// What `apply` did besides changing the node.
#[derive(Debug, Default)]
pub struct Applied {
    /// Properties with no effect in Sketch (reported back to the agent).
    pub ignored: Vec<String>,
    /// Image URL to resolve and use as the top fill (resolved by the caller).
    pub image: Option<(String, ImageMode)>,
    /// `right` / `bottom` offsets to resolve once the size is known.
    pub right: Option<f64>,
    pub bottom: Option<f64>,
}

fn axis_set(n: &mut Node, axis: usize, s: Sizing) {
    n.layout_child.sizing[axis] = s;
}

fn layout_mut(n: &mut Node) -> &mut AutoLayout {
    n.layout.get_or_insert(AutoLayout {
        direction: Direction::Horizontal,
        gap: 0.0,
        cross_gap: 0.0,
        padding: [0.0; 4],
        ..AutoLayout::default()
    })
}

fn set_text_flag(n: &mut Node, f: impl Fn(&mut crate::model::TextSpan)) {
    let len = n.kind.text().map_or(0, |t| t.chars().count());
    if len > 0 {
        n.text_style.apply_range(0, len, f);
    }
}

fn set_size(n: &mut Node, axis: usize, v: &str, ctx: &Context, a: &mut Applied, key: &str) {
    let fs = n.kind.font_size().unwrap_or(16.0);
    let v = v.trim();
    let is_text = matches!(n.kind, NodeKind::Text { .. });
    match v {
        "fit-content" | "auto" | "max-content" | "min-content" => {
            n.layout_child.percent[axis] = None;
            if is_text {
                if axis == 0
                    && let NodeKind::Text { auto_width, .. } = &mut n.kind
                {
                    *auto_width = true;
                }
                // `max-content` never wraps; the others fit the space they're given.
                let wraps = axis == 0 && v != "max-content";
                axis_set(n, axis, if wraps { Sizing::Hug } else { Sizing::Fixed });
            } else {
                axis_set(n, axis, Sizing::Hug);
            }
        }
        _ => {
            if let Some(p) = percent(v) {
                if ctx.parent_layout.is_some() && p >= 0.999 {
                    n.layout_child.percent[axis] = None;
                    axis_set(n, axis, Sizing::Fill);
                    if axis == 0
                        && let NodeKind::Text { auto_width, .. } = &mut n.kind
                    {
                        *auto_width = false;
                    }
                } else if ctx.parent_layout.is_some() {
                    // Layout resolves it against the parent's final size.
                    n.layout_child.percent[axis] = Some(p);
                    axis_set(n, axis, Sizing::Fixed);
                    if axis == 0
                        && let NodeKind::Text { auto_width, .. } = &mut n.kind
                    {
                        *auto_width = false;
                    }
                } else if let Some(ps) = ctx.parent_size {
                    let size = if axis == 0 { ps.0 } else { ps.1 } * p;
                    resize_axis(n, axis, size);
                    axis_set(n, axis, Sizing::Fixed);
                } else {
                    a.ignored.push(key.into());
                }
            } else if let Some(px) = length(v, fs) {
                n.layout_child.percent[axis] = None;
                resize_axis(n, axis, px.max(0.0));
                axis_set(n, axis, Sizing::Fixed);
            } else {
                a.ignored.push(key.into());
            }
        }
    }
}

fn resize_axis(n: &mut Node, axis: usize, size: f64) {
    let mut r = n.rect;
    if axis == 0 {
        r.w = size;
        if let NodeKind::Text { auto_width, .. } = &mut n.kind {
            *auto_width = false;
        }
    } else {
        r.h = size;
    }
    if matches!(n.kind, NodeKind::Text { .. }) {
        n.rect = r;
    } else {
        n.set_rect_scaling(r);
    }
}

fn self_align(v: &str) -> Option<Option<SelfAlign>> {
    Some(match v.trim() {
        "auto" | "normal" => None,
        "center" => Some(SelfAlign::Center),
        "flex-start" | "start" | "self-start" => Some(SelfAlign::Start),
        "flex-end" | "end" | "self-end" => Some(SelfAlign::End),
        "stretch" => Some(SelfAlign::Stretch),
        _ => return None,
    })
}

fn set_border_space(n: &mut Node, side: Option<usize>, width: f64) {
    if let Some(l) = &mut n.layout {
        match side {
            Some(i) => l.border[i] = width,
            None => l.border = [width; 4],
        }
    }
}

/// The per-side borders, starting from the uniform border when there are none yet.
fn sides_mut(n: &mut Node) -> &mut [BorderSide; 4] {
    let (stroke, width) = (n.style.stroke, n.style.stroke_width);
    n.style.borders.get_or_insert_with(|| {
        [BorderSide {
            width: if stroke.is_some() { width } else { 0.0 },
            color: stroke,
        }; 4]
    })
}

/// One side's `border-top`-style shorthand. False for dashes, which only uniform borders have.
fn apply_side(n: &mut Node, i: usize, v: &str, fs: f64) -> bool {
    let lower = v.to_ascii_lowercase();
    if lower == "none" || lower == "0" {
        sides_mut(n)[i].width = 0.0;
        set_border_space(n, Some(i), 0.0);
        return true;
    }
    let (mut width, mut col, mut solid) = (1.0, Color::INK, true);
    let mut any = false;
    for p in split_ws(&lower) {
        if let Some(w) = length(&p, fs) {
            width = w;
            any = true;
        } else if let Some(c) = color(&p) {
            col = c;
            any = true;
        } else if p == "solid" {
            any = true;
        } else if matches!(p.as_str(), "dashed" | "dotted" | "double") {
            solid = false;
            any = true;
        } else {
            return false;
        }
    }
    if !any {
        return false;
    }
    sides_mut(n)[i] = BorderSide {
        width,
        color: Some(col),
    };
    set_border_space(n, Some(i), width);
    solid
}

const SIDES: [&str; 4] = ["Top", "Right", "Bottom", "Left"];

/// The side a `border<Side><suffix>` property names, if it does.
fn side_of(k: &str, prefix: &str, suffix: &str) -> Option<usize> {
    let rest = k.strip_prefix(prefix)?.strip_suffix(suffix)?;
    SIDES.iter().position(|s| *s == rest)
}

fn align_of(v: &str) -> Option<Align> {
    Some(match v.trim() {
        "flex-start" | "start" | "left" | "top" | "normal" | "stretch" | "baseline" => Align::Start,
        "center" => Align::Center,
        "flex-end" | "end" | "right" | "bottom" => Align::End,
        _ => return None,
    })
}

fn rough_mut(n: &mut Node) -> &mut Rough {
    let seed = (n.id.0 as u32).wrapping_mul(2_654_435_761).max(1);
    n.style.rough.get_or_insert(Rough {
        seed,
        ..Rough::default()
    })
}

/// Apply styles to a node. Layout-dependent properties use `ctx`; text properties apply to
/// text and sticky nodes; box properties to everything else.
pub fn apply(n: &mut Node, styles: &[(String, String)], ctx: &Context) -> Applied {
    let mut a = Applied::default();
    // Font size first: `em` values elsewhere depend on it.
    if let Some((_, v)) = styles.iter().rev().find(|(k, _)| k == "fontSize") {
        let cur = n.kind.font_size().unwrap_or(16.0);
        if let Some(px) = length(v, cur).filter(|p| *p > 0.0) {
            match &mut n.kind {
                NodeKind::Text { font_size, .. } | NodeKind::Sticky { font_size, .. } => {
                    *font_size = px
                }
                _ => {}
            }
        }
    }
    let is_text = n.kind.text().is_some();
    let fs = n.kind.font_size().unwrap_or(16.0);
    // Resolved after the loop, whatever order the properties came in.
    let (mut translate, mut radius_pct) = (None, None);
    let position = styles
        .iter()
        .rev()
        .find(|(k, _)| k == "position")
        .map(|(_, v)| v.trim().to_ascii_lowercase());
    for (k, v) in styles {
        let v = v.trim();
        let lower = v.to_ascii_lowercase();
        let ok = match k.as_str() {
            "fontSize" => true, // applied above; inherited by text children
            "width" => {
                set_size(n, 0, v, ctx, &mut a, k);
                true
            }
            "height" => {
                set_size(n, 1, v, ctx, &mut a, k);
                true
            }
            "left" | "x" | "top" | "y" | "right" | "bottom" => {
                let i = match k.as_str() {
                    "top" | "y" => 0,
                    "right" => 1,
                    "bottom" => 2,
                    _ => 3,
                };
                match dim(v, fs) {
                    Some(d) => {
                        if ctx.parent_layout.is_some() {
                            n.layout_child.inset[i] = (d != Dim::Auto).then_some(d);
                        }
                        if let Dim::Px(px) = d {
                            match i {
                                0 => n.rect.y = px,
                                1 => a.right = Some(px),
                                2 => a.bottom = Some(px),
                                _ => n.rect.x = px,
                            }
                        }
                        // Without a layout parent, only pixel offsets can be honored.
                        ctx.parent_layout.is_some() || matches!(d, Dim::Px(_))
                    }
                    None => false,
                }
            }
            "position" => {
                match lower.as_str() {
                    "absolute" | "fixed" => n.layout_child.absolute = true,
                    _ => n.layout_child.absolute = false,
                }
                // The canvas lays both out as CSS would at scroll 0; the preview does the rest.
                n.edit_proto(|p| {
                    p.position = match lower.as_str() {
                        "fixed" => ScrollPosition::Fixed,
                        "sticky" => ScrollPosition::Sticky,
                        _ => ScrollPosition::Scrolls,
                    }
                });
                true
            }
            "display" => match lower.as_str() {
                "none" => {
                    n.visible = false;
                    true
                }
                "flex" | "inline-flex" if n.is_frame() => {
                    n.visible = true;
                    layout_mut(n).mode = LayoutMode::Flex;
                    true
                }
                "grid" | "inline-grid" if n.is_frame() => {
                    n.visible = true;
                    layout_mut(n).mode = LayoutMode::Grid;
                    true
                }
                "block" | "inline-block" | "inline" | "flow-root" if n.is_frame() => {
                    n.visible = true;
                    layout_mut(n).mode = LayoutMode::Block;
                    true
                }
                "block" | "inline-block" | "inline" | "flow-root" | "flex" | "inline-flex" => {
                    n.visible = true;
                    true
                }
                _ => false,
            },
            "visibility" => {
                n.visible = lower != "hidden";
                true
            }
            "flexDirection" if n.is_frame() => {
                layout_mut(n).direction = if lower.starts_with("column") {
                    Direction::Vertical
                } else {
                    Direction::Horizontal
                };
                !lower.ends_with("-reverse")
            }
            "gridTemplateColumns" | "gridTemplateRows" if n.is_frame() => {
                if lower == "none" {
                    n.grid = None;
                    true
                } else if crate::layout::parse_tracks(&lower).is_some() {
                    let g = n.grid.get_or_insert_with(GridSpec::default);
                    if k == "gridTemplateColumns" {
                        g.columns = lower.clone();
                    } else {
                        g.rows = lower.clone();
                    }
                    true
                } else {
                    false
                }
            }
            "gridColumn" | "gridRow" => match grid_line(v) {
                Some(line) => {
                    if k == "gridColumn" {
                        n.layout_child.grid_column = line;
                    } else {
                        n.layout_child.grid_row = line;
                    }
                    true
                }
                None => false,
            },
            "flexWrap" if n.is_frame() => {
                layout_mut(n).wrap = lower.starts_with("wrap");
                true
            }
            "gap" if n.is_frame() => {
                let parts = split_ws(v);
                match (
                    parts.first().and_then(|p| length(p, fs)),
                    parts.get(1).and_then(|p| length(p, fs)),
                ) {
                    (Some(r), c) => {
                        let l = layout_mut(n);
                        // CSS: `gap: row column`. Main-axis gap is the column gap in a row.
                        let c = c.unwrap_or(r);
                        if l.direction == Direction::Horizontal {
                            l.gap = c;
                            l.cross_gap = r;
                        } else {
                            l.gap = r;
                            l.cross_gap = c;
                        }
                        true
                    }
                    _ => false,
                }
            }
            "rowGap" | "columnGap" if n.is_frame() => match length(v, fs) {
                Some(g) => {
                    let l = layout_mut(n);
                    let main_is_row = l.direction == Direction::Horizontal;
                    if (k == "columnGap") == main_is_row {
                        l.gap = g;
                    } else {
                        l.cross_gap = g;
                    }
                    true
                }
                None => false,
            },
            "padding" if n.is_frame() => match four(v, fs) {
                Some(p) => {
                    layout_mut(n).padding = p;
                    true
                }
                None => false,
            },
            "paddingTop" | "paddingRight" | "paddingBottom" | "paddingLeft" if n.is_frame() => {
                match length(v, fs) {
                    Some(p) => {
                        let i = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"]
                            .iter()
                            .position(|x| x == k)
                            .unwrap();
                        layout_mut(n).padding[i] = p;
                        true
                    }
                    None => false,
                }
            }
            "paddingInline" | "paddingBlock" if n.is_frame() => {
                let parts: Vec<f64> = split_ws(v).iter().filter_map(|p| length(p, fs)).collect();
                match parts.as_slice() {
                    [a0, rest @ ..] => {
                        let b0 = rest.first().copied().unwrap_or(*a0);
                        let l = layout_mut(n);
                        if k == "paddingInline" {
                            l.padding[3] = *a0;
                            l.padding[1] = b0;
                        } else {
                            l.padding[0] = *a0;
                            l.padding[2] = b0;
                        }
                        true
                    }
                    _ => false,
                }
            }
            "justifyContent" if n.is_frame() => {
                let l = layout_mut(n);
                if matches!(
                    lower.as_str(),
                    "space-between" | "space-around" | "space-evenly"
                ) {
                    l.gap_auto = true;
                    l.distribute = match lower.as_str() {
                        "space-around" => Distribute::Around,
                        "space-evenly" => Distribute::Evenly,
                        _ => Distribute::Between,
                    };
                    true
                } else if let Some(al) = align_of(&lower) {
                    l.gap_auto = false;
                    l.align_main = al;
                    true
                } else {
                    false
                }
            }
            "alignItems" if n.is_frame() => match align_of(&lower) {
                Some(al) => {
                    layout_mut(n).align_cross = al;
                    true
                }
                None => false,
            },
            "flexGrow" | "flex" => {
                // `flex` is `grow [shrink] [basis]`, or one of two keywords.
                let parts = split_ws(&lower);
                let (grow, shrink, basis) = if k == "flexGrow" {
                    (lower.parse::<f64>().ok(), None, None)
                } else {
                    match lower.as_str() {
                        "none" => (Some(0.0), Some(0.0), Some(Dim::Auto)),
                        "auto" => (Some(1.0), Some(1.0), Some(Dim::Auto)),
                        _ => {
                            let g = parts.first().and_then(|g| g.parse::<f64>().ok());
                            let s = parts.get(1).and_then(|s| s.parse::<f64>().ok());
                            let b = parts.last().filter(|_| parts.len() > 1).and_then(|p| {
                                p.parse::<f64>().is_err().then(|| dim(p, fs)).flatten()
                            });
                            (g, s.or(Some(1.0)), b.or(Some(Dim::Px(0.0))))
                        }
                    }
                };
                match (ctx.parent_layout, grow) {
                    (Some(pl), Some(grow)) => {
                        let ma = if pl.direction == Direction::Horizontal {
                            0
                        } else {
                            1
                        };
                        n.layout_child.grow = Some(grow);
                        if shrink.is_some() {
                            n.layout_child.shrink = shrink;
                        }
                        if basis.is_some() {
                            n.layout_child.basis = basis;
                        }
                        if grow > 0.0 {
                            axis_set(n, ma, Sizing::Fill);
                            if ma == 0
                                && let NodeKind::Text { auto_width, .. } = &mut n.kind
                            {
                                *auto_width = false;
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }
            "flexShrink" => match lower.parse::<f64>() {
                Ok(s) => {
                    n.layout_child.shrink = Some(s.max(0.0));
                    true
                }
                Err(_) => false,
            },
            "flexBasis" => match dim(v, fs) {
                Some(b) => {
                    n.layout_child.basis = Some(b);
                    true
                }
                None => false,
            },
            "alignSelf" => match (ctx.parent_layout, self_align(&lower)) {
                (Some(pl), Some(al)) => {
                    let ca = if pl.direction == Direction::Horizontal {
                        1
                    } else {
                        0
                    };
                    n.layout_child.align_self = al.filter(|a| *a != SelfAlign::Stretch);
                    if al == Some(SelfAlign::Stretch) {
                        axis_set(n, ca, Sizing::Fill);
                        if ca == 0
                            && let NodeKind::Text { auto_width, .. } = &mut n.kind
                        {
                            *auto_width = false;
                        }
                    }
                    true
                }
                _ => false,
            },
            "minWidth" | "minHeight" | "maxWidth" | "maxHeight" => {
                let axis = usize::from(k.ends_with("Height"));
                let slot = if k.starts_with("min") {
                    &mut n.layout_child.min
                } else {
                    &mut n.layout_child.max
                };
                if lower == "none" {
                    slot[axis] = None;
                    true
                } else {
                    match dim(v, fs) {
                        Some(d) => {
                            slot[axis] = (d != Dim::Auto).then_some(d);
                            true
                        }
                        None => false,
                    }
                }
            }
            "margin" => match four_dims(v, fs) {
                Some(m) => {
                    n.layout_child.margin = m.map(Some);
                    true
                }
                None => false,
            },
            "marginInline" | "marginBlock" => {
                let parts: Vec<Dim> = split_ws(v).iter().filter_map(|p| dim(p, fs)).collect();
                match parts.as_slice() {
                    [a0, rest @ ..] => {
                        let b0 = rest.first().copied().unwrap_or(*a0);
                        let m = &mut n.layout_child.margin;
                        if k == "marginInline" {
                            (m[3], m[1]) = (Some(*a0), Some(b0));
                        } else {
                            (m[0], m[2]) = (Some(*a0), Some(b0));
                        }
                        true
                    }
                    _ => false,
                }
            }
            k_ if side_of(k_, "margin", "").is_some() => match dim(v, fs) {
                Some(d) => {
                    n.layout_child.margin[side_of(k_, "margin", "").unwrap()] = Some(d);
                    true
                }
                None => false,
            },
            "aspectRatio" => {
                let parts: Vec<f64> = lower
                    .split('/')
                    .filter_map(|p| p.trim().parse().ok())
                    .collect();
                match parts.as_slice() {
                    [w] if *w > 0.0 => {
                        n.layout_child.aspect = Some(*w);
                        true
                    }
                    [w, h] if *w > 0.0 && *h > 0.0 => {
                        n.layout_child.aspect = Some(w / h);
                        true
                    }
                    _ => false,
                }
            }
            "transform" => {
                translate = translation(v, fs);
                translate.is_some() && ctx.parent_layout.is_some()
            }
            "boxShadow" if !is_text => match shadows(v, fs) {
                Some(s) => {
                    n.style.shadows = s;
                    true
                }
                None => false,
            },
            // Every frame measures itself this way; `content-box` would need a different model.
            "boxSizing" => lower == "border-box",
            "whiteSpace" => {
                if is_text && matches!(lower.as_str(), "nowrap" | "pre") {
                    if let NodeKind::Text { auto_width, .. } = &mut n.kind {
                        *auto_width = true;
                    }
                    axis_set(n, 0, Sizing::Fixed);
                }
                true
            }
            "fontFamily" => !is_text || is_inter(v),
            "textTransform" => {
                if is_text && let Some(t) = n.kind.text_mut() {
                    let mut chars: Vec<char> = t.chars().collect();
                    transform_text(&mut chars, &lower);
                    *t = chars.into_iter().collect();
                }
                matches!(
                    lower.as_str(),
                    "none" | "uppercase" | "lowercase" | "capitalize"
                )
            }
            // Text already breaks inside a word that doesn't fit, and the cursor is for browsers.
            "cursor" | "wordBreak" | "overflowWrap" => true,
            "overflow" if n.is_frame() => {
                if let NodeKind::Frame { clip } = &mut n.kind {
                    *clip = matches!(lower.as_str(), "hidden" | "clip" | "scroll" | "auto");
                }
                let scrolls = matches!(lower.as_str(), "scroll" | "auto");
                n.edit_proto(|p| {
                    p.overflow = if scrolls {
                        Overflow::Both
                    } else {
                        Overflow::None
                    }
                });
                true
            }
            "overflowX" | "overflowY" if n.is_frame() => {
                let scrolls = matches!(lower.as_str(), "scroll" | "auto");
                if let NodeKind::Frame { clip } = &mut n.kind {
                    *clip = *clip || scrolls || matches!(lower.as_str(), "hidden" | "clip");
                }
                let x = k == "overflowX";
                n.edit_proto(|p| {
                    let (mut sx, mut sy) = (p.overflow.x(), p.overflow.y());
                    if x {
                        sx = scrolls;
                    } else {
                        sy = scrolls;
                    }
                    p.overflow = match (sx, sy) {
                        (true, true) => Overflow::Both,
                        (true, false) => Overflow::Horizontal,
                        (false, true) => Overflow::Vertical,
                        (false, false) => Overflow::None,
                    };
                });
                true
            }
            "backgroundColor" | "background" | "backgroundImage" | "fill" if !is_text => {
                apply_background(n, k, v, &mut a)
            }
            "backgroundColor" | "background"
                if is_text && matches!(n.kind, NodeKind::Sticky { .. }) =>
            {
                apply_background(n, k, v, &mut a)
            }
            "backgroundSize" | "objectFit" => {
                let mode = match lower.as_str() {
                    "contain" => ImageMode::Fit,
                    "100% 100%" | "fill" => ImageMode::Stretch,
                    "auto" | "none" => ImageMode::Tile,
                    _ => ImageMode::Fill,
                };
                if let Some((_, m)) = &mut a.image {
                    *m = mode;
                }
                for p in n.style.fills.iter_mut() {
                    if let Paint::Image { mode: m, .. } = p {
                        *m = mode;
                    }
                }
                true
            }
            // Images always crop around their center.
            "backgroundPosition" | "objectPosition" => {
                matches!(lower.as_str(), "center" | "center center" | "50% 50%")
            }
            "backgroundRepeat" => true,
            "color" if is_text => match color(v) {
                Some(c) => {
                    if matches!(n.kind, NodeKind::Sticky { .. }) {
                        // Sticky text color lives in the spans; the fill is the paper.
                        set_text_flag(n, |s| s.color = Some(c));
                    } else {
                        n.style.set_fill(Some(c));
                    }
                    true
                }
                None => false,
            },
            "color" => true, // inherited by text children; meaningless on boxes
            "fontWeight" if is_text => match font_weight(v) {
                Some(w) => {
                    n.text_style.weight = Some(w);
                    n.text_style.bold = w >= 600;
                    true
                }
                None => false,
            },
            "fontStyle" if is_text => {
                n.text_style.italic = lower == "italic" || lower == "oblique";
                true
            }
            "lineHeight" if is_text => {
                let lh = if lower == "normal" {
                    None
                } else if let Some(p) = percent(v) {
                    Some(p)
                } else if v.ends_with("px") || v.ends_with("em") || v.ends_with("rem") {
                    length(v, fs).map(|px| px / fs)
                } else {
                    v.parse::<f64>().ok()
                };
                n.text_style.line_height = lh.filter(|x| *x > 0.0);
                lh.is_some() || lower == "normal"
            }
            "letterSpacing" if is_text => {
                match length(v, fs).or((lower == "normal").then_some(0.0)) {
                    Some(ls) => {
                        n.text_style.letter_spacing = ls;
                        true
                    }
                    None => false,
                }
            }
            "textAlign" if is_text => {
                let al = match lower.as_str() {
                    "center" => TextAlign::Center,
                    "right" | "end" => TextAlign::Right,
                    _ => TextAlign::Left,
                };
                if let NodeKind::Text { align, .. } = &mut n.kind {
                    *align = al;
                }
                true
            }
            "textDecoration" | "textDecorationLine" if is_text => {
                let u = lower.contains("underline");
                let s = lower.contains("line-through");
                set_text_flag(n, |sp| {
                    sp.underline = Some(u);
                    sp.strike = Some(s);
                });
                true
            }
            "fontWeight" | "fontStyle" | "lineHeight" | "letterSpacing" | "textAlign"
            | "textDecoration" => true, // inherited by text children
            "border" => {
                let ok = apply_border(n, v, fs);
                let width = if n.style.stroke.is_some() {
                    n.style.stroke_width
                } else {
                    0.0
                };
                n.style.borders = None;
                set_border_space(n, None, width);
                ok
            }
            "outline" => apply_border(n, v, fs),
            k_ if side_of(k_, "border", "").is_some() => {
                apply_side(n, side_of(k_, "border", "").unwrap(), v, fs)
            }
            k_ if side_of(k_, "border", "Width").is_some() => match length(v, fs) {
                Some(w) => {
                    let i = side_of(k_, "border", "Width").unwrap();
                    sides_mut(n)[i].width = w;
                    set_border_space(n, Some(i), w);
                    true
                }
                None => false,
            },
            k_ if side_of(k_, "border", "Color").is_some() => match color(v) {
                Some(c) => {
                    sides_mut(n)[side_of(k_, "border", "Color").unwrap()].color = Some(c);
                    true
                }
                None => false,
            },
            "borderWidth" if split_ws(v).len() > 1 => match four(v, fs) {
                Some(w) => {
                    let sides = sides_mut(n);
                    for i in 0..4 {
                        sides[i].width = w[i];
                    }
                    for (i, width) in w.iter().enumerate() {
                        set_border_space(n, Some(i), *width);
                    }
                    true
                }
                None => false,
            },
            "borderColor" | "stroke" => match color(v) {
                Some(c) => {
                    n.style.stroke = (c.a > 0).then_some(c);
                    if n.style.stroke_width <= 0.0 {
                        n.style.stroke_width = 1.0;
                    }
                    true
                }
                None => false,
            },
            "borderWidth" | "strokeWidth" => match length(v, fs) {
                Some(w) => {
                    n.style.stroke_width = w;
                    if k == "borderWidth" {
                        n.style.stroke_align = crate::model::StrokeAlign::Inside;
                        n.style.borders = None;
                        set_border_space(n, None, w);
                    }
                    if w > 0.0 && n.style.stroke.is_none() {
                        n.style.stroke = Some(Color::INK);
                    }
                    true
                }
                None => false,
            },
            "borderStyle" | "strokeStyle" => {
                set_dash(n, &lower);
                true
            }
            "strokeDasharray" => {
                n.style.dash = split_ws(&v.replace(',', " "))
                    .iter()
                    .filter_map(|p| length(p, fs))
                    .collect();
                true
            }
            "borderRadius" => match four(v, fs) {
                Some(r) if r.iter().all(|x| *x == r[0]) => {
                    n.style.corner_radius = r[0];
                    n.style.radii = None;
                    true
                }
                Some(r) => {
                    n.style.radii = Some(r);
                    true
                }
                None => match percent(v) {
                    Some(p) => {
                        radius_pct = Some(p);
                        true
                    }
                    None => false,
                },
            },
            "borderTopLeftRadius"
            | "borderTopRightRadius"
            | "borderBottomRightRadius"
            | "borderBottomLeftRadius" => match length(v, fs) {
                Some(r) => {
                    let i = [
                        "borderTopLeftRadius",
                        "borderTopRightRadius",
                        "borderBottomRightRadius",
                        "borderBottomLeftRadius",
                    ]
                    .iter()
                    .position(|x| x == k)
                    .unwrap();
                    let mut rr = n.style.corner_radii();
                    rr[i] = r;
                    n.style.radii = Some(rr);
                    true
                }
                None => false,
            },
            "opacity" => match v.parse::<f64>().ok().or(percent(v)) {
                Some(o) => {
                    n.style.opacity = o.clamp(0.0, 1.0);
                    true
                }
                None => false,
            },
            "mixBlendMode" => match BlendMode::ALL.iter().find(|b| b.css() == lower) {
                Some(b) => {
                    n.style.blend = *b;
                    true
                }
                None => false,
            },
            // Excalidraw-style hand-drawn rendering (Sketch extensions).
            "sketch" | "handDrawn" => {
                if matches!(lower.as_str(), "true" | "1" | "on" | "yes") {
                    rough_mut(n);
                } else {
                    n.style.rough = None;
                }
                true
            }
            "roughness" | "sloppiness" => {
                let r = match lower.as_str() {
                    "architect" => Some(0.0),
                    "artist" => Some(1.0),
                    "cartoonist" => Some(2.0),
                    _ => v.parse::<f64>().ok(),
                };
                match r {
                    Some(r) => {
                        rough_mut(n).roughness = r.clamp(0.0, 4.0);
                        true
                    }
                    None => false,
                }
            }
            "fillStyle" => {
                let fsty = match lower.as_str() {
                    "hachure" => Some(FillStyle::Hachure),
                    "cross-hatch" | "crosshatch" => Some(FillStyle::CrossHatch),
                    "solid" => Some(FillStyle::Solid),
                    "zigzag" => Some(FillStyle::Zigzag),
                    "dots" => Some(FillStyle::Dots),
                    _ => None,
                };
                match fsty {
                    Some(f) => {
                        rough_mut(n).fill_style = f;
                        true
                    }
                    None => false,
                }
            }
            "bowing" => match v.parse::<f64>() {
                Ok(b) => {
                    rough_mut(n).bowing = b;
                    true
                }
                Err(_) => false,
            },
            "seed" => match v.parse::<u32>() {
                Ok(s) => {
                    rough_mut(n).seed = s.max(1);
                    true
                }
                Err(_) => false,
            },
            _ => false,
        };
        if !ok && !a.ignored.contains(k) {
            a.ignored.push(k.clone());
        }
    }
    if let Some(p) = radius_pct {
        // 50% makes a circle or pill whatever the final size turns out to be.
        n.style.corner_radius = if p >= 0.5 {
            9999.0
        } else {
            n.rect.w.min(n.rect.h) * p
        };
        n.style.radii = None;
    }
    if ctx.parent_layout.is_some() {
        let lc = &mut n.layout_child;
        if !lc.absolute {
            // Offsets only move positioned boxes: `relative` shifts one, and otherwise they do
            // nothing, as in CSS. Either way they aren't absolute offsets.
            let offset = |d: Option<Dim>| match d {
                Some(Dim::Px(v)) => v,
                _ => 0.0,
            };
            let inset = std::mem::take(&mut lc.inset);
            if position.as_deref() == Some("relative") {
                let x = offset(inset[3]) - offset(inset[1]);
                let y = offset(inset[0]) - offset(inset[2]);
                lc.shift = [x, y];
            }
        }
        if let Some(t) = translate {
            let base = if position.as_deref() == Some("relative") {
                lc.shift
            } else {
                [0.0; 2]
            };
            lc.shift = [base[0] + t[0], base[1] + t[1]];
        }
    }
    a
}

fn set_dash(n: &mut Node, style: &str) {
    let w = n.style.stroke_width.max(1.0);
    n.style.dash = match style {
        "dashed" => vec![w * 4.0, w * 3.0],
        "dotted" => vec![w, w * 2.0],
        _ => Vec::new(),
    };
}

fn apply_border(n: &mut Node, v: &str, fs: f64) -> bool {
    let lower = v.to_ascii_lowercase();
    if lower == "none" || lower == "0" {
        n.style.stroke = None;
        return true;
    }
    let mut any = false;
    let mut style = "solid".to_string();
    for p in split_ws(v) {
        if let Some(w) = length(&p, fs) {
            n.style.stroke_width = w;
            any = true;
        } else if let Some(c) = color(&p) {
            n.style.stroke = (c.a > 0).then_some(c);
            any = true;
        } else if matches!(p.as_str(), "solid" | "dashed" | "dotted" | "double") {
            style = p;
            any = true;
        }
    }
    if any {
        if n.style.stroke.is_none() && !lower.contains("transparent") {
            n.style.stroke = Some(Color::INK);
        }
        n.style.stroke_align = crate::model::StrokeAlign::Inside;
        set_dash(n, &style);
    }
    any
}

fn apply_background(n: &mut Node, k: &str, v: &str, a: &mut Applied) -> bool {
    let lower = v.to_ascii_lowercase();
    if lower == "none" || lower == "transparent" {
        if k != "backgroundImage" {
            n.style.fills.clear();
        } else {
            n.style.fills.retain(|p| matches!(p, Paint::Solid { .. }));
        }
        return true;
    }
    // Layers: the first CSS layer is on top; ours are bottom to top.
    let mut fills = Vec::new();
    for layer in split_top(v, ',').iter().rev() {
        if let Some(c) = color(layer) {
            fills.push(Paint::solid(c));
        } else if let Some(g) = gradient(layer) {
            fills.push(g);
        } else if let Some(u) = url(layer) {
            a.image = Some((u, ImageMode::Fill));
        } else if let Some(c) = split_ws(layer).iter().find_map(|p| color(p)) {
            // `background: #fff no-repeat` and similar shorthands.
            fills.push(Paint::solid(c));
        } else {
            return false;
        }
    }
    if k == "backgroundImage" {
        // Keep the solid background color below the image/gradient layers.
        n.style.fills.retain(|p| matches!(p, Paint::Solid { .. }));
        n.style.fills.extend(fills);
    } else {
        n.style.fills = fills;
    }
    true
}

// ------------------------------------------------------------------------------ read back

fn px(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}px", r as i64)
    } else {
        format!("{r}px")
    }
}

fn align_css(a: Align) -> &'static str {
    match a {
        Align::Start => "flex-start",
        Align::Center => "center",
        Align::End => "flex-end",
    }
}

/// The node's styles as camelCase CSS (what `get_computed_styles` returns).
pub fn computed(n: &Node, parent_layout: Option<&AutoLayout>, is_root: bool) -> Map<String, Value> {
    let mut m = Map::new();
    let mut put = |k: &str, v: String| {
        m.insert(k.into(), Value::String(v));
    };
    let in_flow = parent_layout.is_some() && !n.layout_child.absolute;
    if !in_flow && !is_root {
        put("position", "absolute".into());
    }
    if !in_flow {
        put("left", px(n.rect.x));
        put("top", px(n.rect.y));
    }
    let lc = &n.layout_child;
    let dim_css = |d: Dim| match d {
        Dim::Px(v) => px(v),
        Dim::Pct(p) => format!("{}%", (p * 10000.0).round() / 100.0),
        Dim::Auto => "auto".to_string(),
    };
    let size_css = |axis: usize, v: f64| match (lc.percent[axis], lc.sizing[axis]) {
        (Some(p), _) => dim_css(Dim::Pct(p)),
        (None, Sizing::Hug) => "fit-content".to_string(),
        (None, Sizing::Fill) if parent_layout.is_some() => "100%".to_string(),
        _ => px(v),
    };
    if lc.absolute {
        for (i, side) in ["top", "right", "bottom", "left"].iter().enumerate() {
            if let Some(d) = lc.inset[i] {
                put(side, dim_css(d));
            }
        }
    }
    if lc.shift != [0.0; 2] {
        put(
            "transform",
            format!("translate({}, {})", px(lc.shift[0]), px(lc.shift[1])),
        );
    }
    for (axis, (min, max)) in [
        ("Width", (lc.min[0], lc.max[0])),
        ("Height", (lc.min[1], lc.max[1])),
    ] {
        if let Some(d) = min {
            put(&format!("min{axis}"), dim_css(d));
        }
        if let Some(d) = max {
            put(&format!("max{axis}"), dim_css(d));
        }
    }
    if lc.margin.iter().any(Option::is_some) {
        let m = lc.margin.map(|d| dim_css(d.unwrap_or(Dim::Px(0.0))));
        put("margin", m.join(" "));
    }
    if let Some(a) = lc.aspect {
        put("aspectRatio", format!("{}", (a * 1000.0).round() / 1000.0));
    }
    let grid_css = |g: [GridPos; 2]| {
        let one = |p: GridPos| match p {
            GridPos::Auto => "auto".to_string(),
            GridPos::Line(i) => i.to_string(),
            GridPos::Span(n) => format!("span {n}"),
        };
        format!("{} / {}", one(g[0]), one(g[1]))
    };
    if lc.grid_column != [GridPos::Auto; 2] {
        put("gridColumn", grid_css(lc.grid_column));
    }
    if lc.grid_row != [GridPos::Auto; 2] {
        put("gridRow", grid_css(lc.grid_row));
    }
    if let NodeKind::Text { auto_width, .. } = &n.kind {
        if *auto_width {
            put("width", "fit-content".into());
        } else {
            put("width", size_css(0, n.rect.w));
        }
        put("height", "fit-content".into());
    } else {
        put("width", size_css(0, n.rect.w));
        put("height", size_css(1, n.rect.h));
    }
    if let Some(pl) = parent_layout
        && in_flow
    {
        let ma = if pl.direction == Direction::Horizontal {
            0
        } else {
            1
        };
        if n.layout_child.sizing[ma] == Sizing::Fill || lc.grow.is_some() {
            put("flexGrow", format!("{}", lc.grow.unwrap_or(1.0)));
        }
        if n.layout_child.sizing[1 - ma] == Sizing::Fill {
            put("alignSelf", "stretch".into());
        }
        if let Some(a) = lc.align_self {
            put(
                "alignSelf",
                match a {
                    SelfAlign::Start => "flex-start",
                    SelfAlign::Center => "center",
                    SelfAlign::End => "flex-end",
                    SelfAlign::Stretch => "stretch",
                }
                .into(),
            );
        }
        put("flexShrink", format!("{}", lc.shrink.unwrap_or(0.0)));
        if let Some(b) = lc.basis {
            put("flexBasis", dim_css(b));
        }
    }
    if let Some(l) = &n.layout {
        put(
            "display",
            match l.mode {
                LayoutMode::Flex => "flex",
                LayoutMode::Block => "block",
                LayoutMode::Grid => "grid",
            }
            .into(),
        );
        if l.mode == LayoutMode::Flex {
            put(
                "flexDirection",
                if l.direction == Direction::Vertical {
                    "column"
                } else {
                    "row"
                }
                .into(),
            );
            if l.wrap {
                put("flexWrap", "wrap".into());
            }
            if l.gap_auto {
                put(
                    "justifyContent",
                    match l.distribute {
                        Distribute::Between => "space-between",
                        Distribute::Around => "space-around",
                        Distribute::Evenly => "space-evenly",
                    }
                    .into(),
                );
            } else {
                put("justifyContent", align_css(l.align_main).into());
                put("gap", px(l.gap));
            }
            put("alignItems", align_css(l.align_cross).into());
        }
        if l.mode == LayoutMode::Grid {
            if let Some(g) = &n.grid {
                if !g.columns.is_empty() {
                    put("gridTemplateColumns", g.columns.clone());
                }
                if !g.rows.is_empty() {
                    put("gridTemplateRows", g.rows.clone());
                }
            }
            put("gap", px(l.gap));
        }
        let p = l.padding;
        if p.iter().any(|x| *x != 0.0) {
            put(
                "padding",
                if p.iter().all(|x| *x == p[0]) {
                    px(p[0])
                } else {
                    format!("{} {} {} {}", px(p[0]), px(p[1]), px(p[2]), px(p[3]))
                },
            );
        }
    }
    if let NodeKind::Frame { clip: true } = n.kind {
        let ov = n.proto.as_ref().map_or(Overflow::None, |p| p.overflow);
        put(
            "overflow",
            if ov == Overflow::None {
                "hidden"
            } else {
                "auto"
            }
            .into(),
        );
    }
    if !n.visible {
        put("display", "none".into());
    }
    match &n.kind {
        NodeKind::Text {
            font_size, align, ..
        } => {
            put("fontFamily", "Inter".into());
            put("fontSize", px(*font_size));
            let weight = n
                .text_style
                .weight
                .unwrap_or(if n.text_style.bold { 700 } else { 400 });
            put("fontWeight", weight.to_string());
            if n.text_style.italic {
                put("fontStyle", "italic".into());
            }
            if let Some(lh) = n.text_style.line_height {
                put("lineHeight", px(lh * font_size));
            }
            if n.text_style.letter_spacing != 0.0 {
                put(
                    "letterSpacing",
                    format!(
                        "{}em",
                        (n.text_style.letter_spacing / font_size * 1000.0).round() / 1000.0
                    ),
                );
            }
            put(
                "textAlign",
                match align {
                    TextAlign::Left => "left",
                    TextAlign::Center => "center",
                    TextAlign::Right => "right",
                }
                .into(),
            );
            if let Some(c) = n.style.fill() {
                put("color", color_css(c));
            }
        }
        _ => {
            let layers: Vec<String> = n
                .style
                .fills
                .iter()
                .rev()
                .filter_map(|p| match p {
                    Paint::Solid { color } => Some(color_css(*color)),
                    Paint::Image { asset, .. } => Some(format!("url(paper-asset://{asset})")),
                    g => gradient_css(g),
                })
                .collect();
            if layers.len() == 1 && matches!(n.style.fills[0], Paint::Solid { .. }) {
                put("backgroundColor", layers[0].clone());
            } else if !layers.is_empty() {
                put("background", layers.join(", "));
            }
        }
    }
    if let Some(sides) = &n.style.borders {
        for (name, side) in SIDES.iter().zip(sides) {
            if let (true, Some(c)) = (side.width > 0.0, side.color.or(n.style.stroke)) {
                put(
                    &format!("border{name}"),
                    format!("{} solid {}", px(side.width), color_css(c)),
                );
            }
        }
    } else if let Some(s) = n.style.stroke
        && n.style.stroke_width > 0.0
    {
        let kind = if n.style.dash.is_empty() {
            "solid"
        } else if n.style.dash[0] <= n.style.stroke_width * 1.5 {
            "dotted"
        } else {
            "dashed"
        };
        put(
            "border",
            format!("{} {kind} {}", px(n.style.stroke_width), color_css(s)),
        );
    }
    if let Some(r) = n.style.radii {
        put(
            "borderRadius",
            format!("{} {} {} {}", px(r[0]), px(r[1]), px(r[2]), px(r[3])),
        );
    } else if n.style.corner_radius > 0.0 {
        put("borderRadius", px(n.style.corner_radius));
    }
    if !n.style.shadows.is_empty() {
        let layers: Vec<String> = n
            .style
            .shadows
            .iter()
            .map(|s| {
                format!(
                    "{}{} {} {} {} {}",
                    if s.inset { "inset " } else { "" },
                    px(s.x),
                    px(s.y),
                    px(s.blur),
                    px(s.spread),
                    color_css(s.color)
                )
            })
            .collect();
        put("boxShadow", layers.join(", "));
    }
    if n.style.opacity < 1.0 {
        put(
            "opacity",
            format!("{}", (n.style.opacity * 1000.0).round() / 1000.0),
        );
    }
    if n.style.blend != BlendMode::Normal {
        put("mixBlendMode", n.style.blend.css().into());
    }
    if let Some(r) = n.style.rough {
        put("sketch", "true".into());
        put("roughness", format!("{}", r.roughness));
        put(
            "fillStyle",
            match r.fill_style {
                FillStyle::Hachure => "hachure",
                FillStyle::CrossHatch => "cross-hatch",
                FillStyle::Solid => "solid",
                FillStyle::Zigzag => "zigzag",
                FillStyle::Dots => "dots",
            }
            .into(),
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::NodeId;

    #[test]
    fn colors() {
        assert_eq!(color("#fff"), Some(Color::WHITE));
        assert_eq!(
            color("#11223380"),
            Some(Color::rgba(0x11, 0x22, 0x33, 0x80))
        );
        assert_eq!(color("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            color("rgba(0 0 255 / 50%)"),
            Some(Color::rgba(0, 0, 255, 128))
        );
        assert_eq!(color("hsl(120, 100%, 25%)"), Some(Color::rgb(0, 128, 0)));
        assert_eq!(color("RebeccaPurple"), Some(Color::rgb(0x66, 0x33, 0x99)));
        let w = color("oklch(1 0 0)").unwrap();
        assert!(w.r >= 254 && w.g >= 254 && w.b >= 254, "{w:?}");
        assert_eq!(color("nonsense"), None);
    }

    #[test]
    fn inline_and_json_styles() {
        let s = parse_inline("background-color: rgb(1, 2, 3); padding:4px 8px ;font-family:'A; B'");
        assert_eq!(s[0], ("backgroundColor".into(), "rgb(1, 2, 3)".into()));
        assert_eq!(s[1], ("padding".into(), "4px 8px".into()));
        assert_eq!(s[2].1, "'A; B'");
        let j = from_json(&serde_json::json!({"width": 10, "opacity": 0.5})).unwrap();
        assert!(j.contains(&("width".into(), "10px".into())));
        assert!(j.contains(&("opacity".into(), "0.5".into())));
    }

    #[test]
    fn frame_styles_map_to_auto_layout() {
        let mut n = Node::new(
            NodeId(1),
            NodeKind::Frame { clip: false },
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let a = apply(
            &mut n,
            &parse_inline(
                "display:flex; flex-direction:column; gap:12px; padding:8px 16px; \
                 justify-content:center; align-items:flex-end; width:320px; height:fit-content; \
                 background:#ff0000; border:2px dashed #000; border-radius:12px; box-shadow: 0 0 1px red",
            ),
            &Context::default(),
        );
        let l = n.layout.unwrap();
        assert_eq!(l.direction, Direction::Vertical);
        assert_eq!(l.gap, 12.0);
        assert_eq!(l.padding, [8.0, 16.0, 8.0, 16.0]);
        assert_eq!((l.align_main, l.align_cross), (Align::Center, Align::End));
        assert_eq!(n.rect.w, 320.0);
        assert_eq!(n.layout_child.sizing[1], Sizing::Hug);
        assert_eq!(n.style.fill(), Some(Color::rgb(255, 0, 0)));
        assert_eq!(n.style.stroke_width, 2.0);
        assert!(!n.style.dash.is_empty());
        assert_eq!(n.style.corner_radius, 12.0);
        assert!(a.ignored.is_empty(), "{:?}", a.ignored);
        assert_eq!(n.style.shadows.len(), 1);
        let back = computed(&n, None, true);
        assert_eq!(back["flexDirection"], "column");
        assert_eq!(back["borderRadius"], "12px");
    }

    #[test]
    fn gradients_and_sketch() {
        let Some(Paint::Linear {
            stops, from, to, ..
        }) = gradient("linear-gradient(90deg, red, blue)")
        else {
            panic!()
        };
        assert_eq!(stops.len(), 2);
        assert!((from.x - 0.0).abs() < 1e-9 && (to.x - 1.0).abs() < 1e-9);
        let mut n = Node::new(NodeId(3), NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0));
        apply(
            &mut n,
            &parse_inline("roughness: 2; fill-style: cross-hatch"),
            &Context::default(),
        );
        let r = n.style.rough.unwrap();
        assert_eq!((r.roughness, r.fill_style), (2.0, FillStyle::CrossHatch));
    }
}
