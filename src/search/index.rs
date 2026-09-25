//! Building the search index, and its binary form.
//!
//! The index of the built-in knowledge is built at compile time by
//! `build.rs`, with this same code, and embedded in the binary: startup only
//! decodes it. When the user has knowledge files, it is built at runtime.
//! This module depends only on the knowledge model, the template renderer
//! and the tokenizer, so `build.rs` can compile it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::knowledge::model::{Entry, EntryKind};
use crate::knowledge::template::render_default;
use crate::search::tokenizer::{for_each_term, is_stopword, normalize, normalize_into, terms};

/// Per-entry data for whole-query matching (names are short).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub id: String,
    pub name: String,
    /// `name` as chars, for the typo check (avoids an allocation per query).
    pub name_chars: Vec<char>,
    pub name_tokens: Vec<String>,
    /// Other names of the command (`mvnw`, `kubectl`), normalized.
    pub names: Vec<String>,
    pub aliases: Vec<String>,
    /// Aliases with more than one word: index in `aliases`, word count.
    pub phrases: Vec<(usize, u32)>,
    pub tags: Vec<String>,
    pub entry_kind: u8,
}

/// The inverted index: a sorted vocabulary of unique tokens and, for each
/// token, the `(entry, field)` pairs where it appears. Fields, in order:
/// name, aliases, tags, text, options, examples, related.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub entries: Vec<Indexed>,
    pub vocab: Vec<String>,
    /// Token `t` owns `postings[offsets[t]..offsets[t + 1]]`.
    pub postings: Vec<(u32, u8)>,
    pub offsets: Vec<u32>,
    /// Multi-word aliases of recipes and concepts, sorted: a line that is
    /// exactly one of them is a question, even if it starts with a command
    /// name ("java lento").
    pub topics: Vec<String>,
}

/// Indexes `entries` (in order: hits refer to these positions).
pub fn build(entries: &[Entry]) -> Index {
    let names_by_id: HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e.id.as_str(), e.name.as_str()))
        .collect();
    let mut builder = Builder::with_entries(entries.len());
    let indexed: Vec<Indexed> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| builder.entry(&names_by_id, i as u32, e))
        .collect();
    let (vocab, postings, offsets) = builder.finish();
    let topics = topics(&indexed);
    Index {
        entries: indexed,
        vocab,
        postings,
        offsets,
        topics,
    }
}

fn topics(indexed: &[Indexed]) -> Vec<String> {
    let mut topics: Vec<String> = indexed
        .iter()
        .filter(|e| e.entry_kind != EntryKind::Command.rank())
        .flat_map(|e| e.phrases.iter().map(|(i, _)| e.aliases[*i].clone()))
        .collect();
    topics.sort_unstable();
    topics.dedup();
    topics
}

/// Collects the vocabulary and postings while entries are indexed. Built
/// once, so it avoids small allocations: pairs go to one flat list and
/// field text is normalized into a reused buffer.
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

/// Multiplicative hash (FxHash) for the tens of thousands of short words
/// hashed while indexing. The default SipHash resists adversarial keys,
/// which a knowledge base does not have, and costs several times more.
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

    fn entry(&mut self, names_by_id: &HashMap<&str, &str>, index: u32, e: &Entry) -> Indexed {
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
            if let Some(name) = names_by_id.get(r.as_str()) {
                normalize_into(&mut buf, name);
                buf.push(' ');
            }
        }
        self.add_terms(index, 6, &buf, true);
        self.buf = buf;

        let names = e.names.iter().map(|n| normalize(n)).collect();
        derive(e.id.clone(), name, names, aliases, tags, e.kind.rank())
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

/// Completes an [`Indexed`] from its stored parts (the rest is derived).
fn derive(
    id: String,
    name: String,
    names: Vec<String>,
    aliases: Vec<String>,
    tags: Vec<String>,
    entry_kind: u8,
) -> Indexed {
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
        id,
        name_chars: name.chars().collect(),
        name_tokens,
        name,
        names,
        aliases,
        phrases,
        tags,
        entry_kind,
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
pub fn normalize_collapsed(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        normalize_into(&mut out, word);
    }
    out
}

// ---------------------------------------------------------------------------
// Binary form
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"TSIX";
/// Bump when the layout changes.
const FORMAT: u32 = 1;

impl Index {
    /// Little-endian, length-prefixed; derived fields are not stored.
    #[allow(dead_code)] // used by build.rs and the tests
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::with_capacity(512 * 1024));
        w.0.extend_from_slice(MAGIC);
        w.u32(FORMAT);
        w.u32(self.entries.len() as u32);
        for e in &self.entries {
            w.str(&e.id);
            w.str(&e.name);
            w.strings(&e.names);
            w.strings(&e.aliases);
            w.strings(&e.tags);
            w.0.push(e.entry_kind);
        }
        w.strings(&self.vocab);
        w.u32(self.offsets.len() as u32);
        for &o in &self.offsets {
            w.u32(o);
        }
        w.u32(self.postings.len() as u32);
        for &(entry, field) in &self.postings {
            w.u32(entry);
            w.0.push(field);
        }
        w.0
    }

    /// `None` if the bytes are not an index of this format.
    pub fn decode(bytes: &[u8]) -> Option<Index> {
        let mut r = Reader {
            data: bytes,
            pos: 0,
        };
        if r.take(4)? != MAGIC || r.u32()? != FORMAT {
            return None;
        }
        let n = r.u32()? as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let id = r.str()?;
            let name = r.str()?;
            let names = r.strings()?;
            let aliases = r.strings()?;
            let tags = r.strings()?;
            let kind = r.take(1)?[0];
            entries.push(derive(id, name, names, aliases, tags, kind));
        }
        let vocab = r.strings()?;
        let offsets = (0..r.u32()?).map(|_| r.u32()).collect::<Option<Vec<_>>>()?;
        let len = r.u32()? as usize;
        let mut postings = Vec::with_capacity(len);
        for _ in 0..len {
            postings.push((r.u32()?, r.take(1)?[0]));
        }
        let valid = r.pos == bytes.len()
            && offsets.len() == vocab.len() + 1
            && offsets
                .last()
                .is_some_and(|&t| t as usize == postings.len());
        if !valid {
            return None;
        }
        let topics = topics(&entries);
        Some(Index {
            entries,
            vocab,
            postings,
            offsets,
            topics,
        })
    }

    /// The index was built from exactly these entries (same ids, in order).
    pub fn matches(&self, entries: &[Entry]) -> bool {
        self.entries.len() == entries.len()
            && self.entries.iter().zip(entries).all(|(i, e)| i.id == e.id)
    }
}

#[allow(dead_code)] // used by build.rs and the tests
struct Writer(Vec<u8>);

#[allow(dead_code)]
impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }

    fn strings(&mut self, list: &[String]) {
        self.u32(list.len() as u32);
        for s in list {
            self.str(s);
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn strings(&mut self) -> Option<Vec<String>> {
        let n = self.u32()? as usize;
        (0..n).map(|_| self.str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Repository;

    #[test]
    fn encoding_round_trips_and_rejects_garbage() {
        let repo = Repository::embedded().unwrap();
        let index = build(repo.entries());
        let bytes = index.encode();
        assert_eq!(Index::decode(&bytes), Some(index));
        assert_eq!(Index::decode(b"TSIX"), None);
        assert_eq!(Index::decode(&bytes[..bytes.len() - 1]), None);
    }
}
