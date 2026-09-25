//! Deterministic, offline search over the knowledge base.
//!
//! Every entry is normalized once into an [`Indexed`] record (lowercase,
//! accent-folded tokens per field). A query is matched against the whole
//! name first, then term by term against aliases, tags, descriptions,
//! options, examples and related concepts. See [`super::ranking`].

use super::ranking::{self, Hit, MatchKind, TieBreak};
use super::tokenizer::{is_stopword, normalize, search_terms, terms};
use crate::knowledge::template::render_default;
use crate::knowledge::{Entry, Repository};

#[derive(Debug)]
struct Indexed {
    id: String,
    name: String,
    name_tokens: Vec<String>,
    aliases: Vec<String>,
    alias_tokens: Vec<String>,
    tags: Vec<String>,
    text: Vec<String>,
    options: Vec<String>,
    examples: Vec<String>,
    related: Vec<String>,
    entry_kind: u8,
}

/// Search index built from a [`Repository`]. Hits refer to repository
/// indices, so the engine never borrows the repository.
#[derive(Debug)]
pub struct SearchEngine {
    index: Vec<Indexed>,
}

impl SearchEngine {
    pub fn new(repo: &Repository) -> Self {
        let index = repo
            .entries()
            .iter()
            .map(|e| index_entry(repo, e))
            .collect();
        Self { index }
    }

    /// Returns up to `limit` hits, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = collapse(&normalize(query));
        if q.is_empty() {
            return Vec::new();
        }
        let terms = search_terms(&q);
        let mut hits: Vec<Hit> = self
            .index
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                score(e, &q, &terms).map(|(score, kind)| Hit {
                    index: i,
                    score,
                    kind,
                })
            })
            .collect();
        hits.sort_by(|a, b| ranking::compare((a, self.tie(a)), (b, self.tie(b))));
        hits.truncate(limit);
        hits
    }

    fn tie(&self, hit: &Hit) -> TieBreak<'_> {
        let e = &self.index[hit.index];
        TieBreak {
            entry_kind: e.entry_kind,
            name: &e.name,
        }
    }
}

fn index_entry(repo: &Repository, e: &Entry) -> Indexed {
    let name = collapse(&normalize(&e.name));
    let aliases: Vec<String> = e.aliases.iter().map(|a| collapse(&normalize(a))).collect();

    let mut text = String::new();
    push_all(&mut text, [&e.summary]);
    push_all(&mut text, e.description.iter());
    for s in &e.sections {
        push_all(&mut text, [&s.title]);
        push_all(&mut text, s.text.iter());
        push_all(&mut text, &s.items);
        for row in &s.rows {
            push_all(&mut text, row);
        }
    }

    let mut options = String::new();
    for o in &e.options {
        push_all(&mut options, o.short.iter().chain(&o.long).chain(&o.arg));
        push_all(&mut options, [&o.description]);
    }

    let mut examples = String::new();
    for x in &e.examples {
        push_all(&mut examples, [&render_default(&x.command), &x.description]);
    }
    for s in &e.steps {
        push_all(&mut examples, [&s.title, &s.why]);
        push_all(
            &mut examples,
            s.command
                .iter()
                .map(|c| render_default(c))
                .collect::<Vec<_>>()
                .iter(),
        );
    }

    let mut related = String::new();
    for r in &e.related {
        push_all(&mut related, [r]);
        if let Some(entry) = repo.get(r) {
            push_all(&mut related, [&entry.name]);
        }
    }

    Indexed {
        id: e.id.clone(),
        name_tokens: owned_terms(&name, false),
        alias_tokens: owned_terms(&aliases.join(" "), true),
        tags: e.tags.iter().map(|t| normalize(t)).collect(),
        text: owned_terms(&normalize(&text), true),
        options: owned_terms(&normalize(&options), true),
        examples: owned_terms(&normalize(&examples), true),
        related: owned_terms(&normalize(&related), true),
        name,
        aliases,
        entry_kind: e.kind.rank(),
    }
}

fn push_all<'a>(buf: &mut String, parts: impl IntoIterator<Item = &'a String>) {
    for p in parts {
        buf.push_str(p);
        buf.push(' ');
    }
}

fn owned_terms(normalized: &str, drop_stopwords: bool) -> Vec<String> {
    let mut out: Vec<String> = terms(normalized)
        .into_iter()
        .filter(|t| !(drop_stopwords && is_stopword(t)))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Scores one entry. Returns `None` when it does not match.
fn score(e: &Indexed, q: &str, terms: &[&str]) -> Option<(u32, MatchKind)> {
    let whole = whole_query(e, q);

    let mut sum = 0;
    let mut matched = 0;
    let mut best: Option<MatchKind> = None;
    for term in terms {
        if let Some((kind, weight)) = best_term_match(e, term) {
            sum += weight;
            matched += 1;
            best = Some(best.map_or(kind, |b| b.min(kind)));
        }
    }
    let term_score = ranking::combine(sum, matched, terms.len());
    let coverage_ok = if terms.len() > 1 {
        ranking::enough_coverage(matched, terms.len())
    } else {
        matched == 1
    };

    match (whole, best) {
        (Some((base, kind)), _) => Some((base + term_score, kind)),
        (None, Some(kind)) if coverage_ok => Some((term_score, kind)),
        _ => None,
    }
}

/// Matches of the full query against name, aliases and tags.
fn whole_query(e: &Indexed, q: &str) -> Option<(u32, MatchKind)> {
    use MatchKind::*;
    if e.name == q || e.id == q {
        return Some((Exact.tier(), Exact));
    }
    if e.name.starts_with(q) {
        return Some((Prefix.tier(), Prefix));
    }
    if e.name_tokens.len() > 1 {
        if e.name_tokens.iter().any(|t| t == q) {
            return Some((Token.tier(), Token));
        }
        if q.len() >= 3 && e.name_tokens.iter().any(|t| t.starts_with(q)) {
            return Some((Token.tier() - 500, Token));
        }
    }
    // Two letters inside longer names are noise; prefix and typo matching
    // already cover short queries.
    if q.len() >= 3 && e.name.contains(q) {
        return Some((Substring.tier(), Substring));
    }
    if is_typo(q, &e.name) {
        return Some((Fuzzy.tier(), Fuzzy));
    }
    if e.aliases.iter().any(|a| a == q) {
        return Some((Alias.tier(), Alias));
    }
    if e.tags.iter().any(|t| t == q) {
        return Some((Tag.tier(), Tag));
    }
    if q.len() >= 3 && q.contains(' ') && e.aliases.iter().any(|a| a.starts_with(q)) {
        return Some((Tag.tier() - 500, Alias));
    }
    // The query contains a whole alias phrase: "quem usa a porta 8080".
    let phrase = e
        .aliases
        .iter()
        .filter(|a| a.contains(' ') && contains_phrase(q, a))
        .map(|a| a.split(' ').count() as u32)
        .max();
    phrase.map(|words| (Description.tier() + 50 * words, Alias))
}

/// `haystack` contains `needle` on word boundaries.
fn contains_phrase(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Best field a single term matches, with its weight.
fn best_term_match(e: &Indexed, term: &str) -> Option<(MatchKind, u32)> {
    let fields: [(MatchKind, &[String], usize); 7] = [
        (MatchKind::Token, &e.name_tokens, 2),
        (MatchKind::Alias, &e.alias_tokens, 3),
        (MatchKind::Tag, &e.tags, 3),
        (MatchKind::Description, &e.text, 3),
        (MatchKind::Option, &e.options, 3),
        (MatchKind::Example, &e.examples, 3),
        (MatchKind::Related, &e.related, 3),
    ];
    let mut best: Option<(MatchKind, u32)> = None;
    for (kind, tokens, min_prefix) in fields {
        if let Some(exact) = token_match(tokens, term, min_prefix) {
            let weight = kind.term_weight(exact);
            if best.is_none_or(|(_, w)| weight > w) {
                best = Some((kind, weight));
            }
            if exact && kind == MatchKind::Token {
                break;
            }
        }
    }
    best
}

/// `Some(true)` for an exact token, `Some(false)` for a stem or prefix match.
fn token_match(tokens: &[String], term: &str, min_prefix: usize) -> Option<bool> {
    let mut partial = false;
    for t in tokens {
        if t == term {
            return Some(true);
        }
        if !partial && (stem_eq(t, term) || (term.len() >= min_prefix && t.starts_with(term))) {
            partial = true;
        }
    }
    partial.then_some(false)
}

/// Light, language-agnostic stemming: words sharing a long common prefix
/// with short, differing endings (`processo`/`processos`,
/// `modificados`/`modificar`, `testar`/`teste`).
pub fn stem_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.len() - p, b.len() - p);
    let short = a.len().min(b.len());
    if p == short && short >= 3 && ra.max(rb) <= 2 {
        return true;
    }
    p >= 4 && ra <= 3 && rb <= 3 && ra.min(rb) <= 1
}

/// Typo tolerance on the name: same first letter and an edit distance of 1
/// (2 for queries longer than 5 characters) to a prefix of the name.
fn is_typo(q: &str, name: &str) -> bool {
    let qc: Vec<char> = q.chars().collect();
    let nc: Vec<char> = name.chars().collect();
    if qc.len() < 2 || nc.is_empty() || qc[0] != nc[0] {
        return false;
    }
    let max = if qc.len() <= 5 { 1 } else { 2 };
    let lo = qc.len().saturating_sub(1).max(1);
    let hi = (qc.len() + 1).min(nc.len());
    (lo..=hi).any(|len| osa_distance(&qc, &nc[..len]) <= max)
}

/// Optimal string alignment distance (Levenshtein + adjacent transposition).
fn osa_distance(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn fixture() -> &'static (Repository, SearchEngine) {
        static F: OnceLock<(Repository, SearchEngine)> = OnceLock::new();
        F.get_or_init(|| {
            let repo = Repository::embedded().unwrap();
            let engine = SearchEngine::new(&repo);
            (repo, engine)
        })
    }

    fn ids(query: &str, n: usize) -> Vec<String> {
        let (repo, engine) = fixture();
        engine
            .search(query, n)
            .iter()
            .map(|h| repo.entry(h.index).id.clone())
            .collect()
    }

    #[test]
    fn exact_name_comes_first() {
        assert_eq!(ids("grep", 5)[0], "grep");
        assert_eq!(ids("GREP", 1)[0], "grep");
        assert_eq!(ids("curl", 1)[0], "curl");
        assert_eq!(ids("git status", 1)[0], "git-status");
    }

    #[test]
    fn prefix_gr_lists_grep_groups_and_git() {
        let got = ids("gr", 10);
        assert_eq!(got[0], "grep");
        assert_eq!(got[1], "groups");
        let git = got
            .iter()
            .position(|id| id == "git")
            .expect("git via tolerância a erros");
        assert!(git <= 4, "{got:?}");
    }

    #[test]
    fn ranking_is_deterministic() {
        for q in ["gr", "porta", "processos", "s", "http"] {
            assert_eq!(ids(q, 50), ids(q, 50), "{q}");
        }
    }

    #[test]
    fn subcommand_by_token() {
        let got = ids("status", 3);
        assert!(got.contains(&"git-status".to_string()), "{got:?}");
        let got = ids("docker comp", 1);
        assert_eq!(got[0], "docker-compose");
    }

    #[test]
    fn typo_tolerance() {
        assert_eq!(ids("gerp", 1)[0], "grep");
        assert_eq!(ids("dokcer", 1)[0], "docker");
    }

    #[test]
    fn natural_language_queries() {
        let cases = [
            ("encontrar arquivos modificados", "find-modified-files"),
            ("ver processos", "see-processes"),
            ("quem usa a porta 8080", "port-owner"),
            ("testar conexão com servidor", "test-connection"),
            ("não consigo acessar servidor", "network-troubleshooting"),
            ("desfazer alterações", "git-undo"),
            ("entrar no container", "docker-enter-container"),
            ("logs container", "docker-container-logs"),
            ("quem está usando CPU?", "cpu-usage"),
        ];
        for (q, expected) in cases {
            let got = ids(q, 3);
            assert!(got.iter().any(|id| id == expected), "{q} → {got:?}");
        }
    }

    #[test]
    fn port_query_reaches_socket_tools() {
        let (repo, engine) = fixture();
        let mut seen = std::collections::HashSet::new();
        for h in engine.search("porta 8080", 10) {
            let e = repo.entry(h.index);
            seen.insert(e.id.clone());
            seen.extend(e.related.iter().cloned());
            for x in &e.examples {
                seen.extend(x.command.split_whitespace().next().map(str::to_string));
            }
        }
        for tool in ["ss", "lsof", "nc"] {
            assert!(seen.contains(tool), "faltou {tool}: {seen:?}");
        }
    }

    #[test]
    fn accents_are_ignored() {
        assert_eq!(ids("conexao", 3), ids("conexão", 3));
    }

    #[test]
    fn stem_rules() {
        assert!(stem_eq("processos", "processo"));
        assert!(stem_eq("modificados", "modificar"));
        assert!(stem_eq("testar", "teste"));
        assert!(stem_eq("log", "logs"));
        assert!(!stem_eq("remove", "remote"));
        assert!(!stem_eq("ver", "verbose"));
    }

    #[test]
    fn edit_distance() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(osa_distance(&c("gerp"), &c("grep")), 1);
        assert_eq!(osa_distance(&c("abc"), &c("abc")), 0);
        assert_eq!(osa_distance(&c("kitten"), &c("sitting")), 3);
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(ids("   ", 10).is_empty());
    }
}
