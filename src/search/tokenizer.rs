//! Tokenizers.
//!
//! * [`tokenize`] — a shell-aware lexer for command lines. It understands
//!   quotes, escapes, pipes, redirects and substitutions, tolerates incomplete
//!   input (the user is still typing) and classifies each token by *shape*
//!   (`COMMAND`, `OPTION`, `STRING`, `PATH`...). It knows nothing about the
//!   knowledge base; semantic roles such as subcommands are resolved later by
//!   [`crate::search::context`].
//! * [`normalize`] / [`terms`] — text normalization for search: lowercase,
//!   accent folding and stop-word removal.

/// Syntactic class of a command-line token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Command,
    Option,
    /// `--`: everything after it is an argument.
    EndOfOptions,
    String,
    Path,
    Url,
    Host,
    Number,
    Variable,
    Substitution,
    Glob,
    Assignment,
    Word,
    Pipe,
    Operator,
    Redirect,
}

impl TokenKind {
    pub fn label(self) -> &'static str {
        match self {
            TokenKind::Command => "COMMAND",
            TokenKind::Option => "OPTION",
            TokenKind::EndOfOptions => "END_OF_OPTIONS",
            TokenKind::String => "STRING",
            TokenKind::Path => "PATH",
            TokenKind::Url => "URL",
            TokenKind::Host => "HOST",
            TokenKind::Number => "NUMBER",
            TokenKind::Variable => "VARIABLE",
            TokenKind::Substitution => "SUBSTITUTION",
            TokenKind::Glob => "GLOB",
            TokenKind::Assignment => "ASSIGNMENT",
            TokenKind::Word => "WORD",
            TokenKind::Pipe => "PIPE",
            TokenKind::Operator => "OPERATOR",
            TokenKind::Redirect => "REDIRECT",
        }
    }

    /// Separates simple commands (`|`, `&&`, `;`...).
    pub fn is_separator(self) -> bool {
        matches!(self, TokenKind::Pipe | TokenKind::Operator)
    }
}

/// A token with its raw text and byte span in the original line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Exactly as typed, quotes included.
    pub text: String,
    /// Quotes removed and escapes resolved.
    pub value: String,
    pub start: usize,
    pub end: usize,
    /// False when a quote or substitution is still open.
    pub complete: bool,
}

/// Splits a command line into classified tokens.
pub fn tokenize(line: &str) -> Vec<Token> {
    let mut tokens = lex(line);
    classify(&mut tokens);
    tokens
}

struct Raw {
    text_start: usize,
    text_end: usize,
    value: String,
    op: bool,
    fully_quoted: bool,
    complete: bool,
}

fn lex(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut raws: Vec<Raw> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if let Some(len) = operator_len(&line[i..]) {
            raws.push(Raw {
                text_start: i,
                text_end: i + len,
                value: line[i..i + len].to_string(),
                op: true,
                fully_quoted: false,
                complete: true,
            });
            i += len;
            continue;
        }
        if line[i..].starts_with("((") {
            // Arithmetic command `(( i++ ))`: one token, `;` and `<` included.
            let (len, ok) = balanced(&line[i..], '(', ')');
            raws.push(Raw {
                text_start: i,
                text_end: i + len,
                value: line[i..i + len].to_string(),
                op: false,
                fully_quoted: false,
                complete: ok,
            });
            i += len;
            continue;
        }
        raws.push(lex_word(line, &mut i));
    }
    raws.into_iter()
        .map(|r| Token {
            kind: if r.op {
                op_kind(&r.value)
            } else if r.fully_quoted {
                TokenKind::String
            } else {
                TokenKind::Word
            },
            text: line[r.text_start..r.text_end].to_string(),
            value: r.value,
            start: r.text_start,
            end: r.text_end,
            complete: r.complete,
        })
        .collect()
}

/// Length of a shell operator at the start of `s`, if any.
fn operator_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    // fd-prefixed redirects: 2>, 2>>, 2>&1, 1>&2
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < b.len() && matches!(b[digits], b'>' | b'<') {
        return Some(digits + redirect_len(&s[digits..]));
    }
    match b.first()? {
        b'|' => Some(if b.get(1) == Some(&b'|') { 2 } else { 1 }),
        b'&' => match b.get(1) {
            Some(b'&') => Some(2),
            Some(b'>') => Some(if b.get(2) == Some(&b'>') { 3 } else { 2 }),
            _ => Some(1),
        },
        b';' => Some(if b.get(1) == Some(&b';') { 2 } else { 1 }),
        b'>' | b'<' if !s.starts_with("<(") => Some(redirect_len(s)),
        _ => None,
    }
}

fn redirect_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut len = 1;
    if b.get(1) == Some(&b'>') || (b[0] == b'<' && b.get(1) == Some(&b'<')) {
        len = 2;
    }
    // Here-string `<<<` and tab-stripping here-document `<<-`.
    if b[0] == b'<' && len == 2 && matches!(b.get(2), Some(b'<' | b'-')) {
        return 3;
    }
    // >&1, 2>&-, >&2
    if b.get(len) == Some(&b'&') {
        len += 1;
        len += b[len..]
            .iter()
            .take_while(|c| c.is_ascii_digit() || **c == b'-')
            .count();
    }
    len
}

fn op_kind(op: &str) -> TokenKind {
    match op {
        "|" | "|&" => TokenKind::Pipe,
        "&&" | "||" | ";" | ";;" | "&" => TokenKind::Operator,
        _ => TokenKind::Redirect,
    }
}

fn lex_word(line: &str, i: &mut usize) -> Raw {
    let start = *i;
    let mut value = String::new();
    let mut complete = true;
    let mut chars = line[start..].char_indices().peekable();
    let mut end = line.len();
    let first_quote = matches!(line[start..].chars().next(), Some('\'' | '"'));
    let mut quote_segments = 0;
    let mut unquoted_chars = 0;

    while let Some(&(off, c)) = chars.peek() {
        let pos = start + off;
        if c.is_whitespace() {
            end = pos;
            break;
        }
        if matches!(c, '|' | '&' | ';' | '>' | '<') {
            if c == '<' && line[pos..].starts_with("<(") {
                // process substitution <( ... )
                let (len, ok) = balanced(&line[pos + 1..], '(', ')');
                value.push_str(&line[pos..pos + 1 + len]);
                complete &= ok;
                unquoted_chars += 1;
                advance_to(&mut chars, start, pos + 1 + len);
                continue;
            }
            end = pos;
            break;
        }
        match c {
            '\'' => {
                quote_segments += 1;
                chars.next();
                let mut closed = false;
                for (_, q) in chars.by_ref() {
                    if q == '\'' {
                        closed = true;
                        break;
                    }
                    value.push(q);
                }
                complete &= closed;
            }
            '"' => {
                quote_segments += 1;
                chars.next();
                let mut closed = false;
                while let Some((_, q)) = chars.next() {
                    match q {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some((_, e @ ('"' | '\\' | '$' | '`'))) => value.push(e),
                            Some((_, e)) => {
                                value.push('\\');
                                value.push(e);
                            }
                            None => value.push('\\'),
                        },
                        _ => value.push(q),
                    }
                }
                complete &= closed;
            }
            '\\' => {
                chars.next();
                unquoted_chars += 1;
                if let Some((_, e)) = chars.next() {
                    value.push(e);
                }
            }
            '$' if line[pos..].starts_with("$(") => {
                let (len, ok) = balanced(&line[pos + 1..], '(', ')');
                value.push_str(&line[pos..pos + 1 + len]);
                complete &= ok;
                unquoted_chars += 1;
                advance_to(&mut chars, start, pos + 1 + len);
            }
            '`' => {
                let rest = &line[pos + 1..];
                let (len, ok) = match rest.find('`') {
                    Some(j) => (j + 2, true),
                    None => (rest.len() + 1, false),
                };
                value.push_str(&line[pos..pos + len]);
                complete &= ok;
                unquoted_chars += 1;
                advance_to(&mut chars, start, pos + len);
            }
            _ => {
                unquoted_chars += 1;
                value.push(c);
                chars.next();
            }
        }
    }
    *i = end;
    Raw {
        text_start: start,
        text_end: end,
        value,
        op: false,
        fully_quoted: first_quote && quote_segments == 1 && unquoted_chars == 0,
        complete,
    }
}

/// Length (in bytes, including both delimiters) of a balanced `(...)` group
/// starting at `s[0] == open`, and whether it was closed.
fn balanced(s: &str, open: char, close: char) -> (usize, bool) {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == open => depth += 1,
            None if c == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (i + c.len_utf8(), true);
                }
            }
            None => {}
        }
    }
    (s.len(), false)
}

fn advance_to(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    start: usize,
    target: usize,
) {
    while let Some(&(off, _)) = chars.peek() {
        if start + off >= target {
            break;
        }
        chars.next();
    }
}

/// Assigns shape-based kinds. The first word of each simple command is the
/// `COMMAND`; the word after a redirect is its target `PATH`.
fn classify(tokens: &mut [Token]) {
    let mut command_position = true;
    let mut after_redirect = false;
    let mut end_of_options = false;
    for t in tokens.iter_mut() {
        if t.kind.is_separator() {
            command_position = true;
            end_of_options = false;
            continue;
        }
        if t.kind == TokenKind::Redirect {
            after_redirect = !t.text.contains('&') || t.text.ends_with('>');
            continue;
        }
        if after_redirect {
            after_redirect = false;
            if t.kind == TokenKind::Word {
                t.kind = TokenKind::Path;
            }
            continue;
        }
        if command_position {
            if t.kind == TokenKind::Word && is_assignment(&t.text) {
                t.kind = TokenKind::Assignment;
                continue;
            }
            command_position = false;
            if t.kind == TokenKind::Word {
                t.kind = TokenKind::Command;
            }
            continue;
        }
        if t.kind != TokenKind::Word {
            continue;
        }
        t.kind = if !end_of_options && t.text == "--" {
            end_of_options = true;
            TokenKind::EndOfOptions
        } else if !end_of_options && t.text.starts_with('-') && t.text.len() > 1 {
            TokenKind::Option
        } else {
            shape(&t.text)
        };
    }
}

fn is_assignment(text: &str) -> bool {
    match text.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Classifies an argument by its shape.
pub fn shape(text: &str) -> TokenKind {
    if text.starts_with("$(") || text.starts_with('`') || text.starts_with("<(") {
        return TokenKind::Substitution;
    }
    if text.starts_with('$') {
        return TokenKind::Variable;
    }
    if text.starts_with('\'') || text.starts_with('"') {
        return TokenKind::String;
    }
    if is_url(text) {
        return TokenKind::Url;
    }
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return TokenKind::Number;
    }
    if is_host(text) {
        return TokenKind::Host;
    }
    if text.contains(['*', '?']) || (text.contains('[') && text.contains(']')) {
        return TokenKind::Glob;
    }
    if text.contains('/')
        || text.starts_with('~')
        || text.starts_with('.')
        || has_file_extension(text)
    {
        return TokenKind::Path;
    }
    TokenKind::Word
}

fn is_url(text: &str) -> bool {
    match text.split_once("://") {
        Some((scheme, _)) => {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

const HOST_TLDS: &[&str] = &[
    "com",
    "org",
    "net",
    "io",
    "dev",
    "br",
    "local",
    "internal",
    "lan",
    "app",
    "cloud",
    "edu",
    "gov",
    "info",
    "me",
    "co",
    "us",
    "uk",
    "de",
    "eu",
    "localdomain",
    "example",
    "test",
];

/// `user@host`, `host:port`, IPv4 (optionally with port or prefix),
/// `localhost` and domains with a well-known TLD.
fn is_host(text: &str) -> bool {
    let host = match text.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() && !host.is_empty() && !user.contains('/') => {
            return !host.contains('/') || host.contains(':');
        }
        _ => text,
    };
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !h.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        Some(("", _)) => return false,
        _ => host,
    };
    let host = host.split_once('/').map_or(host, |(h, p)| {
        if p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() {
            h
        } else {
            ""
        }
    });
    if host.is_empty() {
        return false;
    }
    if host == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && HOST_TLDS.contains(&labels[labels.len() - 1].to_ascii_lowercase().as_str())
}

fn has_file_extension(text: &str) -> bool {
    match text.rsplit_once('.') {
        Some((name, ext)) => {
            !name.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Search normalization
// ---------------------------------------------------------------------------

/// Lowercases and folds accents: `Conexão` → `conexao`.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    normalize_into(&mut out, text);
    out
}

/// [`normalize`] appending to an existing buffer.
pub fn normalize_into(out: &mut String, text: &str) {
    for c in text.chars() {
        // Most text is ASCII: skip the general Unicode path for it.
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.extend(c.to_lowercase().map(fold));
        }
    }
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

/// Splits already-normalized text into search terms, keeping characters that
/// carry meaning in commands (`-r`, `/24`, `:8080`, `ssh-keygen`).
pub fn terms(normalized: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for_each_term(normalized, |t| out.push(t));
    out
}

/// [`terms`] without collecting: the index builder runs this over the whole
/// knowledge base at startup.
pub fn for_each_term<'a>(normalized: &'a str, mut f: impl FnMut(&'a str)) {
    let mut start = None;
    let mut emit = |raw: &'a str| {
        // Trims punctuation that only separates (`porta:`, `.env`, `$HOME`).
        let t = raw
            .trim_end_matches(['.', ':', ','])
            .trim_start_matches(['.', ':'])
            .trim_start_matches('$');
        if !t.is_empty() && t.chars().any(char::is_alphanumeric) {
            f(t);
        }
    };
    for (i, c) in normalized.char_indices() {
        let keep = c.is_alphanumeric()
            || matches!(
                c,
                '-' | '_' | '.' | '/' | ':' | '@' | '%' | '+' | '#' | '$' | '^'
            );
        match (keep, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                emit(&normalized[s..i]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        emit(&normalized[s..]);
    }
}

/// Terms minus stop words. Falls back to every term when all are stop words.
pub fn search_terms(normalized: &str) -> Vec<&str> {
    let all = terms(normalized);
    let filtered: Vec<&str> = all.iter().copied().filter(|t| !is_stopword(t)).collect();
    if filtered.is_empty() { all } else { filtered }
}

/// Portuguese and English stop words, sorted for binary search.
const STOPWORDS: &[&str] = &[
    "a", "algum", "alguma", "an", "and", "ao", "aos", "are", "as", "can", "com", "como", "da",
    "das", "de", "do", "does", "dos", "e", "em", "essa", "esse", "esta", "estao", "este", "eu",
    "fazer", "for", "how", "i", "in", "is", "isso", "isto", "me", "meu", "meus", "minha", "minhas",
    "my", "na", "nas", "no", "nos", "o", "of", "on", "or", "os", "ou", "para", "pela", "pelo",
    "por", "pra", "preciso", "quais", "qual", "que", "quero", "sao", "se", "ser", "tem", "ter",
    "the", "to", "todas", "todos", "um", "uma", "umas", "uns", "what", "which", "with",
];

/// Runs for every token while indexing, so it is a binary search.
pub fn is_stopword(term: &str) -> bool {
    STOPWORDS.binary_search(&term).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(TokenKind, String)> {
        tokenize(line)
            .into_iter()
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn spec_example() {
        let t = tokenize(r#"grep -rin "ERROR" ./logs"#);
        let got: Vec<_> = t.iter().map(|t| t.kind.label()).collect();
        assert_eq!(got, ["COMMAND", "OPTION", "STRING", "PATH"]);
        assert_eq!(t[2].value, "ERROR");
        assert_eq!(t[2].text, "\"ERROR\"");
        assert_eq!((t[3].start, t[3].end), (18, 24));
    }

    #[test]
    fn pipes_start_new_commands() {
        let t = kinds("grep ERROR app.log | sort | uniq -c");
        use TokenKind::*;
        let k: Vec<_> = t.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            k,
            [Command, Word, Path, Pipe, Command, Pipe, Command, Option]
        );
    }

    #[test]
    fn redirects_and_operators() {
        let t = kinds("make 2>&1 > build.log && echo ok; cat < in.txt 2>/dev/null");
        use TokenKind::*;
        let k: Vec<_> = t.iter().map(|(k, s)| (*k, s.as_str())).collect();
        assert_eq!(
            k,
            [
                (Command, "make"),
                (Redirect, "2>&1"),
                (Redirect, ">"),
                (Path, "build.log"),
                (Operator, "&&"),
                (Command, "echo"),
                (Word, "ok"),
                (Operator, ";"),
                (Command, "cat"),
                (Redirect, "<"),
                (Path, "in.txt"),
                (Redirect, "2>"),
                (Path, "/dev/null"),
            ]
        );
    }

    #[test]
    fn quotes_escapes_and_incomplete_input() {
        let t = tokenize(r#"echo 'a b' "c \"d\"" e\ f 'open"#);
        assert_eq!(t[1].value, "a b");
        assert_eq!(t[1].kind, TokenKind::String);
        assert_eq!(t[2].value, "c \"d\"");
        assert_eq!(t[3].value, "e f");
        assert_eq!(t[4].value, "open");
        assert!(!t[4].complete);
        assert!(t[1].complete);
    }

    #[test]
    fn mixed_quotes_are_words() {
        let t = tokenize("grep --color='auto' x");
        assert_eq!(t[1].kind, TokenKind::Option);
        assert_eq!(t[1].value, "--color=auto");
    }

    #[test]
    fn substitutions_stay_whole() {
        let t = tokenize("echo $(date +%F) `whoami` <(ls -l) $HOME");
        use TokenKind::*;
        let k: Vec<_> = t.iter().map(|t| (t.kind, t.text.as_str())).collect();
        assert_eq!(
            k,
            [
                (Command, "echo"),
                (Substitution, "$(date +%F)"),
                (Substitution, "`whoami`"),
                (Substitution, "<(ls -l)"),
                (Variable, "$HOME"),
            ]
        );
    }

    #[test]
    fn shapes() {
        assert_eq!(shape("https://example.com"), TokenKind::Url);
        assert_eq!(shape("user@host"), TokenKind::Host);
        assert_eq!(shape("192.168.0.1"), TokenKind::Host);
        assert_eq!(shape("10.0.0.0/8"), TokenKind::Host);
        assert_eq!(shape("localhost:8080"), TokenKind::Host);
        assert_eq!(shape("example.com"), TokenKind::Host);
        assert_eq!(shape("app.log"), TokenKind::Path);
        assert_eq!(shape("~/.ssh/id_ed25519"), TokenKind::Path);
        assert_eq!(shape("*.log"), TokenKind::Glob);
        assert_eq!(shape("8080"), TokenKind::Number);
        assert_eq!(shape("status"), TokenKind::Word);
        assert_eq!(shape("12.3"), TokenKind::Word);
    }

    #[test]
    fn here_strings_and_arithmetic() {
        let t = kinds("cat <<-EOF; grep x <<< \"$v\"; ((i++)); for ((i=0; i<3; i++))");
        let texts: Vec<&str> = t.iter().map(|(_, s)| s.as_str()).collect();
        assert!(texts.contains(&"<<-"), "{texts:?}");
        assert!(texts.contains(&"<<<"), "{texts:?}");
        assert!(texts.contains(&"((i++))"), "{texts:?}");
        assert!(texts.contains(&"((i=0; i<3; i++))"), "{texts:?}");
    }

    #[test]
    fn assignments_before_command() {
        let t = tokenize("LANG=C sort file");
        assert_eq!(t[0].kind, TokenKind::Assignment);
        assert_eq!(t[1].kind, TokenKind::Command);
    }

    #[test]
    fn end_of_options() {
        let t = tokenize("rm -- -arquivo");
        assert_eq!(t[1].kind, TokenKind::EndOfOptions);
        assert_eq!(t[2].kind, TokenKind::Word);
    }

    #[test]
    fn stopwords_are_sorted() {
        assert!(STOPWORDS.windows(2).all(|w| w[0] < w[1]));
        assert!(is_stopword("para") && is_stopword("the") && !is_stopword("porta"));
    }

    #[test]
    fn normalization() {
        assert_eq!(normalize("Conexão TCP"), "conexao tcp");
        assert_eq!(
            terms("quem usa a porta :8080?"),
            ["quem", "usa", "a", "porta", "8080"]
        );
        assert_eq!(search_terms("ver os processos"), ["ver", "processos"]);
        assert_eq!(search_terms("a o"), ["a", "o"]);
        assert_eq!(
            terms("grep -r /24 ssh-keygen"),
            ["grep", "-r", "/24", "ssh-keygen"]
        );
    }
}
