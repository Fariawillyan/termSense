//! Regex parser producing a flat intermediate representation.
//!
//! `^[0-9]+$` → `START_ANCHOR`, `CHARACTER_CLASS("[0-9]")`, `QUANTIFIER("+")`,
//! `END_ANCHOR`. The parser is lenient: it never fails, and problems (an
//! unclosed class, a dangling quantifier) become `INVALID` tokens so partial
//! input can still be explained while the user types. It does not match
//! anything — matching is delegated to the `regex` crate.

/// Kind of a regex token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegexTokenKind {
    StartAnchor,
    EndAnchor,
    WordBoundary,
    NonWordBoundary,
    TextStart,
    TextEnd,
    AnyChar,
    Literal,
    CharacterClass,
    ShorthandClass,
    UnicodeClass,
    Quantifier,
    GroupOpen,
    GroupClose,
    Alternation,
    Backreference,
    Flags,
    Invalid,
}

impl RegexTokenKind {
    pub fn label(self) -> &'static str {
        match self {
            RegexTokenKind::StartAnchor => "START_ANCHOR",
            RegexTokenKind::EndAnchor => "END_ANCHOR",
            RegexTokenKind::WordBoundary => "WORD_BOUNDARY",
            RegexTokenKind::NonWordBoundary => "NON_WORD_BOUNDARY",
            RegexTokenKind::TextStart => "TEXT_START",
            RegexTokenKind::TextEnd => "TEXT_END",
            RegexTokenKind::AnyChar => "ANY_CHAR",
            RegexTokenKind::Literal => "LITERAL",
            RegexTokenKind::CharacterClass => "CHARACTER_CLASS",
            RegexTokenKind::ShorthandClass => "SHORTHAND_CLASS",
            RegexTokenKind::UnicodeClass => "UNICODE_CLASS",
            RegexTokenKind::Quantifier => "QUANTIFIER",
            RegexTokenKind::GroupOpen => "GROUP_OPEN",
            RegexTokenKind::GroupClose => "GROUP_CLOSE",
            RegexTokenKind::Alternation => "ALTERNATION",
            RegexTokenKind::Backreference => "BACKREFERENCE",
            RegexTokenKind::Flags => "FLAGS",
            RegexTokenKind::Invalid => "INVALID",
        }
    }

    /// Whether a quantifier may follow this token.
    pub fn quantifiable(self) -> bool {
        matches!(
            self,
            RegexTokenKind::Literal
                | RegexTokenKind::AnyChar
                | RegexTokenKind::CharacterClass
                | RegexTokenKind::ShorthandClass
                | RegexTokenKind::UnicodeClass
                | RegexTokenKind::GroupClose
                | RegexTokenKind::Backreference
        )
    }
}

/// Repetition bounds of a quantifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quant {
    pub min: u32,
    pub max: Option<u32>,
    pub lazy: bool,
    pub possessive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKind {
    Capturing(u32),
    Named(String, u32),
    NonCapturing,
    Lookahead,
    NegativeLookahead,
    Lookbehind,
    NegativeLookbehind,
    Atomic,
    /// `(?i:...)`
    WithFlags(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassItem {
    Char(char),
    Range(char, char),
    Shorthand(char),
    Posix(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharClass {
    pub negated: bool,
    pub items: Vec<ClassItem>,
}

/// Parsed details of a token, depending on its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    None,
    /// Resolved text of a literal (escapes removed).
    Literal(String),
    Class(CharClass),
    Shorthand(char),
    Unicode(String),
    Quant(Quant),
    Group(GroupKind),
    Backref(u32),
    Flags(String),
    Error(String),
}

/// One token of the intermediate representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexToken {
    pub kind: RegexTokenKind,
    /// Raw text as written in the pattern.
    pub text: String,
    pub start: usize,
    pub end: usize,
    /// Group nesting level.
    pub depth: usize,
    pub detail: Detail,
}

/// Parses `pattern` into tokens. Never fails; see the module docs.
pub fn parse(pattern: &str) -> Vec<RegexToken> {
    let mut p = Parser {
        src: pattern,
        chars: pattern.char_indices().collect(),
        i: 0,
        depth: 0,
        groups: 0,
        tokens: Vec::new(),
    };
    while p.i < p.chars.len() {
        p.step();
    }
    if p.depth > 0 {
        let end = pattern.len();
        p.tokens.push(RegexToken {
            kind: RegexTokenKind::Invalid,
            text: String::new(),
            start: end,
            end,
            depth: 0,
            detail: Detail::Error(format!("{} grupo(s) aberto(s) sem ')'", p.depth)),
        });
    }
    p.tokens
}

struct Parser<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    i: usize,
    depth: usize,
    groups: u32,
    tokens: Vec<RegexToken>,
}

impl Parser<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).map(|&(_, c)| c)
    }

    fn offset(&self, index: usize) -> usize {
        self.chars.get(index).map_or(self.src.len(), |&(o, _)| o)
    }

    /// Emits a token spanning `len` chars from the current position.
    fn emit(&mut self, kind: RegexTokenKind, len: usize, detail: Detail) {
        let start = self.offset(self.i);
        let end = self.offset(self.i + len);
        self.tokens.push(RegexToken {
            kind,
            text: self.src[start..end].to_string(),
            start,
            end,
            depth: self.depth,
            detail,
        });
        self.i += len;
    }

    fn invalid(&mut self, len: usize, msg: &str) {
        self.emit(RegexTokenKind::Invalid, len, Detail::Error(msg.to_string()));
    }

    fn step(&mut self) {
        use RegexTokenKind as K;
        let c = self.chars[self.i].1;
        match c {
            '^' => self.emit(K::StartAnchor, 1, Detail::None),
            '$' => self.emit(K::EndAnchor, 1, Detail::None),
            '.' => self.emit(K::AnyChar, 1, Detail::None),
            '|' => self.emit(K::Alternation, 1, Detail::None),
            '(' => self.group(),
            ')' => {
                if self.depth == 0 {
                    self.invalid(1, "')' sem '(' correspondente");
                } else {
                    self.depth -= 1;
                    self.emit(K::GroupClose, 1, Detail::None);
                }
            }
            '[' => self.class(),
            '\\' => self.escape(),
            '*' => self.quantifier(1, 0, None),
            '+' => self.quantifier(1, 1, None),
            '?' => self.quantifier(1, 0, Some(1)),
            '{' => match self.braces() {
                Some((len, min, max)) => self.quantifier(len, min, max),
                None => self.literal(),
            },
            _ => self.literal(),
        }
    }

    fn literal(&mut self) {
        let c = self.chars[self.i].1;
        let here = self.offset(self.i);
        // Merge runs of plain characters into one LITERAL token.
        if let Some(last) = self.tokens.last_mut()
            && last.kind == RegexTokenKind::Literal
            && last.end == here
            && last.detail == Detail::Literal(last.text.clone())
        {
            last.text.push(c);
            last.end += c.len_utf8();
            last.detail = Detail::Literal(last.text.clone());
            self.i += 1;
            return;
        }
        self.emit(RegexTokenKind::Literal, 1, Detail::Literal(c.to_string()));
    }

    fn quantifier(&mut self, len: usize, min: u32, max: Option<u32>) {
        let mut len = len;
        let (mut lazy, mut possessive) = (false, false);
        match self.peek(len) {
            Some('?') => {
                lazy = true;
                len += 1;
            }
            Some('+') => {
                possessive = true;
                len += 1;
            }
            _ => {}
        }
        match self.tokens.last().map(|t| t.kind) {
            Some(k) if k.quantifiable() => {}
            Some(RegexTokenKind::Quantifier) => {
                return self.invalid(len, "quantificador aplicado a outro quantificador");
            }
            _ => return self.invalid(len, "quantificador sem elemento anterior"),
        }
        self.split_last_literal_char();
        self.emit(
            RegexTokenKind::Quantifier,
            len,
            Detail::Quant(Quant {
                min,
                max,
                lazy,
                possessive,
            }),
        );
    }

    /// `abc+` quantifies only `c`: split it into its own literal token.
    fn split_last_literal_char(&mut self) {
        let Some(last) = self.tokens.last_mut() else {
            return;
        };
        let is_plain = matches!(&last.detail, Detail::Literal(t) if *t == last.text);
        if last.kind != RegexTokenKind::Literal || !is_plain || last.text.chars().count() < 2 {
            return;
        }
        let c = last.text.pop().expect("non-empty");
        last.end -= c.len_utf8();
        last.detail = Detail::Literal(last.text.clone());
        let (start, depth) = (last.end, last.depth);
        self.tokens.push(RegexToken {
            kind: RegexTokenKind::Literal,
            text: c.to_string(),
            start,
            end: start + c.len_utf8(),
            depth,
            detail: Detail::Literal(c.to_string()),
        });
    }

    /// `{n}`, `{n,}`, `{n,m}` → (length in chars, min, max).
    fn braces(&self) -> Option<(usize, u32, Option<u32>)> {
        let rest: String = self.chars[self.i..].iter().map(|&(_, c)| c).collect();
        let close = rest.find('}')?;
        let inner = &rest[1..close];
        let (min, max) = match inner.split_once(',') {
            Some((a, b)) => {
                let min = if a.is_empty() { 0 } else { a.parse().ok()? };
                let max = if b.is_empty() {
                    None
                } else {
                    Some(b.parse().ok()?)
                };
                if a.is_empty() && b.is_empty() {
                    return None;
                }
                (min, max)
            }
            None => {
                let n = inner.parse().ok()?;
                (n, Some(n))
            }
        };
        Some((inner.chars().count() + 2, min, max))
    }

    fn group(&mut self) {
        let kind = if self.peek(1) == Some('?') {
            let rest: String = self.chars[self.i + 2..].iter().map(|&(_, c)| c).collect();
            let special = if rest.starts_with(':') {
                Some((3, GroupKind::NonCapturing))
            } else if rest.starts_with('=') {
                Some((3, GroupKind::Lookahead))
            } else if rest.starts_with('!') {
                Some((3, GroupKind::NegativeLookahead))
            } else if rest.starts_with("<=") {
                Some((4, GroupKind::Lookbehind))
            } else if rest.starts_with("<!") {
                Some((4, GroupKind::NegativeLookbehind))
            } else if rest.starts_with('>') {
                Some((3, GroupKind::Atomic))
            } else {
                None
            };
            if let Some(s) = special {
                s
            } else if let Some(name_part) =
                rest.strip_prefix("P<").or_else(|| rest.strip_prefix('<'))
            {
                let prefix_len = if rest.starts_with("P<") { 4 } else { 3 };
                match name_part.find('>') {
                    Some(end) if end > 0 => {
                        self.groups += 1;
                        let name = name_part[..end].to_string();
                        (
                            prefix_len + name.chars().count() + 1,
                            GroupKind::Named(name, self.groups),
                        )
                    }
                    _ => {
                        return self.invalid(self.chars.len() - self.i, "nome de grupo incompleto");
                    }
                }
            } else {
                let flags: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
                    .collect();
                let after = rest[flags.len()..].chars().next();
                match after {
                    Some(')') if !flags.is_empty() => {
                        let len = flags.chars().count() + 3;
                        return self.emit(RegexTokenKind::Flags, len, Detail::Flags(flags));
                    }
                    Some(':') if !flags.is_empty() => {
                        (flags.chars().count() + 3, GroupKind::WithFlags(flags))
                    }
                    None => {
                        return self
                            .invalid(self.chars.len() - self.i, "grupo especial incompleto");
                    }
                    _ => return self.invalid(2, "tipo de grupo desconhecido"),
                }
            }
        } else {
            self.groups += 1;
            (1, GroupKind::Capturing(self.groups))
        };
        let (len, kind) = kind;
        self.emit(RegexTokenKind::GroupOpen, len, Detail::Group(kind));
        self.depth += 1;
    }

    fn class(&mut self) {
        let start = self.i;
        let mut j = self.i + 1;
        let at = |j: usize| self.chars.get(j).map(|&(_, c)| c);
        let negated = at(j) == Some('^');
        if negated {
            j += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(c) = at(j) else {
                let len = self.chars.len() - start;
                return self.invalid(len, "classe '[' sem ']'");
            };
            if c == ']' && !first {
                j += 1;
                break;
            }
            first = false;
            if c == '[' && at(j + 1) == Some(':') {
                let rest: String = self.chars[j..].iter().map(|&(_, c)| c).collect();
                if let Some(end) = rest.find(":]") {
                    items.push(ClassItem::Posix(rest[2..end].to_string()));
                    j += rest[..end + 2].chars().count();
                    continue;
                }
            }
            let (item_char, consumed) = if c == '\\' {
                match at(j + 1) {
                    Some(e @ ('d' | 'D' | 'w' | 'W' | 's' | 'S')) => {
                        items.push(ClassItem::Shorthand(e));
                        j += 2;
                        continue;
                    }
                    Some(e) => (unescape(e), 2),
                    None => ('\\', 1),
                }
            } else {
                (c, 1)
            };
            j += consumed;
            if at(j) == Some('-') && at(j + 1).is_some_and(|n| n != ']') {
                let (end, n) = match at(j + 1) {
                    Some('\\') => (at(j + 2).map(unescape).unwrap_or('\\'), 3),
                    Some(e) => (e, 2),
                    None => unreachable!(),
                };
                items.push(ClassItem::Range(item_char, end));
                j += n;
            } else {
                items.push(ClassItem::Char(item_char));
            }
        }
        self.emit(
            RegexTokenKind::CharacterClass,
            j - start,
            Detail::Class(CharClass { negated, items }),
        );
    }

    fn escape(&mut self) {
        use RegexTokenKind as K;
        let Some(e) = self.peek(1) else {
            return self.invalid(1, "'\\' no final do padrão");
        };
        match e {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                self.emit(K::ShorthandClass, 2, Detail::Shorthand(e))
            }
            'b' => self.emit(K::WordBoundary, 2, Detail::None),
            'B' => self.emit(K::NonWordBoundary, 2, Detail::None),
            'A' => self.emit(K::TextStart, 2, Detail::None),
            'z' | 'Z' => self.emit(K::TextEnd, 2, Detail::None),
            '1'..='9' => {
                let n = e.to_digit(10).unwrap_or(1);
                self.emit(K::Backreference, 2, Detail::Backref(n));
            }
            'p' | 'P' => {
                if self.peek(2) == Some('{') {
                    let rest: String = self.chars[self.i + 3..].iter().map(|&(_, c)| c).collect();
                    match rest.find('}') {
                        Some(end) => {
                            let name = rest[..end].to_string();
                            let len = name.chars().count() + 4;
                            self.emit(K::UnicodeClass, len, Detail::Unicode(name));
                        }
                        None => self.invalid(self.chars.len() - self.i, "classe Unicode sem '}'"),
                    }
                } else if let Some(n) = self.peek(2) {
                    self.emit(K::UnicodeClass, 3, Detail::Unicode(n.to_string()));
                } else {
                    self.invalid(2, "classe Unicode incompleta");
                }
            }
            'x' => {
                let rest: String = self.chars[self.i + 2..].iter().map(|&(_, c)| c).collect();
                let (hex, len) = if let Some(inner) = rest.strip_prefix('{') {
                    match inner.find('}') {
                        Some(end) => (inner[..end].to_string(), end + 4),
                        None => {
                            return self.invalid(self.chars.len() - self.i, "escape \\x{ sem '}'");
                        }
                    }
                } else {
                    (rest.chars().take(2).collect(), 4)
                };
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => self.emit(K::Literal, len, Detail::Literal(c.to_string())),
                    None => self.invalid(
                        len.min(self.chars.len() - self.i),
                        "escape hexadecimal inválido",
                    ),
                }
            }
            _ => self.emit(K::Literal, 2, Detail::Literal(unescape(e).to_string())),
        }
    }
}

fn unescape(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'f' => '\x0c',
        'v' => '\x0b',
        '0' => '\0',
        other => other,
    }
}

impl CharClass {
    /// Whether `c` belongs to the class.
    pub fn contains(&self, c: char) -> bool {
        let hit = self.items.iter().any(|item| match item {
            ClassItem::Char(x) => *x == c,
            ClassItem::Range(a, b) => (*a..=*b).contains(&c),
            ClassItem::Shorthand(s) => shorthand_contains(*s, c),
            ClassItem::Posix(name) => posix_contains(name, c),
        });
        hit != self.negated
    }
}

pub fn shorthand_contains(s: char, c: char) -> bool {
    match s {
        'd' => c.is_ascii_digit(),
        'D' => !c.is_ascii_digit(),
        'w' => c.is_alphanumeric() || c == '_',
        'W' => !(c.is_alphanumeric() || c == '_'),
        's' => c.is_whitespace(),
        'S' => !c.is_whitespace(),
        _ => false,
    }
}

fn posix_contains(name: &str, c: char) -> bool {
    match name {
        "digit" => c.is_ascii_digit(),
        "alpha" => c.is_ascii_alphabetic(),
        "alnum" => c.is_ascii_alphanumeric(),
        "upper" => c.is_ascii_uppercase(),
        "lower" => c.is_ascii_lowercase(),
        "space" => c.is_ascii_whitespace(),
        "blank" => c == ' ' || c == '\t',
        "punct" => c.is_ascii_punctuation(),
        "xdigit" => c.is_ascii_hexdigit(),
        "word" => c.is_ascii_alphanumeric() || c == '_',
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RegexTokenKind as K;

    fn kinds(p: &str) -> Vec<RegexTokenKind> {
        parse(p).iter().map(|t| t.kind).collect()
    }

    #[test]
    fn spec_pattern() {
        let t = parse("^[0-9]+$");
        let labels: Vec<_> = t.iter().map(|t| t.kind.label()).collect();
        assert_eq!(
            labels,
            [
                "START_ANCHOR",
                "CHARACTER_CLASS",
                "QUANTIFIER",
                "END_ANCHOR"
            ]
        );
        assert_eq!(t[1].text, "[0-9]");
        assert_eq!(
            t[1].detail,
            Detail::Class(CharClass {
                negated: false,
                items: vec![ClassItem::Range('0', '9')]
            })
        );
        assert_eq!(
            t[2].detail,
            Detail::Quant(Quant {
                min: 1,
                max: None,
                lazy: false,
                possessive: false
            })
        );
    }

    #[test]
    fn literals_merge_and_split_before_quantifier() {
        let t = parse("abc+");
        assert_eq!(
            t.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            ["ab", "c", "+"]
        );
        assert_eq!(kinds("ab"), [K::Literal]);
    }

    #[test]
    fn escapes_and_shorthands() {
        assert_eq!(
            kinds(r"\d\w\s\b\.\1"),
            [
                K::ShorthandClass,
                K::ShorthandClass,
                K::ShorthandClass,
                K::WordBoundary,
                K::Literal,
                K::Backreference
            ]
        );
        let t = parse(r"\.");
        assert_eq!(t[0].detail, Detail::Literal(".".into()));
    }

    #[test]
    fn groups_and_depth() {
        let t = parse("(?:a|b)(?P<ano>\\d{4})");
        let k: Vec<_> = t.iter().map(|t| (t.kind, t.depth)).collect();
        assert_eq!(
            k,
            [
                (K::GroupOpen, 0),
                (K::Literal, 1),
                (K::Alternation, 1),
                (K::Literal, 1),
                (K::GroupClose, 0),
                (K::GroupOpen, 0),
                (K::ShorthandClass, 1),
                (K::Quantifier, 1),
                (K::GroupClose, 0),
            ]
        );
        assert_eq!(
            t[5].detail,
            Detail::Group(GroupKind::Named("ano".into(), 1))
        );
        assert_eq!(
            t[7].detail,
            Detail::Quant(Quant {
                min: 4,
                max: Some(4),
                lazy: false,
                possessive: false
            })
        );
    }

    #[test]
    fn quantifier_forms() {
        let q = |p: &str| match &parse(p)[1].detail {
            Detail::Quant(q) => *q,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            q("a{2,}"),
            Quant {
                min: 2,
                max: None,
                lazy: false,
                possessive: false
            }
        );
        assert_eq!(
            q("a{2,5}"),
            Quant {
                min: 2,
                max: Some(5),
                lazy: false,
                possessive: false
            }
        );
        assert!(q("a*?").lazy);
        assert_eq!(kinds("a{x}"), [K::Literal]);
    }

    #[test]
    fn classes() {
        let t = parse(r"[^a-z_\d[:space:]]");
        let Detail::Class(c) = &t[0].detail else {
            panic!()
        };
        assert!(c.negated);
        assert_eq!(
            c.items,
            [
                ClassItem::Range('a', 'z'),
                ClassItem::Char('_'),
                ClassItem::Shorthand('d'),
                ClassItem::Posix("space".into())
            ]
        );
        assert!(c.contains('A'));
        assert!(!c.contains('b'));
        assert!(!c.contains('7'));
        assert_eq!(kinds("[]a]"), [K::CharacterClass]);
    }

    #[test]
    fn invalid_input_is_tolerated() {
        assert_eq!(kinds("[abc"), [K::Invalid]);
        assert_eq!(kinds("+a"), [K::Invalid, K::Literal]);
        assert_eq!(kinds("a)"), [K::Literal, K::Invalid]);
        assert_eq!(kinds("(a").last(), Some(&K::Invalid));
        assert_eq!(kinds("a\\"), [K::Literal, K::Invalid]);
    }

    #[test]
    fn flags_and_lookaround() {
        assert_eq!(kinds("(?i)abc"), [K::Flags, K::Literal]);
        let t = parse("(?=x)");
        assert_eq!(t[0].detail, Detail::Group(GroupKind::Lookahead));
        let t = parse("(?<!x)");
        assert_eq!(t[0].detail, Detail::Group(GroupKind::NegativeLookbehind));
    }
}
