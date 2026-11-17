//! Design check: the app screen against its Figma frame, measured the way a UI tester measures — deterministically, in
//! dp, in well under a second, with no LLM in the loop.
//!
//! 1. Both images are cut into visual blocks: runs of rows that hold content, found from colour edges. Long vertical
//!    edges (card and container sides) are ignored, so the content of a card still splits into its own blocks; rows
//!    closer than `merge_gap` (lines of one text, an icon and its label) form one block.
//! 2. The two block lists are aligned in order: the chain of similar blocks (height, margins, pieces side by side,
//!    colour) whose vertical spacing changes least. Blocks that exist on one side only (an empty state against a design
//!    with data, a status bar) are left out of the chain.
//! 3. Neighbouring matched blocks give gaps; matched blocks give heights, left margins and text colours (CIEDE2000).
//!    Differences above the tolerance become findings tagged [category | confidence], each with a FIGMA | APP crop.
//!
//! With the app tree (qapilot screen) and the Figma metadata (the Figma MCP's get_metadata XML) the blocks also get
//! names, and the layout boxes give the box-to-box gaps that bug reports quote ("tabs → amount 36 dp instead of 32").
use image::{imageops, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug)]
pub struct Opts {
    /// device px per dp (Android density / 160; iPhone screenshots 3)
    pub app_scale: f32,
    /// Figma PNG px per pt (the Figma MCP's get_screenshot is 1)
    pub figma_scale: f32,
    /// dp difference that counts as a finding
    pub tol: f32,
    /// rows closer than this (dp) belong to one block
    pub merge_gap: f32,
    /// a label for the Figma side of the pictures, e.g. the node id
    pub figma_name: String,
    /// the status bar zone (dp): blocks that end above it are not compared (Android vs the iOS bar in Figma)
    pub skip_top: f32,
    /// the Figma frame's name (from the metadata), for the pictures
    pub figma_frame: String,
    /// the app and version ("MyApp 2.3.0 (230)") and the device ("Android · Pixel 9 Pro"), for the pictures
    pub app_label: String,
    pub device_label: String,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { app_scale: 3.0, figma_scale: 1.0, tol: 2.0, merge_gap: 8.0, figma_name: String::new(), skip_top: 40.0, figma_frame: String::new(), app_label: String::new(), device_label: String::new() }
    }
}

/// A named box from the app tree or the Figma metadata, in dp from the top-left corner.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedBox {
    pub name: String,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Block {
    pub n: usize,
    /// ink extent in dp
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    /// dp from the right edge of the screen / frame
    pub margin_right: f32,
    /// rows of content merged into the block (lines of a text)
    pub lines: usize,
    /// pieces side by side (tabs, chips, buttons)
    pub items: usize,
    /// core colour of the text / icons when one colour dominates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// extent of the named layout boxes inside the block (app tree / Figma metadata)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_top: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_bottom: Option<f32>,
    pub label: String,
    #[serde(skip)]
    lab: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub id: String,
    /// Distances · Sizing · Colors · Typography · Layout
    pub category: &'static str,
    /// CERTAIN or UNCERTAIN (a person should confirm)
    pub confidence: &'static str,
    pub title: String,
    pub app: serde_json::Value,
    pub figma: serde_json::Value,
    /// app minus Figma, in dp (or ΔE for colours)
    pub delta: f32,
    /// what was measured: "box" (layout boxes from the app tree / Figma metadata) or "ink" (visible pixels)
    pub basis: &'static str,
    /// block numbers: [app blocks…], [Figma blocks…]
    pub app_blocks: Vec<usize>,
    pub figma_blocks: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub app: serde_json::Value,
    pub figma: serde_json::Value,
    pub app_blocks: Vec<Block>,
    pub figma_blocks: Vec<Block>,
    /// matched blocks: [app, figma, similarity]
    pub matches: Vec<(usize, usize, f32)>,
    pub findings: Vec<Finding>,
    /// blocks on one side only, between matched ones (structure differences, not bugs by themselves)
    pub notes: Vec<String>,
    pub ms: u128,
}

pub struct Pic {
    pub img: RgbImage,
    pub scale: f32,
}

impl Pic {
    pub fn load(path: &str, scale: f32) -> Result<Pic, String> {
        let img = image::open(path).map_err(|e| format!("{path}: {e}"))?.to_rgb8();
        Ok(Pic { img, scale })
    }
    pub fn w(&self) -> usize {
        self.img.width() as usize
    }
    pub fn h(&self) -> usize {
        self.img.height() as usize
    }
    pub fn dp(&self, px: usize) -> f32 {
        px as f32 / self.scale
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Colour

/// Squared colour distance on a 0–255 scale, weighted roughly like the eye (green most, red least).
fn d2(a: &[u8], b: &[u8]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    ((2 * dr * dr + 4 * dg * dg + 3 * db * db) / 9) as u32
}

fn lab(rgb: [f32; 3]) -> [f32; 3] {
    let lin = |c: f32| {
        let c = c / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference.
pub fn ciede2000(l1: [f32; 3], l2: [f32; 3]) -> f32 {
    use std::f32::consts::PI;
    let (l1_, a1, b1) = (l1[0], l1[1], l1[2]);
    let (l2_, a2, b2) = (l2[0], l2[1], l2[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f32.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h = |b: f32, a: f32| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let v = b.atan2(a).to_degrees();
            if v < 0.0 { v + 360.0 } else { v }
        }
    };
    let h1p = h(b1, a1p);
    let h2p = h(b2, a2p);
    let dlp = l2_ - l1_;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();
    let lpm = (l1_ + l2_) / 2.0;
    let cpm = (c1p + c2p) / 2.0;
    let hpm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * ((hpm - 30.0) * PI / 180.0).cos() + 0.24 * ((2.0 * hpm) * PI / 180.0).cos() + 0.32 * ((3.0 * hpm + 6.0) * PI / 180.0).cos()
        - 0.20 * ((4.0 * hpm - 63.0) * PI / 180.0).cos();
    let dtheta = 30.0 * (-((hpm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cpm.powi(7) / (cpm.powi(7) + 25f32.powi(7))).sqrt();
    let sl = 1.0 + (0.015 * (lpm - 50.0).powi(2)) / (20.0 + (lpm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cpm;
    let sh = 1.0 + 0.015 * cpm * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh)).sqrt()
}

// ---------------------------------------------------------------------------------------------------------------
// Blocks

const EDGE2: u32 = 18 * 18;

/// Edge pixels: the colour changes by more than 18 (of 255) within ~½ dp to the right or below. Smooth gradients and
/// soft shadows stay below that; text, icons, borders and dividers do not.
fn edges(p: &Pic) -> Vec<bool> {
    let (w, h) = (p.w(), p.h());
    let raw = p.img.as_raw();
    let k = ((p.scale * 0.5).round() as usize).max(1);
    let mut e = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let a = &raw[i..i + 3];
            let right = x + k < w && d2(a, &raw[i + k * 3..i + k * 3 + 3]) > EDGE2;
            let below = !right && y + k < h && d2(a, &raw[i + k * w * 3..i + k * w * 3 + 3]) > EDGE2;
            e[y * w + x] = right || below;
        }
    }
    // Long straight vertical edges are the sides of cards and containers, not content.
    let min_len = (48.0 * p.scale) as usize;
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if e[y * w + x] {
                let s = y;
                while y < h && e[y * w + x] {
                    y += 1;
                }
                if y - s >= min_len {
                    for yy in s..y {
                        e[yy * w + x] = false;
                    }
                }
            } else {
                y += 1;
            }
        }
    }
    e
}

/// Runs of true values in `v` separated by at least `sep` falses, dropping runs shorter than `min`: [start, end).
fn runs(v: &[bool], sep: usize, min: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i] {
            let s = i;
            while i < v.len() && v[i] {
                i += 1;
            }
            match out.last_mut() {
                Some(last) if s - last.1 < sep => last.1 = i,
                _ => out.push((s, i)),
            }
        } else {
            i += 1;
        }
    }
    out.retain(|(s, e)| e - s >= min);
    out
}

fn blocks(p: &Pic, merge_gap: f32, boxes: &[NamedBox]) -> Vec<Block> {
    let (w, h) = (p.w(), p.h());
    let e = edges(p);
    let min_count = (p.scale.round() as usize).max(2);
    let rows: Vec<bool> = (0..h).map(|y| e[y * w..(y + 1) * w].iter().filter(|&&b| b).count() >= min_count).collect();
    // Bands: content rows, split by ≥ 1 dp of empty rows; then blocks: bands closer than merge_gap.
    let bands = runs(&rows, (p.scale.round() as usize).max(1), (p.scale * 0.6).round().max(1.0) as usize);
    let mut groups: Vec<(usize, usize, usize)> = Vec::new(); // start, end, bands
    for (s, t) in bands {
        match groups.last_mut() {
            Some(g) if (s - g.1) as f32 / p.scale < merge_gap => {
                g.1 = t;
                g.2 += 1;
            }
            _ => groups.push((s, t, 1)),
        }
    }
    let raw = p.img.as_raw();
    let mut out = Vec::new();
    for (s, t, lines) in groups {
        let cols: Vec<bool> = (0..w).map(|x| (s..t).any(|y| e[y * w + x])).collect();
        let pieces = runs(&cols, (3.0 * p.scale) as usize, 1);
        let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else { continue };
        let (x0, x1) = (first.0, last.1);
        // Background = the commonest colour of the block; the core colour = the pixels farthest from it.
        let mut hist = std::collections::HashMap::<u32, u32>::new();
        for y in s..t {
            for x in x0..x1 {
                let i = (y * w + x) * 3;
                let q = ((raw[i] as u32 >> 3) << 10) | ((raw[i + 1] as u32 >> 3) << 5) | (raw[i + 2] as u32 >> 3);
                *hist.entry(q).or_default() += 1;
            }
        }
        let bgq = hist.into_iter().max_by_key(|(_, n)| *n).map(|(q, _)| q).unwrap_or(0);
        let bg = [(((bgq >> 10) & 31) << 3) as u8 + 4, (((bgq >> 5) & 31) << 3) as u8 + 4, ((bgq & 31) << 3) as u8 + 4];
        let mut fg: Vec<(u32, [u8; 3])> = Vec::new();
        for y in s..t {
            for x in x0..x1 {
                let i = (y * w + x) * 3;
                let px = [raw[i], raw[i + 1], raw[i + 2]];
                let d = d2(&px, &bg);
                if d > 60 * 60 {
                    fg.push((d, px));
                }
            }
        }
        let (color, labv) = if fg.len() >= 12 {
            fg.sort_by_key(|p| std::cmp::Reverse(p.0));
            let core = &fg[..(fg.len() * 2 / 5).max(6)];
            let med = |c: usize| {
                let mut v: Vec<u8> = core.iter().map(|(_, p)| p[c]).collect();
                v.sort_unstable();
                v[v.len() / 2]
            };
            let m = [med(0), med(1), med(2)];
            let close = core.iter().filter(|(_, p)| d2(p, &m) < 30 * 30).count();
            if close * 10 >= core.len() * 7 {
                (Some(format!("#{:02x}{:02x}{:02x}", m[0], m[1], m[2])), Some(lab([m[0] as f32, m[1] as f32, m[2] as f32])))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };
        let (top, bottom) = (p.dp(s), p.dp(t));
        let inside: Vec<&NamedBox> = boxes
            .iter()
            .filter(|b| {
                let c = (b.top + b.bottom) / 2.0;
                c >= top - 3.0 && c <= bottom + 3.0 && (b.bottom - b.top) <= (bottom - top) + 24.0 && b.bottom - b.top < 0.5 * p.dp(h)
            })
            .collect();
        let mut names: Vec<String> = Vec::new();
        for b in &inside {
            let n: String = b.name.replace(['\n', '\r'], " ").chars().take(32).collect();
            if !n.trim().is_empty() && !names.contains(&n) && names.len() < 3 {
                names.push(n);
            }
        }
        out.push(Block {
            n: out.len() + 1,
            top,
            bottom,
            left: p.dp(x0),
            margin_right: p.dp(w - x1),
            lines,
            items: pieces.len(),
            color,
            box_top: inside.iter().map(|b| b.top).reduce(f32::min),
            box_bottom: inside.iter().map(|b| b.bottom).reduce(f32::max),
            label: names.join(" · "),
            lab: labv,
        });
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Alignment

/// How a block sits across the width: full width, centred, left- or right-aligned. Text width changes with the data
/// and the language (and the screen is wider than the frame), so only the anchored side is compared.
fn anchor(b: &Block) -> char {
    if b.left < 8.0 && b.margin_right < 8.0 {
        'F'
    } else if (b.left - b.margin_right).abs() <= 8.0 {
        'C'
    } else if b.left < b.margin_right {
        'L'
    } else {
        'R'
    }
}

fn similarity(f: &Block, a: &Block) -> f32 {
    // Text that wraps differently (Greek is longer than the English design) is compared per line.
    let (hf, ha) = if f.lines != a.lines { ((f.bottom - f.top) / f.lines as f32, (a.bottom - a.top) / a.lines as f32) } else { (f.bottom - f.top, a.bottom - a.top) };
    let hs = (-(hf - ha).abs() / 4f32.max(0.15 * hf.max(ha))).exp();
    let ms = match (anchor(f), anchor(a)) {
        ('L', 'L') => (-(f.left - a.left).abs() / 10.0).exp(),
        ('R', 'R') => (-(f.margin_right - a.margin_right).abs() / 10.0).exp(),
        (x, y) if x == y => 1.0,
        _ => (-(f.left - a.left).abs().min((f.margin_right - a.margin_right).abs()) / 10.0).exp() * 0.85,
    };
    let is = (-((f.items as f32) - (a.items as f32)).abs() / 4.0).exp();
    let cs = match (f.lab, a.lab) {
        (Some(x), Some(y)) => (-ciede2000(x, y) / 25.0).exp(),
        _ => 0.8,
    };
    hs.powf(0.45) * ms.powf(0.25) * is.powf(0.1) * cs.powf(0.2)
}

const MIN_SIM: f32 = 0.35;

/// The best ordered chain of matched blocks: similar blocks whose vertical spacing changes least from one match to the
/// next. Returns (app block index, figma block index, similarity).
fn align(fb: &[Block], ab: &[Block], skip_top: f32) -> Vec<(usize, usize, f32)> {
    let (n, m) = (fb.len(), ab.len());
    let s: Vec<Vec<f32>> = (0..n)
        .map(|i| (0..m).map(|j| if fb[i].bottom <= skip_top || ab[j].bottom <= skip_top { 0.0 } else { similarity(&fb[i], &ab[j]) }).collect())
        .collect();
    let mut best = vec![vec![f32::NEG_INFINITY; m]; n];
    let mut prev: Vec<Vec<Option<(usize, usize)>>> = vec![vec![None; m]; n];
    for i in 0..n {
        for j in 0..m {
            if s[i][j] < MIN_SIM {
                continue;
            }
            let mut b = s[i][j];
            let mut p = None;
            for i2 in 0..i {
                for j2 in 0..j {
                    if best[i2][j2] == f32::NEG_INFINITY {
                        continue;
                    }
                    let d = ((ab[j].top - ab[j2].top) - (fb[i].top - fb[i2].top)).abs();
                    let skipped = (i - i2 - 1) + (j - j2 - 1);
                    // The spacing may change where a section exists on one side only, but every dp of change and every
                    // skipped block costs: a far jump to a look-alike further down the frame never pays off.
                    let v = best[i2][j2] + s[i][j] + 0.8 * (-d / 16.0).exp() - 0.45 - 0.002 * (d - 24.0).max(0.0) - 0.05 * skipped as f32;
                    if v > b {
                        b = v;
                        p = Some((i2, j2));
                    }
                }
            }
            best[i][j] = b;
            prev[i][j] = p;
        }
    }
    let mut end = None;
    let mut top = f32::NEG_INFINITY;
    for (i, row) in best.iter().enumerate() {
        for (j, &v) in row.iter().enumerate() {
            if v > top {
                top = v;
                end = Some((i, j));
            }
        }
    }
    let mut chain = Vec::new();
    while let Some((i, j)) = end {
        chain.push((j, i, s[i][j]));
        end = prev[i][j];
    }
    chain.reverse();
    chain
}

// ---------------------------------------------------------------------------------------------------------------
// Figma metadata and app tree → named boxes

/// The leaves of a get_metadata XML (texts, instances, shapes) with absolute positions inside the root frame.
pub fn figma_boxes(xml: &str) -> Vec<NamedBox> {
    use quick_xml::events::Event;
    let mut r = quick_xml::reader::Reader::from_str(xml);
    let mut stack: Vec<(f32, f32)> = Vec::new();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        let ev = r.read_event_into(&mut buf);
        let (empty, e) = match ev {
            Ok(Event::Start(e)) => (false, e.into_owned()),
            Ok(Event::Empty(e)) => (true, e.into_owned()),
            Ok(Event::End(_)) => {
                stack.pop();
                buf.clear();
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let (mut x, mut y, mut wdt, mut hgt, mut name) = (0f32, 0f32, 0f32, 0f32, String::new());
        for a in e.attributes().flatten() {
            let v = a.unescape_value().map(|c| c.into_owned()).unwrap_or_default();
            match a.key.as_ref() {
                b"x" => x = v.parse().unwrap_or(0.0),
                b"y" => y = v.parse().unwrap_or(0.0),
                b"width" => wdt = v.parse().unwrap_or(0.0),
                b"height" => hgt = v.parse().unwrap_or(0.0),
                b"name" => name = v,
                _ => {}
            }
        }
        // The root frame's own x/y is its place on the canvas: everything inside is measured from its corner.
        let (ox, oy) = match stack.last() {
            Some(&(px, py)) => (px + x, py + y),
            None => (0.0, 0.0),
        };
        if empty {
            if !stack.is_empty() && wdt > 0.0 && hgt > 0.0 {
                out.push(NamedBox { name, top: oy, bottom: oy + hgt, left: ox, right: ox + wdt });
            }
        } else {
            stack.push((ox, oy));
        }
        buf.clear();
    }
    out
}

/// The root frame's name in a get_metadata XML ("Investing dashboard - by asset type").
pub fn figma_frame_name(xml: &str) -> String {
    let start = xml.find("name=\"").map(|i| i + 6).unwrap_or(0);
    xml[start..].split('"').next().filter(|_| start > 0).map(|n| n.replace("&amp;", "&").replace("&quot;", "\"")).unwrap_or_default()
}

/// Named boxes from a qapilot screen (px → dp).
pub fn app_boxes(screen: &crate::screen::Screen, scale: f32) -> Vec<NamedBox> {
    screen
        .elements
        .iter()
        .filter_map(|e| {
            let name = if !e.text.is_empty() { e.text.clone() } else if !e.desc.is_empty() { e.desc.clone() } else { e.id.rsplit('/').next().unwrap_or("").to_string() };
            // The screen reader's position hint (", 2 από 5" / ", 2 of 5") is not part of the element's name.
            static HINT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let name = HINT.get_or_init(|| regex::Regex::new(r",\s*\d+\s*(από|of)\s*\d+\s*$").unwrap()).replace(&name, "").trim().to_string();
            if name.is_empty() {
                return None;
            }
            let [l, t, r, b] = e.bounds;
            Some(NamedBox { name, top: t as f32 / scale, bottom: b as f32 / scale, left: l as f32 / scale, right: r as f32 / scale })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Findings

fn r1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn name_of(b: &Block, fallback: &str) -> String {
    if b.label.is_empty() {
        format!("{fallback} #{}", b.n)
    } else {
        format!("«{}»", b.label)
    }
}

/// A matched pair's name: the design's component name (Figma layer) when there is one, else the app's text.
fn pair_name(a: &Block, f: &Block) -> String {
    if f.label.is_empty() { name_of(a, "app block") } else { format!("«{}»", f.label) }
}

pub fn compare(app: &Pic, fig: &Pic, app_named: &[NamedBox], fig_named: &[NamedBox], o: &Opts) -> Report {
    let t0 = std::time::Instant::now();
    let ab = blocks(app, o.merge_gap, app_named);
    let fb = blocks(fig, o.merge_gap, fig_named);
    let chain = align(&fb, &ab, o.skip_top);
    // Ink edges of a 1× Figma render are ±1 px each: a gap measured from ink alone needs a wider margin.
    let tol_ink = if o.figma_scale < 2.0 { o.tol.max(3.0) } else { o.tol };
    let mut findings: Vec<Finding> = Vec::new();
    let mut notes = Vec::new();
    let mut push = |f: Finding| findings.push(Finding { id: format!("D{:02}", findings.len() + 1), ..f });
    for w in chain.windows(2) {
        let ((a1, f1, s1), (a2, f2, s2)) = (w[0], w[1]);
        let (pa, na, pf, nf) = (&ab[a1], &ab[a2], &fb[f1], &fb[f2]);
        if a2 != a1 + 1 || f2 != f1 + 1 {
            notes.push(format!(
                "between {} and {}: {} app block(s) and {} Figma block(s) have no counterpart",
                name_of(pa, "app"),
                name_of(na, "app"),
                a2 - a1 - 1,
                f2 - f1 - 1
            ));
            continue;
        }
        let ga = na.top - pa.bottom;
        let gf = nf.top - pf.bottom;
        let d = ga - gf;
        // Layout boxes decide when both sides have boxes of the same kind in both blocks (heights within 3 dp): those
        // are the numbers designers and bug reports use. Otherwise the ink (what one sees) decides.
        let bh = |b: &Block| match (b.box_top, b.box_bottom) {
            (Some(t), Some(u)) => Some(u - t),
            _ => None,
        };
        let comparable = |x: &Block, y: &Block| matches!((bh(x), bh(y)), (Some(p), Some(q)) if (p - q).abs() <= 3.0);
        let boxed = comparable(pa, pf) && comparable(na, nf);
        let (gav, gfv, delta, tol) = if boxed {
            let (x, y) = (na.box_top.unwrap() - pa.box_bottom.unwrap(), nf.box_top.unwrap() - pf.box_bottom.unwrap());
            (x, y, x - y, o.tol)
        } else {
            (ga, gf, d, tol_ink)
        };
        // A clear difference, and a coherent step (the rest of the screen is not shifted wildly).
        if delta.abs() >= tol && delta.abs() <= 0.6 * gfv.max(gav).max(24.0) {
            // Visible edges of text move with the glyphs (Greek accents rise above the Latin cap height of the English
            // design), so a gap measured on ink alone is certain only when it is large and the blocks wrap alike.
            let sure = if boxed {
                s1.min(s2) >= 0.45 && delta.abs() >= tol + 1.0 && d.signum() == delta.signum()
            } else {
                s1.min(s2) >= 0.6 && delta.abs() >= 10.0 && pa.lines == pf.lines && na.lines == nf.lines
            };
            let bx = |p: &Block, n: &Block| {
                (p.box_bottom.is_some() && n.box_top.is_some()).then(|| json!({ "from": p.box_bottom.map(r1), "to": n.box_top.map(r1), "gap": r1(n.box_top.unwrap() - p.box_bottom.unwrap()) }))
            };
            push(Finding {
                id: String::new(),
                category: "Distances",
                confidence: if sure { "CERTAIN" } else { "UNCERTAIN" },
                title: format!(
                    "Gap {} → {}: {} dp in the app, {} dp in Figma ({:+}, {})",
                    pair_name(pa, pf),
                    pair_name(na, nf),
                    r1(gav),
                    r1(gfv),
                    r1(delta),
                    if boxed { "layout boxes" } else { "visible edges" }
                ),
                app: json!({ "ink": { "from": r1(pa.bottom), "to": r1(na.top), "gap": r1(ga) }, "box": bx(pa, na) }),
                figma: json!({ "ink": { "from": r1(pf.bottom), "to": r1(nf.top), "gap": r1(gf) }, "box": bx(pf, nf) }),
                delta: r1(delta),
                basis: if boxed { "box" } else { "ink" },
                app_blocks: vec![pa.n, na.n],
                figma_blocks: vec![pf.n, nf.n],
                image: None,
            });
        }
    }
    for &(a, f, s) in &chain {
        let (ba, bf) = (&ab[a], &fb[f]);
        if s < 0.45 {
            continue;
        }
        let (ha, hf) = (ba.bottom - ba.top, bf.bottom - bf.top);
        // Height of a single-line block (a button, a chip row, one line of text): sizing or font size.
        if ba.lines == bf.lines && (ha - hf).abs() >= tol_ink.max(0.08 * hf) && ba.lines == 1 && s >= 0.55 {
            let text_like = hf <= 40.0 && ba.items >= 1;
            push(Finding {
                id: String::new(),
                category: if text_like && ha.max(hf) <= 32.0 { "Typography" } else { "Sizing" },
                confidence: "UNCERTAIN",
                title: format!("Height of {}: {} dp in the app, {} dp in Figma ({:+})", pair_name(ba, bf), r1(ha), r1(hf), r1(ha - hf)),
                app: json!({ "top": r1(ba.top), "bottom": r1(ba.bottom), "height": r1(ha) }),
                figma: json!({ "top": r1(bf.top), "bottom": r1(bf.bottom), "height": r1(hf) }),
                delta: r1(ha - hf),
                basis: "ink",
                app_blocks: vec![ba.n],
                figma_blocks: vec![bf.n],
                image: None,
            });
        }
        // Left margin of content that is left-aligned on both sides (centred and evenly spread rows move with the
        // screen width, which is wider than the frame).
        let dl = ba.left - bf.left;
        // A row of 3+ pieces spread over most of the width (action buttons, tabs) moves with the screen width too.
        let spread = |b: &Block, w: f32| b.items >= 3 && (w - b.left - b.margin_right) > 0.6 * w;
        let (aw, fw) = (app.dp(app.w()), fig.dp(fig.w()));
        if dl.abs() >= tol_ink && anchor(ba) == 'L' && anchor(bf) == 'L' && !(spread(ba, aw) || spread(bf, fw)) {
            push(Finding {
                id: String::new(),
                category: "Distances",
                confidence: if dl.abs() >= tol_ink + 2.0 && s >= 0.6 { "CERTAIN" } else { "UNCERTAIN" },
                title: format!("Left margin of {}: {} dp in the app, {} dp in Figma ({:+})", pair_name(ba, bf), r1(ba.left), r1(bf.left), r1(dl)),
                app: json!({ "left": r1(ba.left) }),
                figma: json!({ "left": r1(bf.left) }),
                delta: r1(dl),
                basis: "ink",
                app_blocks: vec![ba.n],
                figma_blocks: vec![bf.n],
                image: None,
            });
        }
        // Content that starts at the very left edge of the screen while the design keeps a margin: the first item of
        // a row is cut off (typically a horizontal list scrolled past its start).
        if ba.left < 1.0 && bf.left >= 8.0 {
            push(Finding {
                id: String::new(),
                category: "Layout",
                confidence: if s >= 0.55 { "CERTAIN" } else { "UNCERTAIN" },
                title: format!("{} starts at the left edge of the screen (cut off) — Figma keeps a {} dp margin", pair_name(ba, bf), r1(bf.left)),
                app: json!({ "left": r1(ba.left) }),
                figma: json!({ "left": r1(bf.left) }),
                delta: r1(ba.left - bf.left),
                basis: "ink",
                app_blocks: vec![ba.n],
                figma_blocks: vec![bf.n],
                image: None,
            });
        }
        // Colour of the text / icons when each side has one dominant colour.
        if let (Some(la), Some(lf)) = (ba.lab, bf.lab) {
            let de = ciede2000(la, lf);
            if de >= 10.0 {
                push(Finding {
                    id: String::new(),
                    category: "Colors",
                    confidence: if de >= 18.0 && s >= 0.7 { "CERTAIN" } else { "UNCERTAIN" },
                    title: format!(
                        "Colour of {}: {} in the app, {} in Figma (ΔE00 {:.1})",
                        pair_name(ba, bf),
                        ba.color.clone().unwrap_or_default(),
                        bf.color.clone().unwrap_or_default(),
                        de
                    ),
                    app: json!({ "color": ba.color }),
                    figma: json!({ "color": bf.color }),
                    delta: r1(de),
                    basis: "ink",
                    app_blocks: vec![ba.n],
                    figma_blocks: vec![bf.n],
                    image: None,
                });
            }
        }
    }
    Report {
        app: json!({ "label": o.app_label, "device": o.device_label, "scale": app.scale, "width": r1(app.dp(app.w())), "height": r1(app.dp(app.h())) }),
        figma: json!({ "name": o.figma_name, "frame": o.figma_frame, "scale": fig.scale, "width": r1(fig.dp(fig.w())), "height": r1(fig.dp(fig.h())) }),
        app_blocks: ab,
        figma_blocks: fb,
        matches: chain.iter().map(|&(a, f, s)| (a + 1, f + 1, (s * 100.0).round() / 100.0)).collect(),
        findings,
        notes,
        ms: t0.elapsed().as_millis(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Pictures: an overview (FIGMA | APP, 1 px = 1 dp, matched blocks boxed and linked) and one crop per finding
// (FIGMA | APP around the measured spot, 2 px = 1 dp), like the composites UI testers attach to bug reports.


fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        ',' => [0, 0, 0, 0, 0x0C, 0x04, 0x08],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
        '/' => [0, 0x01, 0x02, 0x04, 0x08, 0x10, 0],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '=' => [0, 0, 0x1F, 0, 0x1F, 0, 0],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '_' => [0, 0, 0, 0, 0, 0, 0x1F],
        _ => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    }
}

pub fn pixel_text(img: &mut RgbImage, x: i32, y: i32, s: &str, size: i32, c: Rgb<u8>) {
    for (k, ch) in s.chars().enumerate() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    rect_fill(img, x + (k as i32 * 6 + col) * size, y + row as i32 * size, size, size, c);
                }
            }
        }
    }
}

fn rect_fill(img: &mut RgbImage, x: i32, y: i32, w: i32, h: i32, c: Rgb<u8>) {
    for yy in y.max(0)..(y + h).min(img.height() as i32) {
        for xx in x.max(0)..(x + w).min(img.width() as i32) {
            img.put_pixel(xx as u32, yy as u32, c);
        }
    }
}




/// The picture at `k` px per dp.
fn at(p: &Pic, k: f32) -> RgbImage {
    let w = ((p.w() as f32 / p.scale) * k).round().max(1.0) as u32;
    let h = ((p.h() as f32 / p.scale) * k).round().max(1.0) as u32;
    imageops::resize(&p.img, w, h, imageops::FilterType::Triangle)
}

// Pictures (drawn by pics.rs): cards side by side — the design, then the app — with the measured elements and values.

/// The platform of the app side, from the device label: its card title and colour.
pub fn platform(rep: &Report) -> (&'static str, Rgb<u8>) {
    let d = rep.app["device"].as_str().unwrap_or("");
    if d.starts_with("iOS") {
        ("iOS", crate::pics::IOS)
    } else if d.starts_with("Android") {
        ("Android", crate::pics::ANDROID)
    } else {
        ("App", crate::pics::ANDROID)
    }
}

pub fn design_subtitle(rep: &Report) -> String {
    let node = rep.figma["name"].as_str().unwrap_or("");
    let frame = rep.figma["frame"].as_str().unwrap_or("");
    let mut parts = vec![if node.is_empty() { "Figma".to_string() } else { format!("Figma {node}") }];
    if !frame.is_empty() {
        parts.push(format!("«{frame}»"));
    }
    parts.push(format!("{} pt wide", num(rep.figma["width"].as_f64().unwrap_or(0.0))));
    parts.join(" · ")
}

pub fn app_subtitle(rep: &Report) -> String {
    let mut parts = Vec::new();
    if let Some(v) = rep.app["label"].as_str().filter(|v| !v.is_empty()) {
        parts.push(v.to_string());
    }
    // The card's title already says the platform: "Android · Pixel 9 Pro (emulator)" → "Pixel 9 Pro (emulator)".
    let dev = rep.app["device"].as_str().unwrap_or("");
    let dev = dev.split_once(" · ").map(|(_, d)| d).unwrap_or(if dev == "Android" || dev == "iOS" { "" } else { dev });
    if !dev.is_empty() {
        parts.push(dev.to_string());
    }
    let unit = if platform(rep).0 == "iOS" { "pt" } else { "dp" };
    parts.push(format!("{} {unit} wide", num(rep.app["width"].as_f64().unwrap_or(0.0))));
    parts.join(" · ")
}

/// The vertical window (dp) that shows these blocks with some room around them.
pub fn window(bs: &[&Block], max_h: f32) -> (f32, f32) {
    let t0 = bs.iter().map(|b| b.top).fold(f32::MAX, f32::min);
    let t1 = bs.iter().map(|b| b.bottom).fold(0.0, f32::max);
    let win = ((t1 - t0) + 2.0 * 70.0).min(460.0);
    let c = (t0 + t1) / 2.0;
    let y0 = (c - win / 2.0).max(0.0);
    (y0, (y0 + win).min(max_h))
}

/// A picture cut to a dp window at `k` px per dp.
pub fn cut(p: &Pic, k: f32, (y0, y1): (f32, f32)) -> RgbImage {
    let full = at(p, k);
    let y = (y0 * k) as u32;
    let h = (((y1 - y0) * k) as u32).min(full.height().saturating_sub(y)).max(1);
    imageops::crop_imm(&full, 0, y, full.width(), h).to_image()
}

/// The edges a finding measured: layout boxes for a "box" finding, else the visible ink.
pub fn measured(b: &Block, boxed: bool) -> (f32, f32) {
    if boxed {
        (b.box_top.unwrap_or(b.top), b.box_bottom.unwrap_or(b.bottom))
    } else {
        (b.top, b.bottom)
    }
}

/// The marks of one side of a finding: its blocks outlined and named, the measured distance with its value.
#[allow(clippy::too_many_arguments)]
pub fn finding_marks(bs: &[&Block], f: &Finding, side: &serde_json::Value, wdp: f32, y0: f32, k: f32, color: Rgb<u8>, value_color: Rgb<u8>, value: &str, fallback: &str) -> Vec<crate::pics::Mark> {
    use crate::pics::Mark;
    let boxed = f.basis == "box";
    let px = |v: f32| (v * k).round() as i32;
    let mut m = Vec::new();
    for (i, b) in bs.iter().enumerate() {
        let (t, u) = measured(b, boxed);
        let label = if b.label.is_empty() { format!("{fallback} {}", i + 1) } else { b.label.clone() };
        m.push(Mark::Box { x: px(b.left) - 6, y: px(t - y0) - 4, w: px(wdp - b.left - b.margin_right) + 12, h: px(u - t) + 8, label, color });
    }
    match f.category {
        "Distances" if bs.len() == 2 => {
            let ((_, pb), (nt, _)) = (measured(bs[0], boxed), measured(bs[1], boxed));
            m.push(Mark::VDim { x: px(wdp) - 34, y0: px(pb - y0), y1: px(nt - y0), value: value.to_string(), color: value_color });
        }
        "Distances" | "Layout" if !bs.is_empty() => {
            let l = side["left"].as_f64().unwrap_or(bs[0].left as f64) as f32;
            m.push(Mark::HDim { x0: 2, x1: px(l).max(4), y: px(bs[0].bottom - y0) + 22, value: value.to_string(), color: value_color });
        }
        _ if !bs.is_empty() => {
            let (t, u) = measured(bs[0], boxed);
            m.push(Mark::VDim { x: px(wdp) - 34, y0: px(t - y0), y1: px(u - y0), value: value.to_string(), color: value_color });
        }
        _ => {}
    }
    m
}

/// "32 dp" / "#1c1f22" — the value one side of a finding measured.
pub fn side_value(f: &Finding, v: &serde_json::Value) -> String {
    match f.category {
        "Distances" if f.app_blocks.len() == 2 => {
            let g = &v[if f.basis == "box" { "box" } else { "ink" }]["gap"];
            format!("{} dp", num(g.as_f64().unwrap_or(0.0)))
        }
        "Colors" => v["color"].as_str().unwrap_or("").to_string(),
        "Distances" | "Layout" => format!("{} dp", num(v["left"].as_f64().unwrap_or(0.0))),
        _ => format!("{} dp", num(v["height"].as_f64().unwrap_or(0.0))),
    }
}

pub fn signed(d: f32, unit: &str) -> String {
    format!("{}{}{unit}", if d > 0.0 { "+" } else { "" }, num(d as f64))
}

/// Whole screen: the design and the app side by side, matched blocks in colour, the rest grey, differences tagged.
pub fn overview(app: &Pic, fig: &Pic, rep: &Report, path: &str) -> Result<(), String> {
    use crate::pics::{compose, Card, Mark, BAD, DESIGN, INK, MUTED, WARN};
    let k = 1.5f32;
    let (pname, pcol) = platform(rep);
    let fig_h = rep.matches.last().map(|&(_, f, _)| rep.figma_blocks[f - 1].bottom + 60.0).unwrap_or(fig.dp(fig.h())).max(app.dp(app.h())).min(fig.dp(fig.h()));
    let matched_a: Vec<usize> = rep.matches.iter().map(|m| m.0).collect();
    let matched_f: Vec<usize> = rep.matches.iter().map(|m| m.1).collect();
    let px = |v: f32| (v * k).round() as i32;
    let outline = |b: &Block, wdp: f32, on: bool, c: Rgb<u8>| {
        if on {
            Mark::Box { x: px(b.left) - 3, y: px(b.top) - 3, w: px(wdp - b.left - b.margin_right) + 6, h: px(b.bottom - b.top) + 6, label: String::new(), color: c }
        } else {
            Mark::Ghost { x: px(b.left) - 3, y: px(b.top) - 3, w: px(wdp - b.left - b.margin_right) + 6, h: px(b.bottom - b.top) + 6 }
        }
    };
    let (fw, aw) = (fig.dp(fig.w()), app.dp(app.w()));
    let fmarks: Vec<Mark> = rep.figma_blocks.iter().filter(|b| b.bottom <= fig_h).map(|b| outline(b, fw, matched_f.contains(&b.n), DESIGN)).collect();
    let mut amarks: Vec<Mark> = rep.app_blocks.iter().map(|b| outline(b, aw, matched_a.contains(&b.n), pcol)).collect();
    // One tag per element, listing every difference found on it.
    let mut tagged: Vec<(usize, Vec<&Finding>)> = Vec::new();
    for f in &rep.findings {
        let Some(&a0) = f.app_blocks.first() else { continue };
        match tagged.iter_mut().find(|(a, _)| *a == a0) {
            Some((_, v)) => v.push(f),
            None => tagged.push((a0, vec![f])),
        }
    }
    for (a0, fs) in tagged {
        let b = &rep.app_blocks[a0 - 1];
        let tag = fs.iter().map(|f| format!("{} {}", f.id, signed(f.delta, if f.category == "Colors" { " ΔE" } else { " dp" }))).collect::<Vec<_>>().join(" · ");
        let sure = fs.iter().any(|f| f.confidence == "CERTAIN");
        amarks.push(Mark::Box { x: px(b.left) - 3, y: px(b.top) - 3, w: px(aw - b.left - b.margin_right) + 6, h: px(b.bottom - b.top) + 6, label: tag, color: if sure { BAD } else { WARN } });
    }
    let cards = [
        Card { title: "Design".into(), subtitle: design_subtitle(rep), color: DESIGN, shot: cut(fig, k, (0.0, fig_h)), marks: fmarks },
        Card { title: pname.into(), subtitle: app_subtitle(rep), color: pcol, shot: at(app, k), marks: amarks },
    ];
    let certain = rep.findings.iter().filter(|f| f.confidence == "CERTAIN").count();
    let chips = vec![
        (format!("{} matched", rep.matches.len()), MUTED),
        (format!("{certain} certain"), BAD),
        (format!("{} to confirm", rep.findings.len() - certain), WARN),
    ];
    let footer = vec![(format!("Violet: the design's elements · {}: the app's elements ({pname}) · grey: no counterpart on the other side · red / amber tags: the differences", if pname == "iOS" { "blue" } else { "green" }), MUTED, false)];
    let _ = INK;
    compose(&cards, &format!("Screen check · {}", rep.figma["name"].as_str().filter(|n| !n.is_empty()).map(|n| format!("Figma {n}")).unwrap_or_else(|| "Figma".into())), &chips, &footer, path)
}

/// One finding: the design and the app cut around it, the measured elements named, the values on both sides.
pub fn finding_picture(app: &Pic, fig: &Pic, rep: &Report, f: &Finding, path: &str) -> Result<(), String> {
    use crate::pics::{compose, Card, BAD, DESIGN, INK, MUTED, WARN};
    let k = 2.0f32;
    let (pname, pcol) = platform(rep);
    let ab: Vec<&Block> = f.app_blocks.iter().map(|&n| &rep.app_blocks[n - 1]).collect();
    let fb: Vec<&Block> = f.figma_blocks.iter().map(|&n| &rep.figma_blocks[n - 1]).collect();
    let (fwin, awin) = (window(&fb, fig.dp(fig.h())), window(&ab, app.dp(app.h())));
    let unit = if f.category == "Colors" { " ΔE" } else { " dp" };
    let (fv, av) = (side_value(f, &f.figma), side_value(f, &f.app));
    let bad = if f.confidence == "CERTAIN" { BAD } else { WARN };
    let cards = [
        Card { title: "Design".into(), subtitle: design_subtitle(rep), color: DESIGN, shot: cut(fig, k, fwin), marks: finding_marks(&fb, f, &f.figma, fig.dp(fig.w()), fwin.0, k, DESIGN, DESIGN, &fv, "design element") },
        Card { title: pname.into(), subtitle: app_subtitle(rep), color: pcol, shot: cut(app, k, awin), marks: finding_marks(&ab, f, &f.app, app.dp(app.w()), awin.0, k, pcol, bad, &format!("{av} ({})", signed(f.delta, unit)), "app element") },
    ];
    let chips = vec![(f.id.clone(), INK), (f.category.to_string(), MUTED), ((if f.confidence == "CERTAIN" { "Certain" } else { "To confirm" }).to_string(), bad)];
    let how = match (f.category, f.basis) {
        ("Distances", "box") if f.app_blocks.len() == 2 => "measured on the layout boxes (the elements' frames in the app tree and in Figma)",
        ("Distances", _) if f.app_blocks.len() == 2 => "measured on the visible edges (pixels) — check the picture",
        ("Colors", _) => "colour of the text / icons, CIEDE2000",
        _ => "measured on the visible edges (pixels)",
    };
    let footer = vec![(format!("Design {fv}   ·   {pname} {av}   ·   difference {}", signed(f.delta, unit)), INK, true), (how.to_string(), MUTED, false)];
    compose(&cards, &f.title, &chips, &footer, path)
}

/// The run folder (the nearest folder above `out` with a run.json), so the markdown's pictures resolve from report.md.
pub fn md_base(out: &str) -> Option<std::path::PathBuf> {
    let abs = std::fs::canonicalize(out).ok()?;
    abs.ancestors().find(|d| d.join("run.json").exists()).map(|d| d.to_path_buf())
}

pub fn rel(path: &str, base: &Option<std::path::PathBuf>) -> String {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
    match base {
        Some(b) => p.strip_prefix(b).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string()),
        None => p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

/// 32 → "32", 426.70001 → "426.7".
pub fn num(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r:.1}") }
}

/// A block's name: its own label, else the smallest named box around it ("part of «Investments graph»").
fn name(b: &Block, named: &[NamedBox]) -> String {
    let y = format!("y {}–{}", num(b.top as f64), num(b.bottom as f64));
    if !b.label.is_empty() {
        return format!("«{}» ({y})", b.label.replace('|', "/").replace('\n', " "));
    }
    named
        .iter()
        .filter(|n| n.top <= b.top + 2.0 && n.bottom >= b.bottom - 2.0 && !n.name.trim().is_empty())
        .min_by(|a, c| (a.bottom - a.top).total_cmp(&(c.bottom - c.top)))
        .map(|n| format!("part of «{}» ({y})", n.name.replace('|', "/").replace('\n', " ").chars().take(40).collect::<String>()))
        .unwrap_or_else(|| format!("a block ({y})"))
}

/// A ready-to-paste report section for this screen: what the design has and what the device shows, side by side, and
/// every measured difference with its picture.
pub fn markdown(rep: &Report, o: &Opts, out: &str, app_named: &[NamedBox], fig_named: &[NamedBox]) -> String {
    let base = md_base(out);
    let figma = if o.figma_name.is_empty() { "the Figma frame".to_string() } else { format!("Figma {}", o.figma_name) };
    let g = |v: &serde_json::Value, k: &str| num(v[k].as_f64().unwrap_or(0.0));
    let certain = rep.findings.iter().filter(|f| f.confidence == "CERTAIN").count();
    let mut m = format!("#### Design check · {figma}\n\n");
    m.push_str("| | Figma (design) | App (device) |\n|---|---|---|\n");
    m.push_str(&format!("| Size | {} × {} pt | {} × {} dp ({} px per dp) |\n", g(&rep.figma, "width"), g(&rep.figma, "height"), g(&rep.app, "width"), g(&rep.app, "height"), num(o.app_scale as f64)));
    m.push_str(&format!("| Visual blocks | {} | {} ({} matched) |\n", rep.figma_blocks.len(), rep.app_blocks.len(), rep.matches.len()));
    m.push_str(&format!("| Differences | {certain} certain · {} to confirm | tolerance {} dp |\n\n", rep.findings.len() - certain, num(o.tol as f64)));
    m.push_str(&format!("![FIGMA | APP — matched blocks boxed and linked, differences marked]({})\n\n", rel(&format!("{out}/design.png"), &base)));

    // Side by side, in screen order: what the design has, what the app shows, and how they compare.
    let issues_of = |a: usize| -> Vec<&Finding> { rep.findings.iter().filter(|f| f.app_blocks.contains(&a)).collect() };
    m.push_str("**Side by side** (top to bottom):\n\n| Figma (design) | App (device) | |\n|---|---|---|\n");
    let mut prev: Option<(usize, usize)> = None;
    for &(a, f, _) in &rep.matches {
        if let Some((pa, pf)) = prev {
            let (fs, aps): (Vec<usize>, Vec<usize>) = ((pf + 1..f).collect(), (pa + 1..a).collect());
            for i in 0..fs.len().max(aps.len()) {
                let fd = fs.get(i).map(|n| name(&rep.figma_blocks[n - 1], fig_named)).unwrap_or_else(|| "—".into());
                let ap = aps.get(i).map(|n| name(&rep.app_blocks[n - 1], app_named)).unwrap_or_else(|| "—".into());
                let st = match (fs.get(i), aps.get(i)) {
                    (Some(_), Some(_)) => "different content",
                    (Some(_), None) => "only in the design",
                    _ => "only in the app",
                };
                m.push_str(&format!("| {fd} | {ap} | ➖ {st} |\n"));
            }
        }
        let iss = issues_of(a);
        let st = if iss.is_empty() { "✅ as designed".to_string() } else { format!("⚠️ {}", iss.iter().map(|f| f.id.as_str()).collect::<Vec<_>>().join(", ")) };
        m.push_str(&format!("| {} | {} | {st} |\n", name(&rep.figma_blocks[f - 1], fig_named), name(&rep.app_blocks[a - 1], app_named)));
        prev = Some((a, f));
    }
    let below = rep.figma_blocks.len().saturating_sub(rep.matches.last().map(|m| m.1).unwrap_or(0));
    if below > 0 {
        m.push_str(&format!("| _{below} more block(s) further down the frame_ | _not on the screen_ | scroll and check again |\n"));
    }
    m.push('\n');

    // The measured differences: both values, the difference, and the picture.
    if rep.findings.is_empty() {
        m.push_str("No measurable difference between the matched blocks.\n");
        return m;
    }
    m.push_str("**Differences**\n\n| # | Check | Figma (design) | App (device) | Δ | | Picture |\n|---|---|---|---|---|---|---|\n");
    for f in &rep.findings {
        let what = f.title.split(':').next().unwrap_or(&f.title).replace('|', "/");
        let side = |v: &serde_json::Value| -> String {
            match f.category {
                "Distances" if f.app_blocks.len() == 2 => {
                    let gap = &v[if f.basis == "box" { "box" } else { "ink" }];
                    format!("{} dp ({} → {})", num(gap["gap"].as_f64().unwrap_or(0.0)), num(gap["from"].as_f64().unwrap_or(0.0)), num(gap["to"].as_f64().unwrap_or(0.0)))
                }
                "Colors" => v["color"].as_str().unwrap_or("").to_string(),
                _ => v.as_object().map(|o| o.iter().map(|(k, x)| format!("{k} {}", x.as_f64().map(num).unwrap_or_else(|| x.to_string()))).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            }
        };
        let basis = if f.category == "Distances" && f.app_blocks.len() == 2 { if f.basis == "box" { " · layout boxes" } else { " · visible edges" } } else { "" };
        let unit = if f.category == "Colors" { " ΔE" } else { " dp" };
        let pic = f.image.as_ref().map(|p| format!("[{}]({})", f.id, rel(p, &base))).unwrap_or_default();
        let sign = if f.delta > 0.0 { "+" } else { "" };
        m.push_str(&format!(
            "| {} | {what}{basis} | {} | {} | {sign}{}{unit} | {} | {pic} |\n",
            f.id,
            side(&f.figma),
            side(&f.app),
            num(f.delta as f64),
            if f.confidence == "CERTAIN" { "✅ certain" } else { "❓ confirm" }
        ));
    }
    m.push_str("\n**Pictures** (FIGMA left · APP right, same scale; the measured edges are drawn)\n\n");
    for f in &rep.findings {
        if let Some(p) = &f.image {
            m.push_str(&format!("![{} — {}]({})\n\n", f.id, f.title.replace(['[', ']'], ""), rel(p, &base)));
        }
    }
    m
}

/// Writes design.json, design.md, design.png and one picture per finding into `out`, and returns a short text summary.
pub fn run(app: &Pic, fig: &Pic, app_named: &[NamedBox], fig_named: &[NamedBox], o: &Opts, out: &str) -> Result<String, String> {
    let mut rep = compare(app, fig, app_named, fig_named, o);
    std::fs::create_dir_all(out).map_err(|e| format!("{out}: {e}"))?;
    let ov = format!("{out}/design.png");
    overview(app, fig, &rep, &ov)?;
    for i in 0..rep.findings.len() {
        let p = format!("{out}/{}.png", rep.findings[i].id);
        finding_picture(app, fig, &rep, &rep.findings[i], &p)?;
        rep.findings[i].image = Some(p);
    }
    let boxes = format!("{out}/app-boxes.json");
    std::fs::write(&boxes, serde_json::to_string(app_named).unwrap_or_default()).map_err(|e| format!("{boxes}: {e}"))?;
    let js = format!("{out}/design.json");
    std::fs::write(&js, serde_json::to_string_pretty(&rep).unwrap_or_default()).map_err(|e| format!("{js}: {e}"))?;
    let md = format!("{out}/design.md");
    std::fs::write(&md, markdown(&rep, o, out, app_named, fig_named)).map_err(|e| format!("{md}: {e}"))?;
    let mut s = format!(
        "design check: {} app blocks, {} Figma blocks, {} matched, {} finding(s) in {} ms (tolerance {} dp)\n",
        rep.app_blocks.len(),
        rep.figma_blocks.len(),
        rep.matches.len(),
        rep.findings.len(),
        rep.ms,
        o.tol
    );
    for f in &rep.findings {
        s.push_str(&format!("{} [{} | {}] {}\n", f.id, f.category, f.confidence, f.title));
    }
    for n in &rep.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    s.push_str(&format!("overview: {ov}\nreport section (paste into the report): {md}\ndetails: {js}\n"));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white screen with dark bars at the given (top, height, left, width) in dp.
    fn screen(scale: f32, w: f32, h: f32, bars: &[(f32, f32, f32, f32)]) -> Pic {
        let mut img = RgbImage::from_pixel((w * scale) as u32, (h * scale) as u32, crate::pics::WHITE);
        for &(t, hh, l, ww) in bars {
            rect_fill(&mut img, (l * scale) as i32, (t * scale) as i32, (ww * scale) as i32, (hh * scale) as i32, Rgb([30, 35, 40]));
        }
        Pic { img, scale }
    }

    #[test]
    fn finds_a_gap_that_differs_and_nothing_else() {
        // Figma (1 px/pt, 375 wide): title, tabs, amount with gaps 32 and 32. App (3 px/dp, 427 wide): the second gap is 36.
        let fig = screen(1.0, 375.0, 400.0, &[(64.0, 32.0, 24.0, 180.0), (128.0, 32.0, 24.0, 300.0), (192.0, 40.0, 24.0, 120.0)]);
        let app = screen(3.0, 427.0, 420.0, &[(70.0, 32.0, 24.0, 180.0), (134.0, 32.0, 24.0, 300.0), (202.0, 40.0, 24.0, 120.0)]);
        let rep = compare(&app, &fig, &[], &[], &Opts::default());
        assert_eq!(rep.matches.len(), 3, "{:?}", rep.matches);
        let gaps: Vec<&Finding> = rep.findings.iter().filter(|f| f.category == "Distances").collect();
        assert_eq!(gaps.len(), 1, "{:#?}", rep.findings);
        assert!((gaps[0].delta - 4.0).abs() < 0.5, "{:?}", gaps[0]);
    }

    #[test]
    fn skips_blocks_that_exist_on_one_side_only() {
        // The app has an extra banner between the tabs and the amount: that is a note, not a gap finding.
        let fig = screen(1.0, 375.0, 400.0, &[(64.0, 32.0, 24.0, 180.0), (128.0, 32.0, 24.0, 300.0), (192.0, 40.0, 24.0, 120.0)]);
        let app = screen(3.0, 427.0, 500.0, &[(64.0, 32.0, 24.0, 180.0), (128.0, 32.0, 24.0, 300.0), (176.0, 60.0, 24.0, 380.0), (268.0, 40.0, 24.0, 120.0)]);
        let rep = compare(&app, &fig, &[], &[], &Opts::default());
        assert!(rep.findings.iter().all(|f| f.category != "Distances" || f.app_blocks != vec![2, 4]), "{:#?}", rep.findings);
        assert_eq!(rep.notes.len(), 1, "{:?} {:?}", rep.notes, rep.matches);
    }

    #[test]
    fn figma_metadata_boxes_are_absolute() {
        let xml = r#"<frame id="1:1" name="Screen" x="80" y="174" width="375" height="800">
            <frame id="1:2" name="Body" x="0" y="136" width="375" height="600">
              <instance id="1:3" name="Nav-Tabs" x="0" y="0" width="375" height="32" />
              <frame id="1:4" name="Amount" x="0" y="64" width="375" height="72">
                <text id="1:5" name="Text" x="26" y="0" width="183" height="40" />
              </frame>
            </frame>
          </frame>"#;
        let b = figma_boxes(xml);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].name.as_str(), b[0].top, b[0].bottom), ("Nav-Tabs", 136.0, 168.0));
        assert_eq!((b[1].name.as_str(), b[1].top, b[1].left), ("Text", 200.0, 26.0));
    }

    #[test]
    fn ciede2000_reference_pair() {
        // Sharma et al. test data, pair 1: ΔE00 = 2.0425
        let d = ciede2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]);
        assert!((d - 2.0425).abs() < 0.01, "{d}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Entry point shared by the CLI and the MCP tool

#[derive(Debug, Default)]
pub struct Args {
    pub figma: String,
    pub figma_meta: Option<String>,
    pub figma_scale: Option<f32>,
    pub figma_name: Option<String>,
    /// a saved screenshot; without it the live screen is taken (screenshot + tree)
    pub app: Option<String>,
    /// a saved qapilot screen (JSON) for a saved screenshot
    pub tree: Option<String>,
    pub app_scale: Option<f32>,
    pub out: String,
    pub tol: Option<f32>,
    /// the app under test for the pictures, e.g. "MyApp 2.3.0 (230)"
    pub app_name: Option<String>,
}

pub fn check(serial: &str, a: &Args) -> Result<String, String> {
    let mut o = Opts::default();
    if let Some(t) = a.tol {
        o.tol = t;
    }
    o.figma_scale = a.figma_scale.unwrap_or(1.0);
    o.figma_name = a.figma_name.clone().unwrap_or_default();
    let fig = Pic::load(&a.figma, o.figma_scale)?;
    let fig_named = match &a.figma_meta {
        Some(p) => {
            let xml = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            o.figma_frame = figma_frame_name(&xml);
            figma_boxes(&xml)
        }
        None => Vec::new(),
    };
    o.app_label = a.app_name.clone().unwrap_or_default();
    let (app, app_named) = match &a.app {
        Some(p) => {
            o.app_scale = a.app_scale.unwrap_or(3.0);
            let named = match &a.tree {
                Some(t) => {
                    let sc: crate::screen::Screen = serde_json::from_str(&std::fs::read_to_string(t).map_err(|e| format!("{t}: {e}"))?).map_err(|e| format!("{t}: {e}"))?;
                    app_boxes(&sc, o.app_scale)
                }
                None => Vec::new(),
            };
            (Pic::load(p, o.app_scale)?, named)
        }
        None => {
            let mut s = crate::session::Session::new(serial);
            o.app_scale = a.app_scale.unwrap_or_else(|| s.d.scale());
            let sc = s.read()?;
            o.device_label = s.d.label();
            if o.app_label.is_empty() {
                o.app_label = sc.package.clone();
            }
            let png = s.d.screenshot_png()?;
            std::fs::create_dir_all(&a.out).map_err(|e| format!("{}: {e}", a.out))?;
            let shot = format!("{}/app.png", a.out);
            std::fs::write(&shot, &png).map_err(|e| format!("{shot}: {e}"))?;
            (Pic::load(&shot, o.app_scale)?, app_boxes(&sc, o.app_scale))
        }
    };
    let summary = run(&app, &fig, &app_named, &fig_named, &o, &a.out)?;
    if let Some(p) = &a.app {
        let _ = std::fs::copy(p, format!("{}/app.png", a.out));
    }
    let inputs = json!({
        "figma": std::fs::canonicalize(&a.figma).map(|p| p.display().to_string()).unwrap_or_else(|_| a.figma.clone()),
        "figma_meta": a.figma_meta.as_ref().map(|m| std::fs::canonicalize(m).map(|p| p.display().to_string()).unwrap_or_else(|_| m.clone())),
        "figma_name": o.figma_name, "figma_scale": o.figma_scale, "app_scale": o.app_scale, "app_label": o.app_label, "device_label": o.device_label,
    });
    let _ = std::fs::write(format!("{}/inputs.json", a.out), serde_json::to_string_pretty(&inputs).unwrap_or_default());
    Ok(summary)
}
