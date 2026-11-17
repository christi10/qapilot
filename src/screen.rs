//! The screen as Claude needs it: the visible elements that carry something (text, description, id, or a tap
//! target), numbered `e1…` in reading order, plus a fingerprint that stays the same while the same screen is shown
//! (numbers, countdowns and amounts do not change it).
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct El {
    pub r#ref: String,
    pub id: String,
    pub text: String,
    pub desc: String,
    pub class: String,
    pub package: String,
    pub clickable: bool,
    pub checked: bool,
    pub enabled: bool,
    pub focused: bool,
    pub password: bool,
    pub bounds: [i32; 4],
}

impl El {
    pub fn center(&self) -> (i32, i32) {
        ((self.bounds[0] + self.bounds[2]) / 2, (self.bounds[1] + self.bounds[3]) / 2)
    }
    /// The way to find it again on a later screen: id, else its text, else its description.
    pub fn selector(&self) -> Option<String> {
        if !self.id.is_empty() {
            Some(format!("id:{}", self.id))
        } else if !self.text.is_empty() {
            Some(format!("text:{}", self.text))
        } else if !self.desc.is_empty() {
            Some(format!("desc:{}", self.desc))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Screen {
    pub fingerprint: String,
    pub package: String,
    pub width: i32,
    pub height: i32,
    pub elements: Vec<El>,
    /// elements left out because their bounds are broken (bottom above top) — Maestro/uiautomator cannot tap them
    pub broken: usize,
    /// The soft keyboard's keys when it is up (Android): its window is not the app's screen, so its ~60 nodes stay
    /// out of `elements`, the fingerprint and what Claude reads; `digits` still taps them.
    #[serde(default)]
    pub keyboard: Vec<El>,
    /// a spinner is showing: the screen is still loading
    #[serde(default)]
    pub busy: bool,
}

/// An input method's package (Gboard, the AOSP keyboard, Samsung's, SwiftKey).
fn is_keyboard(package: &str) -> bool {
    let p = package.to_lowercase();
    p.contains("inputmethod") || p.contains("honeyboard") || p.contains("swiftkey") || p.ends_with(".ime") || p.ends_with(".keyboard")
}

fn parse_bounds(s: &str) -> Option<[i32; 4]> {
    let n: Vec<i32> = s.split(|c: char| !c.is_ascii_digit() && c != '-').filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
    (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The words that identify a screen: ids, descriptions and texts with digits removed (balances, timers, dates).
fn stable(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_ascii_digit()).collect();
    t.trim().chars().take(40).collect()
}

pub fn parse(xml: &str) -> Screen {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out = Screen::default();
    let mut pkg_count: std::collections::HashMap<String, usize> = Default::default();
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"node" => {
                let mut el = El { r#ref: String::new(), id: String::new(), text: String::new(), desc: String::new(), class: String::new(), package: String::new(), clickable: false, checked: false, enabled: true, focused: false, password: false, bounds: [0; 4] };
                let mut visible = true;
                for a in e.attributes().flatten() {
                    let v = a.unescape_value().map(|c| c.into_owned()).unwrap_or_default();
                    match a.key.as_ref() {
                        b"resource-id" => el.id = v,
                        b"text" => el.text = v,
                        b"content-desc" => el.desc = v,
                        b"class" => el.class = v.rsplit('.').next().unwrap_or("").to_string(),
                        b"package" => el.package = v,
                        b"clickable" => el.clickable = v == "true",
                        b"checked" => el.checked = v == "true",
                        b"enabled" => el.enabled = v == "true",
                        b"focused" => el.focused = v == "true",
                        b"password" => el.password = v == "true",
                        b"visible-to-user" => visible = v != "false",
                        b"bounds" => el.bounds = parse_bounds(&v).unwrap_or([0; 4]),
                        _ => {}
                    }
                }
                let [l, t, rr, b] = el.bounds;
                if l == 0 && t == 0 && rr > out.width {
                    out.width = rr;
                    out.height = out.height.max(b);
                }
                if visible && rr > l && b > t && el.class == "ProgressBar" && !is_keyboard(&el.package) {
                    out.busy = true;
                }
                let meaningful = !el.text.is_empty() || !el.desc.is_empty() || !el.id.is_empty() || el.clickable;
                if !visible || !meaningful {
                    buf.clear();
                    continue;
                }
                if is_keyboard(&el.package) {
                    if rr > l && b > t && el.clickable && (!el.text.is_empty() || !el.desc.is_empty()) {
                        out.keyboard.push(el);
                    }
                    buf.clear();
                    continue;
                }
                if rr <= l || b <= t {
                    out.broken += 1;
                    buf.clear();
                    continue;
                }
                if el.package.starts_with("com.android.systemui") {
                    buf.clear();
                    continue;
                }
                *pkg_count.entry(el.package.clone()).or_default() += 1;
                out.elements.push(el);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out.package = pkg_count.into_iter().max_by_key(|(_, n)| *n).map(|(p, _)| p).unwrap_or_default();
    finish(out)
}

/// The common last step for every platform: drops wrappers that repeat a child exactly with the same bounds (React
/// Native doubles them), numbers the elements e1… and fingerprints the screen from its package and stable words.
pub fn finish(mut out: Screen) -> Screen {
    let mut seen = std::collections::HashSet::new();
    out.elements.retain(|e| seen.insert((e.id.clone(), e.text.clone(), e.desc.clone(), e.bounds)));
    for (i, e) in out.elements.iter_mut().enumerate() {
        e.r#ref = format!("e{}", i + 1);
    }
    let mut keys: Vec<String> = out.elements.iter().flat_map(|e| [&e.id, &e.desc, &e.text]).map(|k| stable(k)).filter(|s| !s.is_empty()).collect();
    keys.sort();
    keys.dedup();
    out.fingerprint = format!("{:08x}", fnv(&format!("{}|{}", out.package, keys.join("|"))) as u32);
    out
}

impl Screen {
    /// One line per element: `e12 * id | text | desc [l,t][r,b]` (`*` = tappable).
    pub fn render(&self, name: Option<&str>) -> String {
        let mut s = format!(
            "screen {}{} · {} · {} elements{}{}{}\n",
            self.fingerprint,
            name.map(|n| format!(" \"{n}\"")).unwrap_or_default(),
            if self.package.is_empty() { "?" } else { &self.package },
            self.elements.len(),
            if self.keyboard.is_empty() { "" } else { " · keyboard up" },
            if self.busy { " · loading (a spinner is showing)" } else { "" },
            if self.broken > 0 { format!(" · {} hidden (broken bounds — an overlay such as a tour may cover the app)", self.broken) } else { String::new() }
        );
        for e in &self.elements {
            let flags = format!(
                "{}{}{}{}",
                if e.checked { " [checked]" } else { "" },
                if !e.enabled { " [disabled]" } else { "" },
                if e.focused { " [focused]" } else { "" },
                if e.password { " [password]" } else { "" }
            );
            let clip = |t: &str| if t.chars().count() > 100 { format!("{}…", t.chars().take(100).collect::<String>()) } else { t.to_string() };
            s.push_str(&format!(
                "{} {} {} | {} | {} [{},{}][{},{}]{}\n",
                e.r#ref,
                if e.clickable { "*" } else { " " },
                e.id,
                clip(&e.text).replace('\n', " "),
                clip(&e.desc).replace('\n', " "),
                e.bounds[0], e.bounds[1], e.bounds[2], e.bounds[3],
                flags
            ));
        }
        s
    }

    /// Elements matching a selector: `e12` · `id:x` · `text:…` · `desc:…` · `re:<regex>` (text or desc) · plain words
    /// (case-insensitive, contained in text or desc; line breaks and repeated spaces count as one space). Exact text
    /// matches come first.
    pub fn find(&self, sel: &str) -> Result<Vec<&El>, String> {
        let sel = sel.trim();
        let lc = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if let Some(n) = sel.strip_prefix('e').filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
            return Ok(self.elements.iter().filter(|e| e.r#ref == format!("e{n}")).collect());
        }
        let mut hits: Vec<&El> = if let Some(id) = sel.strip_prefix("id:") {
            self.elements.iter().filter(|e| e.id == id || e.id.ends_with(&format!("/{id}"))).collect()
        } else if let Some(t) = sel.strip_prefix("text:") {
            self.elements.iter().filter(|e| lc(&e.text).contains(&lc(t))).collect()
        } else if let Some(t) = sel.strip_prefix("desc:") {
            self.elements.iter().filter(|e| lc(&e.desc).contains(&lc(t))).collect()
        } else if let Some(p) = sel.strip_prefix("re:") {
            let re = regex::Regex::new(&format!("(?is)^(?:{p})$")).map_err(|e| format!("bad regex: {e}"))?;
            self.elements.iter().filter(|e| re.is_match(&e.text) || re.is_match(&e.desc)).collect()
        } else {
            self.elements.iter().filter(|e| lc(&e.text).contains(&lc(sel)) || lc(&e.desc).contains(&lc(sel))).collect()
        };
        let want = lc(sel.split_once(':').map(|(_, v)| v).unwrap_or(sel));
        hits.sort_by_key(|e| !(lc(&e.text) == want || lc(&e.desc) == want));
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const XML: &str = r#"<hierarchy rotation="0">
      <node text="" resource-id="" class="android.widget.FrameLayout" package="com.example.wallet" content-desc="" clickable="false" bounds="[0,0][1280,2856]">
        <node text="Γεια σου 👋" resource-id="" class="android.widget.TextView" package="com.example.wallet" content-desc="" clickable="false" bounds="[264,228][821,312]"/>
        <node text="€ 15.331" resource-id="balance" class="android.widget.TextView" package="com.example.wallet" content-desc="" clickable="false" bounds="[72,576][435,696]"/>
        <node text="Αργότερα" resource-id="" class="android.widget.TextView" package="com.example.wallet" content-desc="" clickable="true" bounds="[120,1050][307,144]"/>
        <node text="" resource-id="" class="android.view.ViewGroup" package="com.example.wallet" content-desc="Login" clickable="true" bounds="[72,2424][1208,2544]"/>
        <node text="Login" resource-id="" class="android.widget.TextView" package="com.example.wallet" content-desc="" clickable="false" bounds="[533,2455][652,2512]"/>
      </node>
    </hierarchy>"#;

    #[test]
    fn keeps_meaningful_visible_elements_and_drops_broken_bounds() {
        let s = parse(XML);
        assert_eq!(s.package, "com.example.wallet");
        assert_eq!((s.width, s.height), (1280, 2856));
        assert_eq!(s.broken, 1);
        assert_eq!(s.elements.iter().map(|e| e.r#ref.as_str()).collect::<Vec<_>>(), ["e1", "e2", "e3", "e4"]);
        assert!(s.render(None).contains("e3 *  |  | Login"));
    }

    #[test]
    fn the_keyboard_is_not_the_screen() {
        let kb = r#"<node text="" resource-id="com.google.android.inputmethod.latin:id/key_pos_number_line1_1" class="android.view.View" package="com.google.android.inputmethod.latin" content-desc="1" clickable="true" bounds="[166,2004][482,2181]"/>
        <node text="" resource-id="android:id/inputArea" class="android.widget.FrameLayout" package="com.google.android.inputmethod.latin" content-desc="" clickable="false" bounds="[0,156][1280,2856]"/>
        <node text="" resource-id="" class="android.widget.ProgressBar" package="com.example.wallet" content-desc="" clickable="false" bounds="[600,1200][680,1280]"/>
      </node>
    </hierarchy>"#;
        let (plain, typing) = (parse(XML), parse(&XML.replace("</node>\n    </hierarchy>", kb)));
        assert_eq!(typing.package, "com.example.wallet");
        assert_eq!(typing.fingerprint, plain.fingerprint); // the same screen with the keyboard up
        assert_eq!(typing.elements.len(), plain.elements.len());
        assert_eq!(typing.keyboard.iter().map(|k| k.desc.as_str()).collect::<Vec<_>>(), ["1"]);
        assert!(typing.busy && !plain.busy);
        assert!(typing.render(None).contains("keyboard up · loading"));
    }

    #[test]
    fn fingerprint_ignores_numbers() {
        let a = parse(XML);
        let b = parse(&XML.replace("€ 15.331", "€ 99.000"));
        let c = parse(&XML.replace("Γεια σου", "Καλησπέρα"));
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_ne!(a.fingerprint, c.fingerprint);
    }

    #[test]
    fn selectors() {
        let s = parse(XML);
        assert_eq!(s.find("id:balance").unwrap()[0].r#ref, "e2");
        assert_eq!(s.find("login").unwrap()[0].r#ref, "e3"); // desc "Login" and text "Login": both, exact first
        assert_eq!(s.find("re:Γεια.*").unwrap().len(), 1);
        assert_eq!(s.find("e4").unwrap()[0].text, "Login");
        assert!(s.find("nothing here").unwrap().is_empty());
        let wrapped = parse(&XML.replace("text=\"Login\"", "text=\"Log&#10;in now\""));
        assert_eq!(wrapped.find("text:log in now").unwrap()[0].r#ref, "e4"); // a label wrapped over two lines
    }
}
