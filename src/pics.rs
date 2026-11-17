//! The pictures of the design check: cards side by side (Design · Android · iOS), each with its title, the screenshot
//! with rounded corners, the measured elements outlined and named, and the measured distances with their values; a
//! title row with chips above and a footer with the numbers below. Text is SF Pro (Latin and Greek) when the system
//! has it, else Arial, else a 5×7 pixel font.
use image::{imageops, Rgb, RgbImage};

pub const WHITE: Rgb<u8> = Rgb([255, 255, 255]);
pub const PAGE: Rgb<u8> = Rgb([243, 244, 246]);
pub const BORDER: Rgb<u8> = Rgb([226, 232, 240]);
pub const INK: Rgb<u8> = Rgb([17, 24, 39]);
pub const MUTED: Rgb<u8> = Rgb([100, 116, 139]);
pub const DESIGN: Rgb<u8> = Rgb([124, 58, 237]);
pub const ANDROID: Rgb<u8> = Rgb([22, 163, 74]);
pub const IOS: Rgb<u8> = Rgb([37, 99, 235]);
pub const BAD: Rgb<u8> = Rgb([220, 38, 38]);
pub const WARN: Rgb<u8> = Rgb([217, 119, 6]);
pub const QUIET: Rgb<u8> = Rgb([148, 163, 184]);

/// A soft tint of a colour (chips' backgrounds).
pub fn tint(c: Rgb<u8>, a: f32) -> Rgb<u8> {
    Rgb([0, 1, 2].map(|i| (255.0 * (1.0 - a) + c[i] as f32 * a).round() as u8))
}

// ---------------------------------------------------------------------------------------------------------------
// Text

fn load(path: &str, weight: f32) -> Option<ab_glyph::FontVec> {
    use ab_glyph::VariableFont;
    let mut f = ab_glyph::FontVec::try_from_vec(std::fs::read(path).ok()?).ok()?;
    f.set_variation(b"wght", weight);
    Some(f)
}

fn font(bold: bool) -> Option<&'static ab_glyph::FontVec> {
    use std::sync::OnceLock;
    static REG: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();
    static BOLD: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();
    let pick = |w: f32, fallback: &[&str]| {
        load("/System/Library/Fonts/SFNS.ttf", w).or_else(|| fallback.iter().find_map(|p| load(p, w)))
    };
    if bold {
        BOLD.get_or_init(|| pick(640.0, &["/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"])).as_ref()
    } else {
        REG.get_or_init(|| pick(420.0, &["/System/Library/Fonts/Supplemental/Arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"])).as_ref()
    }
}

fn blend(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x as u32 >= img.width() || y as u32 >= img.height() {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = a.min(1.0);
    for i in 0..3 {
        p[i] = (p[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    }
}

/// Draws `s` with its top-left corner at (x, y), `px` tall; returns the width drawn.
pub fn write(img: &mut RgbImage, x: i32, y: i32, s: &str, px: f32, c: Rgb<u8>, bold: bool) -> i32 {
    use ab_glyph::{Font, ScaleFont};
    let Some(f) = font(bold) else {
        let size = (px / 8.0).round().max(1.0) as i32;
        crate::design::pixel_text(img, x, y, s, size, c);
        return s.chars().count() as i32 * 6 * size;
    };
    let sf = f.as_scaled(ab_glyph::PxScale::from(px));
    let mut caret = x as f32;
    let base = y as f32 + sf.ascent();
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in s.chars() {
        let id = f.glyph_id(ch);
        if let Some(p) = prev {
            caret += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(px, ab_glyph::point(caret, base));
        caret += sf.h_advance(id);
        prev = Some(id);
        if let Some(o) = f.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, cov));
        }
    }
    caret.round() as i32 - x
}

pub fn width_of(s: &str, px: f32, bold: bool) -> i32 {
    use ab_glyph::{Font, ScaleFont};
    match font(bold) {
        Some(f) => {
            let sf = f.as_scaled(ab_glyph::PxScale::from(px));
            s.chars().map(|c| sf.h_advance(f.glyph_id(c))).sum::<f32>().round() as i32
        }
        None => s.chars().count() as i32 * 6 * (px / 8.0).round().max(1.0) as i32,
    }
}

/// `s` shortened with "…" to fit `max` px.
pub fn fit(s: &str, px: f32, max: i32, bold: bool) -> String {
    if width_of(s, px, bold) <= max {
        return s.to_string();
    }
    let mut t: Vec<char> = s.chars().collect();
    while !t.is_empty() && width_of(&format!("{}…", t.iter().collect::<String>()), px, bold) > max {
        t.pop();
    }
    format!("{}…", t.iter().collect::<String>())
}

// ---------------------------------------------------------------------------------------------------------------
// Shapes (anti-aliased)

/// Coverage of pixel (px, py) by a rounded rectangle.
fn cover(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (px + 0.5, py + 0.5);
    let qx = (cx - (x + w / 2.0)).abs() - (w / 2.0 - r);
    let qy = (cy - (y + h / 2.0)).abs() - (h / 2.0 - r);
    let d = (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
pub fn round_rect(img: &mut RgbImage, x: i32, y: i32, w: i32, h: i32, r: i32, c: Rgb<u8>, alpha: f32) {
    let (fx, fy, fw, fh, fr) = (x as f32, y as f32, w as f32, h as f32, (r as f32).min(w as f32 / 2.0).min(h as f32 / 2.0));
    for py in y.max(0)..(y + h).min(img.height() as i32) {
        for px in x.max(0)..(x + w).min(img.width() as i32) {
            let a = cover(px as f32, py as f32, fx, fy, fw, fh, fr);
            if a > 0.0 {
                blend(img, px, py, c, a * alpha);
            }
        }
    }
}

/// The outline of a rounded rectangle, `t` px thick.
#[allow(clippy::too_many_arguments)]
pub fn round_frame(img: &mut RgbImage, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, c: Rgb<u8>) {
    let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
    let (ro, ri) = ((r as f32).min(fw / 2.0).min(fh / 2.0), ((r - t) as f32).max(0.0));
    for py in (y - 1).max(0)..(y + h + 1).min(img.height() as i32) {
        for px in (x - 1).max(0)..(x + w + 1).min(img.width() as i32) {
            let outer = cover(px as f32, py as f32, fx, fy, fw, fh, ro);
            let inner = cover(px as f32, py as f32, fx + t as f32, fy + t as f32, fw - 2.0 * t as f32, fh - 2.0 * t as f32, ri);
            let a = (outer - inner).max(0.0);
            if a > 0.0 {
                blend(img, px, py, c, a);
            }
        }
    }
}

/// A soft drop shadow under a card.
fn shadow(img: &mut RgbImage, x: i32, y: i32, w: i32, h: i32, r: i32) {
    for (grow, dy, a) in [(10, 8, 0.025), (6, 5, 0.035), (3, 3, 0.045), (1, 1, 0.05)] {
        round_rect(img, x - grow, y - grow + dy, w + 2 * grow, h + 2 * grow, r + grow, INK, a);
    }
}

/// A pill: text on a rounded colour. Returns (width, height).
pub fn pill(img: &mut RgbImage, x: i32, y: i32, s: &str, px: f32, fg: Rgb<u8>, bg: Rgb<u8>) -> (i32, i32) {
    let (w, h) = (width_of(s, px, true) + (px * 1.1) as i32, (px * 1.6).round() as i32);
    round_rect(img, x, y, w, h, h / 2, bg, 1.0);
    write(img, x + (px * 0.55) as i32, y + ((h as f32 - px * 1.08) / 2.0) as i32, s, px, fg, true);
    (w, h)
}

pub fn pill_width(s: &str, px: f32) -> i32 {
    width_of(s, px, true) + (px * 1.1) as i32
}

fn vline(img: &mut RgbImage, x: i32, y0: i32, y1: i32, t: i32, c: Rgb<u8>) {
    for y in y0.min(y1)..=y0.max(y1) {
        for dx in 0..t {
            blend(img, x + dx - t / 2, y, c, 1.0);
        }
    }
}

fn hline(img: &mut RgbImage, x0: i32, x1: i32, y: i32, t: i32, c: Rgb<u8>) {
    for x in x0.min(x1)..=x0.max(x1) {
        for dy in 0..t {
            blend(img, x, y + dy - t / 2, c, 1.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Cards

/// Something drawn on a card's screenshot, in screenshot pixels.
pub enum Mark {
    /// an element: outlined, its name on a tag above it
    Box { x: i32, y: i32, w: i32, h: i32, label: String, color: Rgb<u8> },
    /// a measured vertical distance at x, from y0 to y1, its value on a pill
    VDim { x: i32, y0: i32, y1: i32, value: String, color: Rgb<u8> },
    /// a measured horizontal distance at y, from x0 to x1
    HDim { x0: i32, x1: i32, y: i32, value: String, color: Rgb<u8> },
    /// a quiet outline (an element without a counterpart)
    Ghost { x: i32, y: i32, w: i32, h: i32 },
}

pub struct Card {
    pub title: String,
    pub subtitle: String,
    pub color: Rgb<u8>,
    pub shot: RgbImage,
    pub marks: Vec<Mark>,
}

/// Cards side by side under a title row (title + chips), with footer lines below.
pub fn compose(cards: &[Card], title: &str, chips: &[(String, Rgb<u8>)], footer: &[(String, Rgb<u8>, bool)], path: &str) -> Result<(), String> {
    let (margin, gap, pad, head, radius) = (40, 28, 18, 96, 22);
    let shot_h = cards.iter().map(|c| c.shot.height() as i32).max().unwrap_or(0);
    let card_w: Vec<i32> = cards.iter().map(|c| c.shot.width() as i32 + 2 * pad).collect();
    let card_h = head + shot_h + pad;
    let title_h = if title.is_empty() { 0 } else { 92 };
    let foot_h = if footer.is_empty() { 0 } else { 28 + footer.len() as i32 * 44 };
    let w = 2 * margin + card_w.iter().sum::<i32>() + gap * (cards.len() as i32 - 1).max(0);
    let h = margin + title_h + card_h + foot_h + margin;
    let mut img = RgbImage::from_pixel(w as u32, h as u32, PAGE);
    // Title row: the finding, then its chips on the right.
    if !title.is_empty() {
        let chips_w: i32 = chips.iter().map(|(s, _)| pill_width(s, 22.0) + 12).sum();
        write(&mut img, margin, margin + 8, &fit(title, 36.0, w - 2 * margin - chips_w - 24, true), 36.0, INK, true);
        let mut cx = w - margin - chips_w;
        for (s, c) in chips {
            cx += pill(&mut img, cx, margin + 10, s, 22.0, *c, tint(*c, 0.14)).0 + 12;
        }
    }
    let top = margin + title_h;
    let mut x = margin;
    for (i, c) in cards.iter().enumerate() {
        let cw = card_w[i];
        shadow(&mut img, x, top, cw, card_h, radius);
        round_rect(&mut img, x, top, cw, card_h, radius, WHITE, 1.0);
        round_frame(&mut img, x, top, cw, card_h, radius, 1, BORDER);
        // Header: a coloured dot and the platform, the detail line under it.
        round_rect(&mut img, x + pad, top + 26, 16, 16, 8, c.color, 1.0);
        write(&mut img, x + pad + 26, top + 16, &fit(&c.title, 30.0, cw - 2 * pad - 30, true), 30.0, INK, true);
        write(&mut img, x + pad, top + 58, &fit(&c.subtitle, 20.0, cw - 2 * pad, false), 20.0, MUTED, false);
        // The screenshot with rounded corners.
        let (sx, sy) = (x + pad, top + head);
        imageops::replace(&mut img, &c.shot, sx as i64, sy as i64);
        let (sw, sh) = (c.shot.width() as i32, c.shot.height() as i32);
        let r = 14.0;
        for py in sy..sy + sh {
            for px in sx..sx + sw {
                let a = cover(px as f32, py as f32, sx as f32, sy as f32, sw as f32, sh as f32, r);
                if a < 1.0 {
                    blend(&mut img, px, py, WHITE, 1.0 - a);
                }
            }
        }
        round_frame(&mut img, sx, sy, sw, sh, 14, 1, BORDER);
        for m in &c.marks {
            draw_mark(&mut img, sx, sy, sw, sh, m);
        }
        x += cw + gap;
    }
    // Footer: the numbers, one line each.
    let mut fy = top + card_h + 24;
    for (s, c, bold) in footer {
        write(&mut img, margin, fy, &fit(s, 24.0, w - 2 * margin, *bold), 24.0, *c, *bold);
        fy += 44;
    }
    img.save(path).map_err(|e| format!("{path}: {e}"))
}

fn draw_mark(img: &mut RgbImage, sx: i32, sy: i32, sw: i32, sh: i32, m: &Mark) {
    // Nothing outside the screenshot: a block that runs past its bottom edge is not drawn.
    let inside = |y: i32, h: i32| y >= -40 && y + h <= sh + 4;
    match m {
        Mark::Box { y, h, .. } | Mark::Ghost { y, h, .. } if !inside(*y, *h) => {}
        Mark::Box { x, y, w, h, label, color } => {
            round_frame(img, sx + x, sy + y, *w, *h, 10, 3, *color);
            if !label.is_empty() {
                let t = fit(label, 18.0, (sw / 2).max(120), true);
                let ph = (18.0 * 1.6) as i32;
                let ty = (sy + y - ph - 4).max(sy + 2);
                pill(img, sx + x, ty, &t, 18.0, WHITE, *color);
            }
        }
        Mark::VDim { x, y0, y1, value, color } => {
            let (ax, a0, a1) = (sx + x, sy + y0, sy + y1);
            vline(img, ax, a0, a1, 3, *color);
            hline(img, ax - 9, ax + 9, a0, 3, *color);
            hline(img, ax - 9, ax + 9, a1, 3, *color);
            let pw = pill_width(value, 22.0);
            pill(img, ax - pw - 12, (a0 + a1) / 2 - 18, value, 22.0, WHITE, *color);
        }
        Mark::HDim { x0, x1, y, value, color } => {
            let (b0, b1, by) = (sx + x0, sx + x1.max(&(x0 + 1)), sy + y);
            hline(img, b0, b1, by, 3, *color);
            vline(img, b0, by - 9, by + 9, 3, *color);
            vline(img, b1, by - 9, by + 9, 3, *color);
            pill(img, b0 + 6, by + 10, value, 22.0, WHITE, *color);
        }
        Mark::Ghost { x, y, w, h } => round_frame(img, sx + x, sy + y, *w, *h, 10, 2, QUIET),
    }
}
