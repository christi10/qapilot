//! Screen audit: what a careful tester checks on every screen, read off the element tree — deterministically, in
//! milliseconds, the same answer on every run (checks "by eye" found a different handful of these each time).
//!
//!   placeholder  a value that never reached the screen: `undefined`, `NaN`, `[object Object]`, `{{name}}`, a
//!                translation key shown instead of its text
//!   no-label     a tap target a screen reader announces as nothing (no text, no description, nothing inside it)
//!   small-target a tap target under 24 dp (WCAG 2.2 AA) — certain; under the platform's 48 dp / 44 pt — to look at
//!   clipped      a text that crosses the left or right edge of the screen
//!   truncated    a text cut with an ellipsis
//!   language     a text in the other language than the rest of the screen (English on a Greek screen)
//!   overlap      two tap targets on top of each other
//!
//! CERTAIN findings are facts of the tree; an UNCERTAIN one says what could explain it — look before reporting it.
use crate::screen::{El, Screen};
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    /// the finding kind a report uses: functional | accessibility | design | copy
    pub kind: &'static str,
    pub certain: bool,
    /// the element: its ref on this screen and the selector that finds it again
    pub r#ref: String,
    pub component: String,
    pub message: String,
    pub bounds: [i32; 4],
}

/// Words that are the same in every language: brands, products, banking and device terms. The app's own brand and
/// product names go in QAPILOT_SAME_WORDS (comma-separated).
const SAME_EVERYWHERE: &[&str] = &[
    "iban", "bic", "swift", "etf", "etfs", "pin", "otp", "sms", "email", "e-mail", "visa", "mastercard", "debit", "apple", "google", "pay", "wallet", "face", "touch", "id", "eur",
    "usd", "gbp", "iris", "online", "mobile", "banking", "internet", "app", "web", "ok", "vip", "atm", "pos", "qr", "wifi", "wi-fi", "pdf", "faq", "gr", "en", "viva", "revolut",
    "paypal", "s&p", "nasdaq", "msci", "world", "usa", "eu", "esg", "tech", "premium", "gold", "silver", "plus", "pro", "max", "mini", "my", "go", "one",
];

fn greek(c: char) -> bool {
    ('\u{0370}'..='\u{03ff}').contains(&c) || ('\u{1f00}'..='\u{1fff}').contains(&c)
}

fn inside(inner: &[i32; 4], outer: &[i32; 4]) -> bool {
    inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}

fn area(b: &[i32; 4]) -> i64 {
    ((b[2] - b[0]).max(0) as i64) * ((b[3] - b[1]).max(0) as i64)
}

/// A value that never reached the screen, if the text holds one.
fn placeholder(text: &str) -> Option<(&'static str, bool)> {
    let word = |w: &str| text.split(|c: char| !c.is_alphanumeric()).any(|t| t == w);
    if text.contains("[object Object]") {
        return Some(("an object printed as text (`[object Object]`)", true));
    }
    if text.contains("{{") || text.contains("}}") || text.contains("${") || text.contains("%s") || text.contains("%d") || text.contains("%@") {
        return Some(("a template placeholder that was not filled in", true));
    }
    if word("undefined") || word("NaN") || word("null") {
        return Some(("a missing value printed as text (`undefined` / `null` / `NaN`)", true));
    }
    if text.to_lowercase().contains("lorem ipsum") {
        return Some(("filler text (lorem ipsum)", true));
    }
    // `home.offers.title` / `transfer_success_title`: a translation key shown instead of its text
    let t = text.trim();
    let keyish = t.len() >= 8
        && !t.contains(char::is_whitespace)
        && !t.contains('@')
        && !t.starts_with("www.")
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        && t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && (t.matches('.').count() >= 2 || t.contains('_'))
        && !t.split('.').all(|p| p.chars().all(|c| c.is_ascii_digit()))
        && ![".gr", ".com", ".eu", ".org", ".net"].iter().any(|d| t.ends_with(d));
    keyish.then_some(("looks like a translation key shown instead of its text", false))
}

fn app_words() -> &'static [String] {
    static WORDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| std::env::var("QAPILOT_SAME_WORDS").unwrap_or_default().split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect())
}

/// Latin words of a text that are not the same in every language.
fn foreign_latin(text: &str) -> usize {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '&'))
        .filter(|w| w.chars().count() >= 3 && w.chars().all(|c| c.is_ascii_alphabetic() || c == '-'))
        .map(str::to_lowercase)
        .filter(|w| !SAME_EVERYWHERE.contains(&w.as_str()) && !app_words().contains(w))
        .count()
}

/// The findings of one screen; `scale` is device pixels per dp / pt.
pub fn audit(s: &Screen, scale: f32, ios: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    let dp = |px: i32| px as f32 / scale.max(0.1);
    let component = |e: &El| e.selector().unwrap_or_else(|| format!("{} at {},{}", if e.class.is_empty() { "element" } else { &e.class }, dp(e.bounds[0]).round(), dp(e.bounds[1]).round()));
    let mut add = |rule: &'static str, kind: &'static str, certain: bool, e: &El, message: String| out.push(Finding { rule, kind, certain, r#ref: e.r#ref.clone(), component: component(e), message, bounds: e.bounds });
    let screen_area = (s.width as i64 * s.height as i64).max(1);
    let min_target = if ios { 44.0 } else { 48.0 };

    // Which language the screen speaks: by its letters.
    let letters: Vec<char> = s.elements.iter().flat_map(|e| e.text.chars()).filter(|c| c.is_alphabetic()).collect();
    let greek_share = letters.iter().filter(|c| greek(**c)).count() as f32 / letters.len().max(1) as f32;
    let (greek_screen, latin_screen) = (letters.len() >= 40 && greek_share > 0.6, letters.len() >= 40 && greek_share < 0.1);

    for e in &s.elements {
        let (w, h) = (e.bounds[2] - e.bounds[0], e.bounds[3] - e.bounds[1]);
        for text in [&e.text, &e.desc] {
            if let Some((what, certain)) = placeholder(text) {
                add("placeholder", "functional", certain, e, format!("shows \"{}\": {what}", clip(text)));
                break;
            }
        }
        if !e.text.is_empty() {
            if s.width > 0 && (e.bounds[0] < 0 || e.bounds[2] > s.width) && e.bounds[2] > 8 && e.bounds[0] < s.width - 8 {
                let over = if e.bounds[0] < 0 { format!("{} dp past the left edge", dp(-e.bounds[0]).round()) } else { format!("{} dp past the right edge", dp(e.bounds[2] - s.width).round()) };
                add("clipped", "design", false, e, format!("\"{}\" is cut off at the screen edge ({over}) — fine in a row that scrolls sideways and rests like this only mid-scroll, a bug if it opens like this", clip(&e.text)));
            }
            let t = e.text.trim_end();
            if t.chars().count() > 3 && (t.ends_with('…') || t.ends_with("...")) {
                add("truncated", "copy", false, e, format!("\"{}\" is cut with an ellipsis — check that the design gives this text enough room", clip(&e.text)));
            }
            let has_greek = e.text.chars().any(greek);
            if greek_screen && !has_greek && foreign_latin(&e.text) >= 2 {
                add("language", "copy", false, e, format!("\"{}\" is in English on a Greek screen (a product or a person's name is fine)", clip(&e.text)));
            } else if latin_screen && has_greek && e.text.chars().filter(|c| greek(*c)).count() >= 8 && e.text.chars().any(|c| c.is_lowercase()) {
                add("language", "copy", false, e, format!("\"{}\" is in Greek on an English screen", clip(&e.text)));
            }
        }
        if !e.clickable || !e.enabled || w <= 0 || h <= 0 {
            continue;
        }
        let backdrop = area(&e.bounds) * 10 > screen_area * 6;
        if e.text.is_empty() && e.desc.is_empty() && !backdrop {
            let named_inside = s.elements.iter().any(|o| o.r#ref != e.r#ref && (!o.text.is_empty() || !o.desc.is_empty()) && inside(&o.bounds, &e.bounds));
            if !named_inside {
                add("no-label", "accessibility", true, e, format!("a tap target of {}×{} dp with no text and no accessibility label — a screen reader announces nothing for it", dp(w).round(), dp(h).round()));
            }
        }
        let small = dp(w.min(h));
        if small < 24.0 {
            add("small-target", "accessibility", true, e, format!("tap target is {}×{} dp — under the 24 dp minimum (WCAG 2.2 target size)", dp(w).round(), dp(h).round()));
        } else if small < min_target - 0.5 && !backdrop {
            add("small-target", "accessibility", false, e, format!("tap target is {}×{} dp, the platform asks for {min_target} — fine if the app extends the touch area (hitSlop does not show in the tree)", dp(w).round(), dp(h).round()));
        }
    }

    // Two tap targets on top of each other (neither holds the other).
    let taps: Vec<&El> = s.elements.iter().filter(|e| e.clickable && e.enabled && area(&e.bounds) * 10 <= screen_area * 6).collect();
    for (i, a) in taps.iter().enumerate() {
        for b in &taps[i + 1..] {
            if inside(&a.bounds, &b.bounds) || inside(&b.bounds, &a.bounds) {
                continue;
            }
            let cross = [a.bounds[0].max(b.bounds[0]), a.bounds[1].max(b.bounds[1]), a.bounds[2].min(b.bounds[2]), a.bounds[3].min(b.bounds[3])];
            if area(&cross) * 4 > area(&a.bounds).min(area(&b.bounds)) {
                add("overlap", "design", false, a, format!("overlaps the tap target {} ({}) — a tap in the shared area can reach the wrong one", b.r#ref, component(b)));
            }
        }
    }
    out
}

fn clip(t: &str) -> String {
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 60 {
        format!("{}…", t.chars().take(60).collect::<String>())
    } else {
        t
    }
}

/// The findings as Claude reads them, certain first; small tap targets that only might be a problem share one line.
pub fn render(s: &Screen, name: Option<&str>, found: &[Finding]) -> String {
    let certain = found.iter().filter(|f| f.certain).count();
    let mut out = format!("audit of screen {}{} · {} elements: {} certain, {} to look at\n", s.fingerprint, name.map(|n| format!(" \"{n}\"")).unwrap_or_default(), s.elements.len(), certain, found.len() - certain);
    let line = |f: &Finding| format!("[{} | {} | {}] {} {} — {}\n", f.kind, if f.certain { "CERTAIN" } else { "UNCERTAIN" }, f.rule, f.r#ref, f.component, f.message);
    for f in found.iter().filter(|f| f.certain) {
        out.push_str(&line(f));
    }
    let (small, rest): (Vec<&Finding>, Vec<&Finding>) = found.iter().filter(|f| !f.certain).partition(|f| f.rule == "small-target");
    for f in rest {
        out.push_str(&line(f));
    }
    if !small.is_empty() {
        out.push_str(&format!("[accessibility | UNCERTAIN | small-target] {} tap target(s) under the platform minimum (fine if the app extends the touch area): {}\n", small.len(), small.iter().map(|f| format!("{} {}", f.r#ref, f.component)).collect::<Vec<_>>().join(" · ")));
    }
    if found.is_empty() {
        out.push_str("nothing found by the tree checks (placeholders, labels, tap targets, clipping, truncation, language, overlap)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(n: usize, text: &str, desc: &str, clickable: bool, bounds: [i32; 4]) -> El {
        El { r#ref: format!("e{n}"), id: String::new(), text: text.into(), desc: desc.into(), class: "View".into(), package: "app".into(), clickable, checked: false, enabled: true, focused: false, password: false, bounds }
    }

    fn screen(elements: Vec<El>) -> Screen {
        Screen { fingerprint: "f".into(), package: "app".into(), width: 1080, height: 2400, elements, ..Default::default() }
    }

    fn rules(s: &Screen) -> Vec<(&'static str, bool, String)> {
        audit(s, 3.0, false).into_iter().map(|f| (f.rule, f.certain, f.r#ref)).collect()
    }

    #[test]
    fn values_that_never_reached_the_screen() {
        let s = screen(vec![
            el(1, "Υπόλοιπο: undefined €", "", false, [0, 0, 600, 60]),
            el(2, "home.offers.title", "", false, [0, 100, 600, 160]),
            el(3, "Γεια σου {{name}}", "", false, [0, 200, 600, 260]),
            el(4, "www.example.gr", "", false, [0, 300, 600, 360]),
            el(5, "10.0.0", "", false, [0, 400, 600, 460]),
            el(6, "Ακύρωση", "", false, [0, 500, 600, 560]),
        ]);
        assert_eq!(rules(&s), [("placeholder", true, "e1".to_string()), ("placeholder", false, "e2".into()), ("placeholder", true, "e3".into())]);
    }

    #[test]
    fn tap_targets_need_a_label_and_a_size() {
        let s = screen(vec![
            el(1, "", "", true, [0, 0, 144, 144]),                // 48 dp, nothing to announce
            el(2, "", "Κλείσιμο", true, [200, 0, 260, 60]),       // 20 dp
            el(3, "", "", true, [300, 0, 600, 144]),              // labelled by the text inside it
            el(4, "Συνέχεια", "", false, [320, 40, 580, 100]),
            el(5, "", "Πίσω", true, [700, 0, 820, 120]),          // 40 dp: under 48, to look at
            el(6, "", "", true, [0, 200, 1080, 2400]),            // a backdrop
        ]);
        assert_eq!(rules(&s), [("no-label", true, "e1".to_string()), ("small-target", true, "e2".into()), ("small-target", false, "e5".into())]);
        let text = render(&s, Some("Home"), &audit(&s, 3.0, false));
        assert!(text.starts_with("audit of screen f \"Home\" · 6 elements: 2 certain, 1 to look at\n"));
        assert!(text.contains("1 tap target(s) under the platform minimum"));
    }

    #[test]
    fn clipped_truncated_and_the_other_language() {
        let greek = "Οι επενδύσεις σου σήμερα είναι εδώ για να τις δεις όλες μαζί";
        let s = screen(vec![
            el(1, greek, "", false, [48, 300, 1000, 400]),
            el(2, "Αρχική", "", false, [-90, 100, 30, 160]),
            el(3, "Επαναλαμβανόμενη επένδ…", "", false, [48, 500, 500, 560]),
            el(4, "Discover stocks and funds", "", false, [48, 600, 800, 660]),
            el(5, "Apple Pay", "", false, [48, 700, 400, 760]),
            el(6, "My Template", "", false, [48, 800, 400, 860]),
        ]);
        assert_eq!(rules(&s), [("clipped", false, "e2".to_string()), ("truncated", false, "e3".into()), ("language", false, "e4".into())]);
    }

    #[test]
    fn overlapping_tap_targets() {
        let s = screen(vec![el(1, "Αγορά", "", true, [0, 0, 300, 150]), el(2, "Πώληση", "", true, [200, 0, 500, 150]), el(3, "Άλλο", "", true, [600, 0, 900, 150])]);
        assert_eq!(rules(&s), [("overlap", false, "e1".to_string())]);
    }

    #[test]
    fn a_clean_screen_says_so() {
        let s = screen(vec![el(1, "Συνέχεια", "", true, [48, 2000, 1032, 2168])]);
        assert!(audit(&s, 3.0, false).is_empty());
        assert!(render(&s, None, &[]).contains("nothing found by the tree checks"));
    }
}
