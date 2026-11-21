use crate::catalog::{CATALOG, RETIRED};
use crate::Code;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Severity {
    Error,
    Warning,
}

/// The family a code's prefix names; JSON calls it `category`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    Syntax,
    Semantic,
    Lint,
    Project,
    Verify,
    Build,
    Runtime,
    Test,
    Cli,
    Internal,
    Uncategorized,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    Active,
    /// Kept only while callers migrate off it; deleted once none remain.
    Transitional,
}

/// A minimal program that gets the code, and the same program repaired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Example {
    pub bad: &'static str,
    pub fixed: &'static str,
}

/// One catalog row.
#[non_exhaustive]
#[derive(Debug)]
pub struct CodeInfo {
    /// `"E0001"`.
    pub id: &'static str,
    /// A unique kebab-case slug, `"type-mismatch"`.
    pub name: &'static str,
    pub severity: Severity,
    /// A lowercase phrase for listings and headings.
    pub title: &'static str,
    /// What the observation means.
    pub summary: &'static str,
    /// The default repair, in the imperative. A diagnostic replaces it with
    /// an instance-specific one when it can say more.
    pub repair: &'static str,
    /// The `meshc guide` topic that teaches the surrounding feature.
    pub topic: Option<&'static str>,
    pub example: Option<Example>,
    pub status: Status,
}

pub const DOCS_BASE: &str = "https://meshlang.dev/docs/errors/";

/// What [`Code::lookup`] found for a query.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Lookup {
    Found(Code),
    /// An ID that once named a code and is never reused.
    Retired(&'static str),
    /// The codes whose ID or name is closest to the query, nearest first.
    Unknown {
        closest: Vec<Code>,
    },
}

/// How far a query may be from an ID or name to be suggested for it.
const LOOKUP_DISTANCE: usize = 2;
const LOOKUP_SUGGESTIONS: usize = 5;

impl Code {
    pub fn info(self) -> &'static CodeInfo {
        &CATALOG[self as usize]
    }

    pub fn id(self) -> &'static str {
        self.info().id
    }

    pub fn severity(self) -> Severity {
        self.info().severity
    }

    pub fn family(self) -> Family {
        let id = self.id();
        if id.starts_with("CFG") {
            return Family::Project;
        }
        match id.as_bytes()[0] {
            b'P' => Family::Syntax,
            b'E' | b'W' => Family::Semantic,
            b'L' => Family::Lint,
            b'M' => Family::Project,
            b'V' => Family::Verify,
            b'B' => Family::Build,
            b'R' => Family::Runtime,
            b'T' => Family::Test,
            b'X' => Family::Cli,
            b'I' => Family::Internal,
            b'C' => Family::Uncategorized,
            _ => unreachable!("catalog ID `{id}` has no family prefix"),
        }
    }

    /// The fragment that names the code in the skill and on the website: `e0001`.
    pub fn anchor(self) -> String {
        self.id().to_ascii_lowercase()
    }

    pub fn docs_url(self) -> String {
        format!("{DOCS_BASE}#{}", self.anchor())
    }

    pub fn explain_command(self) -> String {
        format!("meshc explain {}", self.id())
    }

    pub fn skill_ref(self) -> String {
        format!("references/diagnostics.md#{}", self.anchor())
    }

    /// Every code, in catalog order (sorted by ID).
    pub fn all() -> impl Iterator<Item = Code> {
        Code::ALL.iter().copied()
    }

    /// The code an ID or name names, ignoring case and surrounding space.
    pub fn lookup(query: &str) -> Lookup {
        let query = query.trim();
        if let Some(code) = Code::all().find(|code| {
            query.eq_ignore_ascii_case(code.id()) || query.eq_ignore_ascii_case(code.info().name)
        }) {
            return Lookup::Found(code);
        }
        if let Some(id) = RETIRED.iter().find(|id| query.eq_ignore_ascii_case(id)) {
            return Lookup::Retired(id);
        }
        let upper = query.to_ascii_uppercase();
        let lower = query.to_ascii_lowercase();
        let mut near: Vec<(usize, Code)> = Code::all()
            .map(|code| {
                let distance =
                    levenshtein(&upper, code.id()).min(levenshtein(&lower, code.info().name));
                (distance, code)
            })
            .filter(|&(distance, _)| distance <= LOOKUP_DISTANCE)
            .collect();
        // Many IDs are one edit from an ID-shaped query; the numerically
        // nearest with the same prefix are the likely ones.
        let gap = |id: &str| match (split_id(&upper), split_id(id)) {
            (Some((prefix, a)), Some((other, b))) if prefix == other => a.abs_diff(b),
            _ => u32::MAX,
        };
        near.sort_by_key(|&(distance, code)| (distance, gap(code.id())));
        Lookup::Unknown {
            closest: near
                .into_iter()
                .take(LOOKUP_SUGGESTIONS)
                .map(|(_, code)| code)
                .collect(),
        }
    }
}

/// An ID's letter prefix and number: `("E", 4)` for `E0004`.
fn split_id(id: &str) -> Option<(&str, u32)> {
    let digits = id.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    Some((&id[..id.len() - digits.len()], digits.parse().ok()?))
}

/// The candidate nearest to `target` by edit distance, if one is within
/// `max`. Ties go to the earliest candidate.
pub fn closest<'a>(
    target: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    max: usize,
) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let distance = levenshtein(target, candidate);
        if distance <= max && best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Edit distance between `a` and `b`, in Unicode scalar values.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, a) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &b) in b.iter().enumerate() {
            let cost = usize::from(a != b);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_counts_edits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("count", "cound"), 1);
        assert_eq!(levenshtein("é", "e"), 1, "counts characters, not bytes");
    }

    #[test]
    fn closest_picks_the_nearest_within_max() {
        let candidates = ["count", "counter", "amount"];
        assert_eq!(closest("cont", candidates, 2), Some("count"));
        assert_eq!(closest("count", candidates, 2), Some("count"));
        assert_eq!(closest("xyz", candidates, 2), None);
        assert_eq!(closest("cuont", candidates, 1), None);
    }

    #[test]
    fn closest_breaks_ties_by_order() {
        assert_eq!(closest("cat", ["bat", "hat"], 1), Some("bat"));
        assert_eq!(closest("cat", ["hat", "bat"], 1), Some("hat"));
    }
}
