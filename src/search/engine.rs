//! Deterministic, offline search over the knowledge base.
//!
//! Every entry is normalized once (lowercase, accent-folded tokens per
//! field) into an inverted index: a sorted vocabulary of unique tokens, and
//! for each token the `(entry, field)` pairs where it appears. A query is
//! matched against the whole name first, then term by term: each term looks
//! up the vocabulary tokens it matches (exactly, by stem or by prefix — a
//! contiguous range of the sorted vocabulary) and only their postings are
//! visited. The cost of a keystroke depends on how many tokens match, not on
//! the size of the base. See [`super::ranking`].

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::ranking::{self, Hit, MatchKind, TieBreak};
use super::tokenizer::{
    for_each_term, is_stopword, normalize, normalize_into, search_terms, terms,
};
use crate::knowledge::template::render_default;
use crate::knowledge::{Entry, Repository};

/// Per-entry data for whole-query matching (names are short).
#[derive(Debug)]
struct Indexed {
    id: String,
    name: String,
    /// `name` as chars, for the typo check (avoids an allocation per query).
    name_chars: Vec<char>,
    name_tokens: Vec<String>,
    aliases: Vec<String>,
    /// Aliases with more than one word: index in `aliases`, word count.
    phrases: Vec<(usize, u32)>,
    tags: Vec<String>,
    entry_kind: u8,
}

/// Searchable fields, strongest first; the order breaks weight ties.
const FIELDS: [(MatchKind, usize); 7] = [
    (MatchKind::Token, 2),
    (MatchKind::Alias, 3),
    (MatchKind::Tag, 3),
    (MatchKind::Description, 3),
    (MatchKind::Option, 3),
    (MatchKind::Example, 3),
    (MatchKind::Related, 3),
];

const EXACT: u8 = 1;
const STEM: u8 = 2;
const PREFIX: u8 = 4;

/// Search index built from a [`Repository`]. Hits refer to repository
/// indices, so the engine never borrows the repository.
#[derive(Debug)]
pub struct SearchEngine {
    index: Vec<Indexed>,
    /// Unique tokens of every field, sorted.
    vocab: Vec<String>,
    /// `(entry, field)` pairs of every token, field = index in [`FIELDS`];
    /// token `t` owns `postings[offsets[t]..offsets[t + 1]]`.
    postings: Vec<(u32, u8)>,
    offsets: Vec<u32>,
}

/// Best match of one query term in one entry.
#[derive(Debug, Clone, Copy)]
struct TermHit {
    kind: MatchKind,
    weight: u32,
    exact: bool,
}

impl SearchEngine {
    pub fn new(repo: &Repository) -> Self {
        let mut builder = Builder::with_entries(repo.len());
        let index = repo
            .entries()
            .iter()
            .enumerate()
            .map(|(i, e)| builder.entry(repo, i as u32, e))
            .collect();
        let (vocab, postings, offsets) = builder.finish();
        Self {
            index,
            vocab,
            postings,
            offsets,
        }
    }

    fn postings(&self, token: u32) -> &[(u32, u8)] {
        let t = token as usize;
        &self.postings[self.offsets[t] as usize..self.offsets[t + 1] as usize]
    }

    /// Returns up to `limit` hits, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = collapse(&normalize(query));
        if q.is_empty() {
            return Vec::new();
        }
        let terms = search_terms(&q);
        let query = Query {
            text: &q,
            chars: q.chars().collect(),
        };
        let n = self.index.len();
        let mut sum = vec![0u32; n];
        let mut matched = vec![0usize; n];
        let mut best: Vec<Option<(MatchKind, bool)>> = vec![None; n];
        let mut per_term: Vec<Option<TermHit>> = vec![None; n];
        let mut touched: Vec<u32> = Vec::new();
        let mut candidates: Vec<(u32, u8)> = Vec::new();
        for term in &terms {
            self.term_matches(term, &mut candidates);
            for &(token, flags) in &candidates {
                for &(entry, field) in self.postings(token) {
                    let (kind, min_prefix) = FIELDS[field as usize];
                    let exact = flags & EXACT != 0;
                    let partial =
                        flags & STEM != 0 || (flags & PREFIX != 0 && term.len() >= min_prefix);
                    if !exact && !partial {
                        continue;
                    }
                    let hit = TermHit {
                        kind,
                        weight: kind.term_weight(exact),
                        exact,
                    };
                    let slot = &mut per_term[entry as usize];
                    match slot {
                        None => {
                            touched.push(entry);
                            *slot = Some(hit);
                        }
                        // Same rule as scanning the fields in order: a higher
                        // weight wins, ties keep the stronger field.
                        Some(cur)
                            if hit.weight > cur.weight
                                || (hit.weight == cur.weight && hit.kind < cur.kind) =>
                        {
                            *cur = hit;
                        }
                        Some(_) => {}
                    }
                }
            }
            for &entry in &touched {
                let i = entry as usize;
                if let Some(hit) = per_term[i].take() {
                    sum[i] += hit.weight;
                    matched[i] += 1;
                    if best[i].is_none_or(|(b, _)| hit.kind < b) {
                        best[i] = Some((hit.kind, hit.exact));
                    }
                }
            }
            touched.clear();
        }

        let mut hits: Vec<Hit> = self
            .index
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                score(e, &query, terms.len(), sum[i], matched[i], best[i]).map(
                    |(score, kind, strong)| Hit {
                        index: i,
                        score,
                        kind,
                        strong,
                    },
                )
            })
            .collect();
        hits.sort_by(|a, b| ranking::compare((a, self.tie(a)), (b, self.tie(b))));
        hits.truncate(limit);
        hits
    }

    /// Vocabulary tokens a term matches, with how (`EXACT`, `STEM`,
    /// `PREFIX`). Stems share at least the first three bytes with the term
    /// and prefixes start with it, so every candidate lies in the sorted
    /// range of tokens starting with the term's first bytes.
    fn term_matches(&self, term: &str, out: &mut Vec<(u32, u8)>) {
        out.clear();
        if term.len() < 2 {
            if let Ok(i) = self.vocab.binary_search_by(|t| t.as_str().cmp(term)) {
                out.push((i as u32, EXACT));
            }
            return;
        }
        let mut key_len = term.len().min(3);
        while !term.is_char_boundary(key_len) {
            key_len -= 1;
        }
        let key = &term[..key_len];
        let start = self.vocab.partition_point(|t| t.as_str() < key);
        for (i, t) in self.vocab[start..].iter().enumerate() {
            if !t.starts_with(key) {
                break;
            }
            let flags = if t == term {
                EXACT
            } else {
                let mut f = 0;
                if stem_eq(t, term) {
                    f |= STEM;
                }
                if t.starts_with(term) {
                    f |= PREFIX;
                }
                f
            };
            if flags != 0 {
                out.push(((start + i) as u32, flags));
            }
        }
    }

    fn tie(&self, hit: &Hit) -> TieBreak<'_> {
        let e = &self.index[hit.index];
        TieBreak {
            entry_kind: e.entry_kind,
            name: &e.name,
        }
    }
}

/// Collects the vocabulary and postings while entries are indexed. Built
/// once at startup, so it avoids small allocations: pairs go to one flat
/// list and field text is normalized into a reused buffer.
struct Builder {
    ids: HashMap<String, u32, BuildHasherDefault<WordHasher>>,
    /// Whether each token is a stop word, decided once per unique token.
    stop: Vec<bool>,
    /// Last `(entry, field)` recorded for each token, to skip repeats.
    last: Vec<(u32, u8)>,
    /// `(token, entry, field)` in indexing order.
    pairs: Vec<(u32, u32, u8)>,
    buf: String,
}

impl Builder {
    /// Sized from the number of entries (about 12 unique tokens and 60
    /// postings each), so the collections rarely grow while indexing.
    fn with_entries(entries: usize) -> Self {
        Self {
            ids: HashMap::with_capacity_and_hasher(entries * 12, Default::default()),
            stop: Vec::with_capacity(entries * 12),
            last: Vec::with_capacity(entries * 12),
            pairs: Vec::with_capacity(entries * 64),
            buf: String::with_capacity(4096),
        }
    }
}

/// Multiplicative hash (FxHash) for the tens of thousands of short words
/// hashed at startup. The default SipHash resists adversarial keys, which a
/// built-in vocabulary does not have, and costs several times more.
#[derive(Default)]
struct WordHasher(u64);

impl Hasher for WordHasher {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            let v = u64::from_le_bytes(*c);
            self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(K);
        }
        for &b in rest {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(K);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

impl Builder {
    fn entry(&mut self, repo: &Repository, index: u32, e: &Entry) -> Indexed {
        let name = normalize_collapsed(&e.name);
        let aliases: Vec<String> = e.aliases.iter().map(|a| normalize_collapsed(a)).collect();
        let tags: Vec<String> = e.tags.iter().map(|t| normalize(t)).collect();

        self.add_terms(index, 0, &name, false);
        for a in &aliases {
            self.add_terms(index, 1, a, true);
        }
        for t in &tags {
            self.add(index, 2, t, false);
        }

        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        push_all(&mut buf, [&e.summary]);
        push_all(&mut buf, e.description.iter());
        for s in &e.sections {
            push_all(&mut buf, [&s.title]);
            push_all(&mut buf, s.text.iter());
            push_all(&mut buf, &s.items);
            for row in &s.rows {
                push_all(&mut buf, row);
            }
        }
        self.add_terms(index, 3, &buf, true);

        buf.clear();
        for o in &e.options {
            push_all(&mut buf, o.short.iter().chain(&o.long).chain(&o.arg));
            push_all(&mut buf, [&o.description]);
        }
        self.add_terms(index, 4, &buf, true);

        buf.clear();
        for x in &e.examples {
            push_command(&mut buf, &x.command);
            push_all(&mut buf, [&x.description]);
        }
        for s in &e.steps {
            push_all(&mut buf, [&s.title, &s.why]);
            if let Some(c) = &s.command {
                push_command(&mut buf, c);
            }
        }
        self.add_terms(index, 5, &buf, true);

        buf.clear();
        for r in &e.related {
            push_all(&mut buf, [r]);
            if let Some(entry) = repo.get(r) {
                push_all(&mut buf, [&entry.name]);
            }
        }
        self.add_terms(index, 6, &buf, true);
        self.buf = buf;

        let mut name_tokens: Vec<String> = terms(&name).into_iter().map(str::to_string).collect();
        name_tokens.sort();
        name_tokens.dedup();
        let phrases = aliases
            .iter()
            .enumerate()
            .filter(|(_, a)| a.contains(' '))
            .map(|(i, a)| (i, a.split(' ').count() as u32))
            .collect();
        Indexed {
            id: e.id.clone(),
            name_chars: name.chars().collect(),
            name_tokens,
            name,
            aliases,
            phrases,
            tags,
            entry_kind: e.kind.rank(),
        }
    }

    fn add_terms(&mut self, entry: u32, field: u8, normalized: &str, drop_stopwords: bool) {
        for_each_term(normalized, |t| self.add(entry, field, t, drop_stopwords));
    }

    fn add(&mut self, entry: u32, field: u8, token: &str, drop_stopwords: bool) {
        let id = match self.ids.get(token) {
            Some(&id) => id,
            None => {
                let id = self.stop.len() as u32;
                self.ids.insert(token.to_string(), id);
                self.stop.push(is_stopword(token));
                self.last.push((u32::MAX, u8::MAX));
                id
            }
        };
        let i = id as usize;
        if (drop_stopwords && self.stop[i]) || self.last[i] == (entry, field) {
            return;
        }
        self.last[i] = (entry, field);
        self.pairs.push((id, entry, field));
    }

    /// Sorted vocabulary with its postings, laid out by token. Stop words
    /// seen only in fields that drop them have no postings and are left out.
    fn finish(self) -> (Vec<String>, Vec<(u32, u8)>, Vec<u32>) {
        let mut count = vec![0u32; self.stop.len()];
        for &(token, ..) in &self.pairs {
            count[token as usize] += 1;
        }
        let mut tokens: Vec<(String, u32)> = self
            .ids
            .into_iter()
            .filter(|(_, id)| count[*id as usize] > 0)
            .collect();
        tokens.sort_unstable();
        // Old id → position in the sorted vocabulary.
        let mut rank = vec![u32::MAX; count.len()];
        let mut offsets = Vec::with_capacity(tokens.len() + 1);
        let mut vocab = Vec::with_capacity(tokens.len());
        let mut total = 0u32;
        for (new, (token, old)) in tokens.into_iter().enumerate() {
            rank[old as usize] = new as u32;
            offsets.push(total);
            total += count[old as usize];
            vocab.push(token);
        }
        offsets.push(total);
        // Stable placement: each token keeps its pairs in indexing order.
        let mut next: Vec<u32> = offsets[..vocab.len()].to_vec();
        let mut postings = vec![(0u32, 0u8); total as usize];
        for &(token, entry, field) in &self.pairs {
            let slot = &mut next[rank[token as usize] as usize];
            postings[*slot as usize] = (entry, field);
            *slot += 1;
        }
        (vocab, postings, offsets)
    }
}

/// Appends normalized `parts`, each followed by a space.
fn push_all<'a>(buf: &mut String, parts: impl IntoIterator<Item = &'a String>) {
    for p in parts {
        normalize_into(buf, p);
        buf.push(' ');
    }
}

/// A command with its template placeholders rendered, normalized.
fn push_command(buf: &mut String, command: &str) {
    if command.contains("{{") {
        normalize_into(buf, &render_default(command));
    } else {
        normalize_into(buf, command);
    }
    buf.push(' ');
}

/// `collapse(&normalize(s))` in one pass and one allocation.
fn normalize_collapsed(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        normalize_into(&mut out, word);
    }
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A normalized query, prepared once per search.
struct Query<'a> {
    text: &'a str,
    chars: Vec<char>,
}

/// Scores one entry from its whole-query match and the per-term matches:
/// `(score, kind, strong)`, or `None` when it does not match. See
/// [`Hit::strong`].
fn score(
    e: &Indexed,
    query: &Query<'_>,
    terms: usize,
    sum: u32,
    matched: usize,
    best: Option<(MatchKind, bool)>,
) -> Option<(u32, MatchKind, bool)> {
    let term_score = ranking::combine(sum, matched, terms);
    let coverage_ok = if terms > 1 {
        ranking::enough_coverage(matched, terms)
    } else {
        matched == 1
    };
    if let Some((base, kind)) = whole_query(e, query) {
        return Some((base + term_score, kind, true));
    }
    match best {
        Some((kind, exact)) if coverage_ok => {
            // A single word is trusted in the name, aliases and tags, or as a
            // whole word of the description; several words must all be found.
            let strong = matched == terms
                && (terms > 1
                    || kind <= MatchKind::Tag
                    || (kind == MatchKind::Description && exact));
            Some((term_score, kind, strong))
        }
        _ => None,
    }
}

/// Matches of the full query against name, aliases and tags.
fn whole_query(e: &Indexed, query: &Query<'_>) -> Option<(u32, MatchKind)> {
    use MatchKind::*;
    let q = query.text;
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
    if is_typo(&query.chars, &e.name_chars) {
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
    // The query contains whole alias phrases: "quem usa a porta 8080", or a
    // pasted error message. Every phrase found is evidence, so their words
    // add up: "sudo: unable to resolve host" + "unable to resolve host" beat
    // a single "name or service not known".
    let words: u32 = e
        .phrases
        .iter()
        .filter(|(i, _)| {
            let a = &e.aliases[*i];
            a.len() <= q.len() && contains_phrase(q, a)
        })
        .map(|(_, words)| words)
        .sum();
    (words > 0).then(|| (Description.tier() + 50 * words.min(30), Alias))
}

/// `haystack` contains `needle` on word boundaries.
fn contains_phrase(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
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
/// to a prefix of the name. Queries longer than 5 characters may be 2 edits
/// away, but only if the first two letters agree: otherwise unrelated words
/// slip in (`crontab` → `contar`, `container`).
fn is_typo(qc: &[char], nc: &[char]) -> bool {
    // Checked before anything else: this runs for most entries on every key.
    if qc.len() < 2 || nc.is_empty() || qc[0] != nc[0] {
        return false;
    }
    // Numbers are values, not misspellings: 443 is not a typo of 403.
    if !qc.iter().any(|c| c.is_alphabetic()) {
        return false;
    }
    let max = if qc.len() > 5 && nc.get(1) == Some(&qc[1]) {
        2
    } else {
        1
    };
    let lo = qc.len().saturating_sub(1).max(1);
    let hi = (qc.len() + 1).min(nc.len());
    (lo..=hi).any(|len| osa_distance(qc, &nc[..len]) <= max)
}

/// Optimal string alignment distance (Levenshtein + adjacent transposition).
fn osa_distance(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    // One flat matrix: a single allocation per call.
    let w = m + 1;
    let mut d = vec![0usize; (n + 1) * w];
    for i in 0..=n {
        d[i * w] = i;
    }
    for (j, cell) in d[..w].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[(i - 1) * w + j] + 1)
                .min(d[i * w + j - 1] + 1)
                .min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = v;
        }
    }
    d[n * w + m]
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
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert!(
            !is_typo(&c("crontab"), &c("container")),
            "duas edições exigem as duas primeiras letras iguais"
        );
        assert!(
            !is_typo(&c("443"), &c("403 not found")),
            "números não são erros de digitação"
        );
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
