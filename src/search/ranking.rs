//! Ranking rules.
//!
//! Scores are integers and ties are broken by a total order, so the same
//! query always yields the same list. Whole-query matches on the name win
//! over everything else, following the priority:
//!
//! exact → prefix → token → substring → (typo) → aliases → tags →
//! description → examples → related concepts.

use std::cmp::Ordering;

/// Why an entry matched. Ordered from strongest to weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    Exact,
    Prefix,
    Token,
    Substring,
    /// Name within a small edit distance (typo tolerance).
    Fuzzy,
    Alias,
    Tag,
    Description,
    Option,
    Example,
    Related,
}

impl MatchKind {
    /// Score for a whole-query match of this kind.
    pub fn tier(self) -> u32 {
        match self {
            MatchKind::Exact => 10_000,
            MatchKind::Prefix => 8_000,
            MatchKind::Token => 6_000,
            MatchKind::Substring => 5_000,
            MatchKind::Fuzzy => 4_500,
            MatchKind::Alias => 4_000,
            MatchKind::Tag => 3_000,
            MatchKind::Description => 2_000,
            MatchKind::Option => 1_500,
            MatchKind::Example => 1_000,
            MatchKind::Related => 500,
        }
    }

    /// Weight of a single query term matching a field of this kind; `exact`
    /// distinguishes whole-word matches from stem/prefix matches.
    pub fn term_weight(self, exact: bool) -> u32 {
        let (full, partial) = match self {
            MatchKind::Exact | MatchKind::Prefix | MatchKind::Token | MatchKind::Substring => {
                (100, 70)
            }
            MatchKind::Fuzzy => (40, 40),
            MatchKind::Alias => (60, 45),
            MatchKind::Tag => (50, 35),
            MatchKind::Description => (25, 18),
            MatchKind::Option => (20, 14),
            MatchKind::Example => (15, 10),
            MatchKind::Related => (10, 6),
        };
        if exact { full } else { partial }
    }
}

/// A scored search result pointing at a repository entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub index: usize,
    pub score: u32,
    pub kind: MatchKind,
    /// The match is trustworthy: the whole query hit the name, an alias or a
    /// tag, or every term was found. Weak hits (partial coverage, one word
    /// found only in an example) are still listed, but flagged as approximate.
    pub strong: bool,
}

/// Entry attributes used only to break ties deterministically.
#[derive(Debug, Clone, Copy)]
pub struct TieBreak<'a> {
    pub entry_kind: u8,
    pub name: &'a str,
}

/// Total order: score ↓, match kind ↑, entry kind (commands first),
/// shorter name, alphabetical name, repository index.
pub fn compare(a: (&Hit, TieBreak<'_>), b: (&Hit, TieBreak<'_>)) -> Ordering {
    let (ha, ta) = a;
    let (hb, tb) = b;
    hb.score
        .cmp(&ha.score)
        .then(ha.kind.cmp(&hb.kind))
        .then(ta.entry_kind.cmp(&tb.entry_kind))
        .then(ta.name.chars().count().cmp(&tb.name.chars().count()))
        .then(ta.name.cmp(tb.name))
        .then(ha.index.cmp(&hb.index))
}

/// Accept a multi-term match only if at least half the terms matched.
pub fn enough_coverage(matched: usize, total: usize) -> bool {
    total > 0 && matched * 2 >= total
}

/// Combines per-term weights: partial coverage is penalized quadratically so
/// entries matching every term rise to the top.
pub fn combine(sum: u32, matched: usize, total: usize) -> u32 {
    if total == 0 {
        return 0;
    }
    let (m, t) = (matched as u64, total as u64);
    (u64::from(sum) * m * m / (t * t)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(index: usize, score: u32, kind: MatchKind) -> Hit {
        Hit {
            index,
            score,
            kind,
            strong: true,
        }
    }

    fn tb(name: &str, entry_kind: u8) -> TieBreak<'_> {
        TieBreak { entry_kind, name }
    }

    #[test]
    fn tiers_follow_the_documented_priority() {
        use MatchKind::*;
        let order = [
            Exact,
            Prefix,
            Token,
            Substring,
            Fuzzy,
            Alias,
            Tag,
            Description,
            Option,
            Example,
            Related,
        ];
        for pair in order.windows(2) {
            assert!(
                pair[0].tier() > pair[1].tier(),
                "{:?} vs {:?}",
                pair[0],
                pair[1]
            );
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn ties_break_by_kind_then_length_then_name() {
        let grep = hit(3, 8000, MatchKind::Prefix);
        let groups = hit(1, 8000, MatchKind::Prefix);
        assert_eq!(
            compare((&grep, tb("grep", 0)), (&groups, tb("groups", 0))),
            Ordering::Less
        );
        let get = hit(0, 4500, MatchKind::Fuzzy);
        let git = hit(9, 4500, MatchKind::Fuzzy);
        // Commands (0) before concepts (2) even when the name sorts later.
        assert_eq!(
            compare((&git, tb("git", 0)), (&get, tb("get", 2))),
            Ordering::Less
        );
    }

    #[test]
    fn coverage_penalty() {
        assert_eq!(combine(100, 2, 2), 100);
        assert_eq!(combine(100, 1, 2), 25);
        assert!(enough_coverage(1, 2));
        assert!(!enough_coverage(1, 3));
    }
}
