//! Cross-platform design check: one Figma frame against the Android app and the iOS app, rebuilt from the folders each
//! platform's `design` check left (app.png, app-boxes.json, inputs.json). Every difference — keyed by the design's
//! elements — gets the design's value, Android's and iOS's (or "as designed"), one DESIGN | ANDROID | iOS picture, and
//! a row in cross.md, the report section that shows all three side by side.
use crate::design::{self, Block, Finding, NamedBox, Opts, Pic, Report};
use crate::pics::{self, Card, Mark};
use image::Rgb;
use serde_json::{json, Value};

struct Side {
    name: &'static str,
    color: Rgb<u8>,
    pic: Pic,
    rep: Report,
}

fn read_json(p: &str) -> Result<Value, String> {
    serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}"))
}

fn side(name: &'static str, color: Rgb<u8>, dir: &str, fig: &Pic, fig_named: &[NamedBox], base: &Opts) -> Result<Side, String> {
    let inp = read_json(&format!("{dir}/inputs.json"))?;
    let mut o = base.clone();
    o.app_scale = inp["app_scale"].as_f64().unwrap_or(3.0) as f32;
    o.app_label = inp["app_label"].as_str().unwrap_or("").to_string();
    o.device_label = inp["device_label"].as_str().unwrap_or("").to_string();
    if o.device_label.is_empty() {
        o.device_label = name.to_string();
    }
    let pic = Pic::load(&format!("{dir}/app.png"), o.app_scale)?;
    let boxes: Vec<NamedBox> = serde_json::from_value(read_json(&format!("{dir}/app-boxes.json"))?).unwrap_or_default();
    let rep = design::compare(&pic, fig, &boxes, fig_named, &o);
    Ok(Side { name, color, pic, rep })
}

/// The app block matched to a Figma block.
fn matched(rep: &Report, f: usize) -> Option<&Block> {
    rep.matches.iter().find(|m| m.1 == f).map(|m| &rep.app_blocks[m.0 - 1])
}

/// A difference across platforms: what was checked (category + the design's blocks) and each platform's result.
struct Row<'a> {
    category: &'static str,
    figma_blocks: Vec<usize>,
    basis: &'static str,
    /// per side: the finding, if that platform has one
    found: Vec<Option<&'a Finding>>,
}

/// A side's own measurement of a check that is not a finding there (to show it is as designed).
fn measure(s: &Side, r: &Row) -> Option<String> {
    let bs: Option<Vec<&Block>> = r.figma_blocks.iter().map(|&f| matched(&s.rep, f)).collect();
    let bs = bs?;
    let boxed = r.basis == "box";
    Some(match r.category {
        "Distances" if bs.len() == 2 => {
            let ((_, pb), (nt, _)) = (design::measured(bs[0], boxed), design::measured(bs[1], boxed));
            format!("{} dp", design::num((nt - pb) as f64))
        }
        "Distances" | "Layout" => format!("{} dp", design::num(bs[0].left as f64)),
        "Colors" => bs[0].color.clone().unwrap_or_else(|| "—".into()),
        _ => {
            let (t, u) = design::measured(bs[0], boxed);
            format!("{} dp", design::num((u - t) as f64))
        }
    })
}

fn row_title(r: &Row, fig: &Report) -> String {
    let n = |i: usize| {
        let b = &fig.figma_blocks[r.figma_blocks[i] - 1];
        if b.label.is_empty() { format!("design block {}", b.n) } else { format!("«{}»", b.label) }
    };
    match r.category {
        "Distances" if r.figma_blocks.len() == 2 => format!("Gap {} → {}", n(0), n(1)),
        "Layout" => format!("{} cut off at the left edge", n(0)),
        "Distances" => format!("Left margin of {}", n(0)),
        "Colors" => format!("Colour of {}", n(0)),
        _ => format!("Height of {}", n(0)),
    }
}

pub fn cross(android: &str, ios: &str, out: &str) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let ia = read_json(&format!("{android}/inputs.json"))?;
    let ii = read_json(&format!("{ios}/inputs.json"))?;
    let (na, ni) = (ia["figma_name"].as_str().unwrap_or(""), ii["figma_name"].as_str().unwrap_or(""));
    if na != ni {
        return Err(format!("the two checks used different Figma frames ({na} and {ni})"));
    }
    let fpath = ia["figma"].as_str().ok_or("inputs.json: figma missing")?;
    let mut o = Opts { figma_scale: ia["figma_scale"].as_f64().unwrap_or(1.0) as f32, figma_name: na.to_string(), ..Default::default() };
    let fig = Pic::load(fpath, o.figma_scale)?;
    let fig_named = match ia["figma_meta"].as_str() {
        Some(m) => {
            let xml = std::fs::read_to_string(m).map_err(|e| format!("{m}: {e}"))?;
            o.figma_frame = design::figma_frame_name(&xml);
            design::figma_boxes(&xml)
        }
        None => Vec::new(),
    };
    let sides = [side("Android", pics::ANDROID, android, &fig, &fig_named, &o)?, side("iOS", pics::IOS, ios, &fig, &fig_named, &o)?];
    let figrep = &sides[0].rep; // the design's blocks are the same in both
    // Rows: every finding of either platform, keyed by what it checked on the design.
    let mut rows: Vec<Row> = Vec::new();
    for (si, s) in sides.iter().enumerate() {
        for f in &s.rep.findings {
            let key = (f.category, f.figma_blocks.clone());
            match rows.iter_mut().find(|r| (r.category, r.figma_blocks.clone()) == key) {
                Some(r) => r.found[si] = Some(f),
                None => {
                    let mut found = vec![None, None];
                    found[si] = Some(f);
                    rows.push(Row { category: f.category, figma_blocks: f.figma_blocks.clone(), basis: f.basis, found });
                }
            }
        }
    }
    rows.sort_by(|a, b| figrep.figma_blocks[a.figma_blocks[0] - 1].top.total_cmp(&figrep.figma_blocks[b.figma_blocks[0] - 1].top));
    std::fs::create_dir_all(out).map_err(|e| format!("{out}: {e}"))?;

    // One DESIGN | ANDROID | iOS picture per row.
    let k = 2.0f32;
    let mut pictures = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let id = format!("X{:02}", i + 1);
        let any = r.found.iter().flatten().next().unwrap();
        let fbs: Vec<&Block> = r.figma_blocks.iter().map(|&n| &figrep.figma_blocks[n - 1]).collect();
        let fwin = design::window(&fbs, fig.dp(fig.h()));
        let fval = design::side_value(any, &any.figma);
        let mut cards = vec![Card {
            title: "Design".into(),
            subtitle: design::design_subtitle(&sides[0].rep),
            color: pics::DESIGN,
            shot: design::cut(&fig, k, fwin),
            marks: design::finding_marks(&fbs, any, &any.figma, fig.dp(fig.w()), fwin.0, k, pics::DESIGN, pics::DESIGN, &fval, "design element"),
        }];
        let mut values = vec![format!("Design {fval}")];
        for (si, s) in sides.iter().enumerate() {
            let abs: Option<Vec<&Block>> = r.figma_blocks.iter().map(|&f| matched(&s.rep, f)).collect();
            let (shot, marks, v) = match (&abs, r.found[si]) {
                (Some(abs), found) => {
                    let awin = design::window(abs, s.pic.dp(s.pic.h()));
                    let (v, col) = match found {
                        Some(f) => {
                            let unit = if f.category == "Colors" { " ΔE" } else { " dp" };
                            (format!("{} ({})", design::side_value(f, &f.app), design::signed(f.delta, unit)), if f.confidence == "CERTAIN" { pics::BAD } else { pics::WARN })
                        }
                        None => (format!("{} ✓", measure(s, r).unwrap_or_else(|| "—".into())), s.color),
                    };
                    // Draw with the finding's geometry when there is one, else with a stand-in that has this side's blocks.
                    let probe = found.unwrap_or(any);
                    let side_json = found.map(|f| f.app.clone()).unwrap_or_else(|| json!({ "left": abs[0].left }));
                    let marks = design::finding_marks(abs, probe, &side_json, s.pic.dp(s.pic.w()), awin.0, k, s.color, col, &v, "app element");
                    (design::cut(&s.pic, k, awin), marks, v)
                }
                (None, _) => (design::cut(&s.pic, k, (0.0, 360.0f32.min(s.pic.dp(s.pic.h())))), vec![], "not on this screen".to_string()),
            };
            values.push(format!("{} {v}", s.name));
            cards.push(Card { title: s.name.into(), subtitle: design::app_subtitle(&s.rep), color: s.color, shot, marks });
        }
        let sure = r.found.iter().flatten().any(|f| f.confidence == "CERTAIN");
        let chips = vec![(id.clone(), pics::INK), (r.category.to_string(), pics::MUTED), ((if sure { "Certain" } else { "To confirm" }).to_string(), if sure { pics::BAD } else { pics::WARN })];
        let how = if r.category == "Distances" && r.figma_blocks.len() == 2 { if r.basis == "box" { "measured on the layout boxes (the elements' frames in each app's tree and in Figma)" } else { "measured on the visible edges (pixels)" } } else { "measured on the screen" };
        let p = format!("{out}/{id}.png");
        pics::compose(&cards, &row_title(r, figrep), &chips, &[(values.join("   ·   "), pics::INK, true), (how.to_string(), pics::MUTED, false)], &p)?;
        pictures.push((id, p, values));
    }

    // The whole screen, three ways.
    let overview = format!("{out}/cross.png");
    {
        let k = 1.5f32;
        let last = sides.iter().filter_map(|s| s.rep.matches.last().map(|&(_, f, _)| figrep.figma_blocks[f - 1].bottom + 60.0)).fold(0.0f32, f32::max);
        let fig_h = last.max(sides.iter().map(|s| s.pic.dp(s.pic.h())).fold(0.0, f32::max)).min(fig.dp(fig.h()));
        let px = |v: f32| (v * k).round() as i32;
        let outline = |b: &Block, wdp: f32, on: bool, c: Rgb<u8>, label: String| {
            let (x, y, w, h) = (px(b.left) - 3, px(b.top) - 3, px(wdp - b.left - b.margin_right) + 6, px(b.bottom - b.top) + 6);
            if on { Mark::Box { x, y, w, h, label, color: c } } else { Mark::Ghost { x, y, w, h } }
        };
        let in_any: Vec<usize> = sides.iter().flat_map(|s| s.rep.matches.iter().map(|m| m.1)).collect();
        let fmarks = figrep.figma_blocks.iter().filter(|b| b.bottom <= fig_h).map(|b| outline(b, fig.dp(fig.w()), in_any.contains(&b.n), pics::DESIGN, String::new())).collect();
        let mut cards = vec![Card { title: "Design".into(), subtitle: design::design_subtitle(&sides[0].rep), color: pics::DESIGN, shot: design::cut(&fig, k, (0.0, fig_h)), marks: fmarks }];
        for (si, s) in sides.iter().enumerate() {
            let mut marks: Vec<Mark> = s.rep.app_blocks.iter().map(|b| outline(b, s.pic.dp(s.pic.w()), s.rep.matches.iter().any(|m| m.0 == b.n), s.color, String::new())).collect();
            // One tag per element, listing every difference on it ("X02 · X03").
            let mut tagged: Vec<(usize, Vec<String>, bool)> = Vec::new();
            for (i, r) in rows.iter().enumerate() {
                if let Some(f) = r.found[si] {
                    if let Some(&a0) = f.app_blocks.first() {
                        let id = format!("X{:02}", i + 1);
                        match tagged.iter_mut().find(|t| t.0 == a0) {
                            Some(t) => {
                                t.1.push(id);
                                t.2 |= f.confidence == "CERTAIN";
                            }
                            None => tagged.push((a0, vec![id], f.confidence == "CERTAIN")),
                        }
                    }
                }
            }
            for (a0, ids, sure) in tagged {
                let b = &s.rep.app_blocks[a0 - 1];
                marks.push(outline(b, s.pic.dp(s.pic.w()), true, if sure { pics::BAD } else { pics::WARN }, ids.join(" · ")));
            }
            cards.push(Card { title: s.name.into(), subtitle: design::app_subtitle(&s.rep), color: s.color, shot: design::cut(&s.pic, k, (0.0, s.pic.dp(s.pic.h()))), marks });
        }
        let chips = vec![(format!("{} differences", rows.len()), pics::BAD)];
        pics::compose(&cards, &format!("Design check · Figma {na} · Android and iOS"), &chips, &[("Violet: the design · green: Android · blue: iOS · grey: no counterpart · red / amber tags: the differences (X01…)".into(), pics::MUTED, false)], &overview)?;
    }

    // The report section.
    let base = design::md_base(out);
    let r = |p: &str| design::rel(p, &base);
    let mut m = format!("#### Design check · Figma {na} · Android and iOS\n\n| | Design (Figma) | Android | iOS |\n|---|---|---|---|\n");
    m.push_str(&format!("| What | {} | {} | {} |\n", design::design_subtitle(&sides[0].rep), design::app_subtitle(&sides[0].rep), design::app_subtitle(&sides[1].rep)));
    m.push_str(&format!("| Matched blocks | {} | {} of {} | {} of {} |\n", figrep.figma_blocks.len(), sides[0].rep.matches.len(), sides[0].rep.app_blocks.len(), sides[1].rep.matches.len(), sides[1].rep.app_blocks.len()));
    let count = |si: usize| rows.iter().filter(|r| r.found[si].is_some()).count();
    m.push_str(&format!("| Differences | | {} | {} |\n\n", count(0), count(1)));
    m.push_str(&format!("![DESIGN | ANDROID | iOS — the whole screen]({})\n\n", r(&overview)));
    // Side by side, in the design's order.
    m.push_str("**Side by side** (top to bottom):\n\n| Design (Figma) | Android | iOS |\n|---|---|---|\n");
    let lo = figrep.figma_blocks.iter().map(|b| b.n).filter(|n| in_any_of(&sides, *n)).min().unwrap_or(1);
    let hi = figrep.figma_blocks.iter().map(|b| b.n).filter(|n| in_any_of(&sides, *n)).max().unwrap_or(0);
    for n in lo..=hi {
        let b = &figrep.figma_blocks[n - 1];
        let cell = |si: usize| -> String {
            let s = &sides[si];
            match matched(&s.rep, n) {
                Some(a) => {
                    let ids: Vec<String> = rows.iter().enumerate().filter(|(_, r)| r.figma_blocks.contains(&n) && r.found[si].is_some()).map(|(i, _)| format!("X{:02}", i + 1)).collect();
                    let name = if a.label.is_empty() { "matched".to_string() } else { format!("«{}»", a.label.replace('|', "/")) };
                    if ids.is_empty() { format!("✅ {name}") } else { format!("⚠️ {} · {name}", ids.join(", ")) }
                }
                None => "➖ not there (different content or state)".into(),
            }
        };
        let label = if b.label.is_empty() { format!("design block {} (y {}–{})", b.n, design::num(b.top as f64), design::num(b.bottom as f64)) } else { format!("«{}»", b.label.replace('|', "/")) };
        m.push_str(&format!("| {label} | {} | {} |\n", cell(0), cell(1)));
    }
    m.push('\n');
    if rows.is_empty() {
        m.push_str("No measurable difference on either platform.\n");
    } else {
        m.push_str("**Differences**\n\n| # | Check | Design | Android | iOS | Picture |\n|---|---|---|---|---|---|\n");
        for ((id, p, values), row) in pictures.iter().zip(&rows) {
            let v: Vec<String> = values.iter().map(|x| x.split_once(' ').map(|y| y.1.to_string()).unwrap_or_default()).collect();
            let mark = |si: usize, s: &str| match row.found[si] {
                Some(f) if f.confidence == "CERTAIN" => format!("⚠️ {s}"),
                Some(_) => format!("❓ {s}"),
                None => s.to_string(),
            };
            m.push_str(&format!("| {id} | {} | {} | {} | {} | [{id}]({}) |\n", row_title(row, figrep).replace('|', "/"), v[0], mark(0, &v[1]), mark(1, &v[2]), r(p)));
        }
        m.push_str("\n**Pictures** (DESIGN · ANDROID · iOS, same scale; the measured elements named, the values on each)\n\n");
        for ((id, p, _), row) in pictures.iter().zip(&rows) {
            m.push_str(&format!("![{id} — {}]({})\n\n", row_title(row, figrep), r(p)));
        }
    }
    std::fs::write(format!("{out}/cross.md"), &m).map_err(|e| format!("{out}/cross.md: {e}"))?;
    let js = json!({
        "figma": na, "differences": rows.len(),
        "android": { "findings": count(0), "matched": sides[0].rep.matches.len() },
        "ios": { "findings": count(1), "matched": sides[1].rep.matches.len() },
        "rows": pictures.iter().zip(&rows).map(|((id, p, values), row)| json!({ "id": id, "check": row_title(row, figrep), "category": row.category, "values": values, "picture": r(p) })).collect::<Vec<_>>(),
    });
    std::fs::write(format!("{out}/cross.json"), serde_json::to_string_pretty(&js).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(format!(
        "cross-platform design check (Figma {na}): {} difference(s) — Android {}, iOS {} — in {} ms\noverview: {overview}\nreport section: {out}/cross.md\n",
        rows.len(),
        count(0),
        count(1),
        t0.elapsed().as_millis()
    ))
}

fn in_any_of(sides: &[Side], n: usize) -> bool {
    sides.iter().any(|s| s.rep.matches.iter().any(|m| m.1 == n))
}
