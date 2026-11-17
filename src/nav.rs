//! Per-app screen graph, learned while driving: screens by fingerprint (with a name once someone gives one), and
//! edges = the step that moved the app from one screen to another. `goto <name>` replays the shortest known path.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// a replayable step with a stable selector, e.g. `tap id:credentials-next-button` or `tap text:Invest`
    pub step: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Nav {
    #[serde(skip)]
    file: PathBuf,
    pub names: HashMap<String, String>,
    pub edges: Vec<Edge>,
}

fn dir() -> PathBuf {
    crate::home().join("nav")
}

impl Nav {
    pub fn load(package: &str) -> Nav {
        let safe: String = package.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
        let file = dir().join(format!("{}.json", if safe.is_empty() { "unknown" } else { &safe }));
        let mut n: Nav = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        n.file = file;
        n
    }

    pub fn save(&self) {
        if let Some(d) = self.file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&self.file, serde_json::to_string_pretty(self).unwrap_or_default());
    }

    pub fn name_of(&self, fp: &str) -> Option<&str> {
        self.names.get(fp).map(String::as_str)
    }

    pub fn set_name(&mut self, fp: &str, name: &str) {
        self.names.retain(|_, n| n != name); // one screen per name: the newest wins
        self.names.insert(fp.to_string(), name.to_string());
    }

    /// Remembers that `step` led from `from` to `to` (the newest step wins for the same pair).
    pub fn record(&mut self, from: &str, to: &str, step: &str) -> bool {
        if from == to || step.is_empty() {
            return false;
        }
        self.edges.retain(|e| !(e.from == from && e.to == to));
        self.edges.push(Edge { from: from.into(), to: to.into(), step: step.into() });
        true
    }

    /// Fingerprints whose name matches `target` (exact first, else contained, case-insensitive), or the fingerprint itself.
    pub fn targets(&self, target: &str) -> Vec<String> {
        let t = target.to_lowercase();
        let exact: Vec<String> = self.names.iter().filter(|(_, n)| n.to_lowercase() == t).map(|(f, _)| f.clone()).collect();
        if !exact.is_empty() {
            return exact;
        }
        let mut v: Vec<String> = self.names.iter().filter(|(_, n)| n.to_lowercase().contains(&t)).map(|(f, _)| f.clone()).collect();
        if v.is_empty() && self.edges.iter().any(|e| e.to == target || e.from == target) {
            v.push(target.to_string());
        }
        v
    }

    /// Shortest path (fewest steps) from `from` to any of `to`.
    pub fn path(&self, from: &str, to: &[String]) -> Option<Vec<Edge>> {
        if to.iter().any(|t| t == from) {
            return Some(vec![]);
        }
        let mut prev: HashMap<String, Edge> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
        let mut q = VecDeque::from([from.to_string()]);
        while let Some(cur) = q.pop_front() {
            for e in self.edges.iter().filter(|e| e.from == cur) {
                if !seen.insert(e.to.clone()) {
                    continue;
                }
                prev.insert(e.to.clone(), e.clone());
                if to.contains(&e.to) {
                    let mut out = vec![e.clone()];
                    let mut at = e.from.clone();
                    while at != from {
                        let p = prev.get(&at)?.clone();
                        at = p.from.clone();
                        out.push(p);
                    }
                    out.reverse();
                    return Some(out);
                }
                q.push_back(e.to.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortest_path_and_names() {
        let mut n = Nav::default();
        n.record("home", "invest", "tap text:Invest");
        n.record("invest", "drawer", "tap desc:Φίλτρα");
        n.record("home", "activity", "tap text:Δραστηριότητα");
        n.record("activity", "drawer", "tap desc:Φίλτρα");
        n.record("home", "home", "tap e1"); // no move: not recorded
        n.set_name("drawer", "Filters drawer");
        let t = n.targets("filters");
        assert_eq!(t, vec!["drawer".to_string()]);
        let p = n.path("home", &t).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].from, "home");
        assert_eq!(p[1].to, "drawer");
        assert!(n.path("drawer", &["home".to_string()]).is_none());
        assert_eq!(n.path("home", &["home".to_string()]).unwrap().len(), 0);
        n.record("invest", "drawer", "tap id:filter");
        assert_eq!(n.edges.iter().filter(|e| e.from == "invest" && e.to == "drawer").count(), 1);
    }
}
