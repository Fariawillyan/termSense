//! Regex analysis: token explanations, a plain-language interpretation and
//! sample strings. Samples are *generated* from the pattern structure and
//! then *verified* with the `regex` crate, so every string listed under
//! "casa" really matches and every string under "não casa" really does not.

use super::RegexAnalyzer;
use super::matcher::Matcher;
use super::parser::{
    self, CharClass, ClassItem, Detail, GroupKind, Quant, RegexToken, RegexTokenKind,
};

/// Explanation of one token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenExplanation {
    pub text: String,
    pub label: &'static str,
    pub description: String,
    pub depth: usize,
    pub invalid: bool,
}

/// Full analysis of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexAnalysis {
    pub pattern: String,
    pub tokens: Vec<TokenExplanation>,
    pub interpretation: String,
    /// Compilation error from the `regex` crate, if any.
    pub error: Option<String>,
    pub matches: Vec<String>,
    pub non_matches: Vec<String>,
    /// Portability hints (grep BRE/ERE/PCRE differences...).
    pub notes: Vec<String>,
}

/// A completion for a partially typed pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexSuggestion {
    pub snippet: String,
    pub description: String,
    /// The pattern after applying the snippet.
    pub result: String,
}

/// Default analyzer backed by [`parser`] and the `regex` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct StandardRegexAnalyzer;

const MAX_SAMPLES: usize = 4;

impl RegexAnalyzer for StandardRegexAnalyzer {
    fn parse(&self, pattern: &str) -> Vec<RegexToken> {
        parser::parse(pattern)
    }

    fn analyze(&self, pattern: &str) -> RegexAnalysis {
        let tokens = parser::parse(pattern);
        let explained = tokens.iter().map(explain_token).collect();
        let tree = build_tree(&tokens);
        let interpretation = interpret(&tokens, &tree);
        let mut analysis = RegexAnalysis {
            pattern: pattern.to_string(),
            tokens: explained,
            interpretation,
            error: None,
            matches: Vec::new(),
            non_matches: Vec::new(),
            notes: portability_notes(&tokens),
        };
        match Matcher::compile(pattern) {
            Ok(m) => {
                let (yes, no) = samples(&tokens, &tree, &m);
                analysis.matches = yes;
                analysis.non_matches = no;
            }
            Err(e) => analysis.error = Some(e),
        }
        analysis
    }

    fn suggest(&self, partial: &str) -> Vec<RegexSuggestion> {
        suggest(partial)
    }

    fn explain(&self, token: &str) -> Option<String> {
        if token.starts_with(['*', '+', '?', '{']) {
            let t = parser::parse(&format!("x{token}"));
            if t.len() == 2 && t[1].kind == RegexTokenKind::Quantifier {
                return Some(explain_token(&t[1]).description);
            }
        }
        let t: Vec<RegexToken> = parser::parse(token)
            .into_iter()
            .filter(|t| !t.text.is_empty())
            .collect();
        (t.len() == 1).then(|| explain_token(&t[0]).description)
    }
}

// ---------------------------------------------------------------------------
// Token explanations
// ---------------------------------------------------------------------------

fn explain_token(t: &RegexToken) -> TokenExplanation {
    TokenExplanation {
        text: t.text.clone(),
        label: t.kind.label(),
        description: describe(t),
        depth: t.depth,
        invalid: t.kind == RegexTokenKind::Invalid,
    }
}

fn describe(t: &RegexToken) -> String {
    use RegexTokenKind as K;
    match (&t.kind, &t.detail) {
        (K::StartAnchor, _) => "Início da string (ou de cada linha, com a flag m).".into(),
        (K::EndAnchor, _) => "Fim da string (ou de cada linha, com a flag m).".into(),
        (K::WordBoundary, _) => "Limite de palavra: fronteira entre \\w e \\W.".into(),
        (K::NonWordBoundary, _) => "Posição que não é limite de palavra.".into(),
        (K::TextStart, _) => "Início absoluto do texto (ignora a flag m).".into(),
        (K::TextEnd, _) => "Fim absoluto do texto (ignora a flag m).".into(),
        (K::AnyChar, _) => "Qualquer caractere, exceto quebra de linha.".into(),
        (K::Literal, Detail::Literal(s)) => describe_literal(s, t.text.starts_with('\\')),
        (K::CharacterClass, Detail::Class(c)) => describe_class(c),
        (K::ShorthandClass, Detail::Shorthand(s)) => describe_shorthand(*s).into(),
        (K::UnicodeClass, Detail::Unicode(name)) => {
            format!("Classe Unicode {name} (\\p{{L}} = letras, \\p{{N}} = números).")
        }
        (K::Quantifier, Detail::Quant(q)) => describe_quant(q),
        (K::GroupOpen, Detail::Group(g)) => describe_group(g),
        (K::GroupClose, _) => "Fim do grupo.".into(),
        (K::Alternation, _) => "Alternância: casa o que está à esquerda OU à direita.".into(),
        (K::Backreference, Detail::Backref(n)) => format!(
            "Retrorreferência: repete o texto capturado pelo grupo {n} (não suportado pela crate regex nem pelo grep -E; use grep -P)."
        ),
        (K::Flags, Detail::Flags(f)) => format!("Ativa flags: {}.", describe_flags(f)),
        (K::Invalid, Detail::Error(e)) => format!("Erro: {e}."),
        _ => String::new(),
    }
}

fn describe_literal(s: &str, escaped: bool) -> String {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => match c {
            ' ' => "O caractere espaço.".into(),
            '\n' => "Quebra de linha (\\n).".into(),
            '\t' => "Tabulação (\\t).".into(),
            '\r' => "Retorno de carro (\\r).".into(),
            c if escaped => format!("O caractere \"{c}\" literal (escapado com \\)."),
            c => format!("O caractere literal \"{c}\"."),
        },
        _ => format!("O texto literal \"{s}\"."),
    }
}

fn describe_shorthand(s: char) -> &'static str {
    match s {
        'd' => "Dígito (equivale a [0-9]).",
        'D' => "Qualquer caractere que não seja dígito.",
        'w' => "Caractere de palavra: letra, dígito ou _ (equivale a [A-Za-z0-9_]).",
        'W' => "Qualquer caractere que não seja de palavra.",
        's' => "Espaço em branco: espaço, tabulação ou quebra de linha.",
        'S' => "Qualquer caractere que não seja espaço em branco.",
        _ => "Classe abreviada.",
    }
}

fn describe_class(c: &CharClass) -> String {
    if let [ClassItem::Range(a, b)] = c.items.as_slice() {
        let what = match (a, b) {
            ('0', '9') => Some(format!("dígito entre {a} e {b}")),
            ('a', 'z') => Some(format!("letra minúscula entre {a} e {b}")),
            ('A', 'Z') => Some(format!("letra maiúscula entre {a} e {b}")),
            _ => None,
        };
        if let Some(what) = what {
            return if c.negated {
                format!("Qualquer caractere que não seja {what}.")
            } else {
                format!("Qualquer {what}.")
            };
        }
    }
    let items = class_items_text(c);
    if c.negated {
        format!("Qualquer caractere EXCETO: {items}.")
    } else {
        format!("Um caractere dentre: {items}.")
    }
}

fn class_items_text(c: &CharClass) -> String {
    c.items
        .iter()
        .map(|i| match i {
            ClassItem::Char(' ') => "espaço".to_string(),
            ClassItem::Char(ch) => format!("\"{ch}\""),
            ClassItem::Range(a, b) => format!("{a}–{b}"),
            ClassItem::Shorthand(s) => format!("\\{s}"),
            ClassItem::Posix(n) => format!("[:{n}:]"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_quant(q: &Quant) -> String {
    let base = match (q.min, q.max) {
        (0, Some(1)) => "Zero ou um (opcional).".to_string(),
        (0, None) => "Zero ou mais.".to_string(),
        (1, None) => "Um ou mais.".to_string(),
        (n, None) => format!("{n} ou mais vezes."),
        (n, Some(m)) if n == m => format!("Exatamente {n} vez(es)."),
        (n, Some(m)) => format!("Entre {n} e {m} vezes."),
    };
    if q.lazy {
        format!("{base} Preguiçoso: casa o mínimo possível.")
    } else if q.possessive {
        format!("{base} Possessivo: não devolve caracteres (não suportado pela crate regex).")
    } else {
        base
    }
}

fn describe_group(g: &GroupKind) -> String {
    match g {
        GroupKind::Capturing(n) => format!("Início do grupo de captura nº {n}."),
        GroupKind::Named(name, n) => {
            format!("Início do grupo de captura nº {n}, nomeado \"{name}\".")
        }
        GroupKind::NonCapturing => {
            "Início de grupo não capturante: agrupa sem guardar o texto.".into()
        }
        GroupKind::Lookahead => {
            "Lookahead: exige que o trecho à frente case, sem consumi-lo (use grep -P).".into()
        }
        GroupKind::NegativeLookahead => {
            "Lookahead negativo: exige que o trecho à frente NÃO case (use grep -P).".into()
        }
        GroupKind::Lookbehind => {
            "Lookbehind: exige que o trecho anterior case (use grep -P).".into()
        }
        GroupKind::NegativeLookbehind => {
            "Lookbehind negativo: exige que o trecho anterior NÃO case (use grep -P).".into()
        }
        GroupKind::Atomic => "Grupo atômico: não permite backtracking (use grep -P).".into(),
        GroupKind::WithFlags(f) => format!("Grupo com flags: {}.", describe_flags(f)),
    }
}

fn describe_flags(flags: &str) -> String {
    let mut out = Vec::new();
    let mut negate = false;
    for f in flags.chars() {
        let text = match f {
            '-' => {
                negate = true;
                continue;
            }
            'i' => "ignora maiúsculas/minúsculas",
            'm' => "multilinha (^ e $ em cada linha)",
            's' => "ponto casa quebra de linha",
            'x' => "modo verboso (ignora espaços)",
            'U' => "inverte a gulosidade",
            'u' => "Unicode",
            _ => "flag desconhecida",
        };
        out.push(if negate {
            format!("desativa {text}")
        } else {
            text.to_string()
        });
    }
    out.join(", ")
}

// ---------------------------------------------------------------------------
// Structure (alternatives → sequence of quantified items)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Item {
    atom: Atom,
    quant: Option<Quant>,
}

#[derive(Debug, Clone)]
enum Atom {
    Token(usize),
    Group(GroupKind, Vec<Vec<Item>>),
}

type Alternatives = Vec<Vec<Item>>;

fn build_tree(tokens: &[RegexToken]) -> Alternatives {
    let mut pos = 0;
    let mut alts = parse_alts(tokens, &mut pos);
    // Stray closers are INVALID tokens already; keep going after them.
    while pos < tokens.len() {
        pos += 1;
        let more = parse_alts(tokens, &mut pos);
        alts.last_mut()
            .expect("non-empty")
            .extend(more.into_iter().flatten());
    }
    alts
}

fn parse_alts(tokens: &[RegexToken], pos: &mut usize) -> Alternatives {
    use RegexTokenKind as K;
    let mut alts: Alternatives = vec![Vec::new()];
    while *pos < tokens.len() {
        let t = &tokens[*pos];
        match (&t.kind, &t.detail) {
            (K::Alternation, _) => {
                alts.push(Vec::new());
                *pos += 1;
            }
            (K::GroupClose, _) => return alts,
            (K::GroupOpen, Detail::Group(kind)) => {
                *pos += 1;
                let inner = parse_alts(tokens, pos);
                if *pos < tokens.len() && tokens[*pos].kind == K::GroupClose {
                    *pos += 1;
                }
                current(&mut alts).push(Item {
                    atom: Atom::Group(kind.clone(), inner),
                    quant: None,
                });
            }
            (K::Quantifier, Detail::Quant(q)) => {
                if let Some(last) = current(&mut alts).last_mut() {
                    last.quant = Some(*q);
                }
                *pos += 1;
            }
            (K::Flags | K::Invalid, _) => *pos += 1,
            _ => {
                current(&mut alts).push(Item {
                    atom: Atom::Token(*pos),
                    quant: None,
                });
                *pos += 1;
            }
        }
    }
    alts
}

fn current(alts: &mut Alternatives) -> &mut Vec<Item> {
    alts.last_mut().expect("alternatives are never empty")
}

// ---------------------------------------------------------------------------
// Interpretation
// ---------------------------------------------------------------------------

/// Noun used to describe a character-level atom, with grammatical gender.
struct Noun {
    singular: String,
    plural: String,
    feminine: bool,
}

impl Noun {
    fn new(singular: &str, plural: &str, feminine: bool) -> Self {
        Self {
            singular: singular.into(),
            plural: plural.into(),
            feminine,
        }
    }
}

fn interpret(tokens: &[RegexToken], alts: &Alternatives) -> String {
    if tokens.is_empty() {
        return "Padrão vazio: casa em qualquer posição de qualquer texto.".into();
    }
    if alts.len() > 1 {
        let parts: Vec<String> = alts
            .iter()
            .enumerate()
            .map(|(i, seq)| format!("({}) {}", i + 1, seq_phrase(tokens, seq)))
            .collect();
        return format!(
            "A expressão casa qualquer uma das alternativas: {}.",
            parts.join("; ")
        );
    }
    let seq = &alts[0];
    let is_anchor = |item: &Item, kinds: &[RegexTokenKind]| {
        matches!(item.atom, Atom::Token(i) if kinds.contains(&tokens[i].kind))
            && item.quant.is_none()
    };
    let start = seq
        .first()
        .is_some_and(|i| is_anchor(i, &[RegexTokenKind::StartAnchor, RegexTokenKind::TextStart]));
    let end = seq.len() > usize::from(start)
        && seq
            .last()
            .is_some_and(|i| is_anchor(i, &[RegexTokenKind::EndAnchor, RegexTokenKind::TextEnd]));
    let core = &seq[usize::from(start)..seq.len() - usize::from(end)];

    if start && end {
        if let [item] = core
            && let (Atom::Token(i), Some(q)) = (&item.atom, &item.quant)
            && let Some(noun) = noun(&tokens[*i])
        {
            let um = if noun.feminine { "uma" } else { "um" };
            let only = format!(
                "A expressão aceita uma string composta somente por {}",
                noun.plural
            );
            match (q.min, q.max) {
                (1, None) => {
                    return format!("{only}, contendo pelo menos {um} {}.", noun.singular);
                }
                (0, None) => return format!("{only}, ou a string vazia."),
                (n, Some(m)) if n == m => {
                    return format!(
                        "A expressão aceita uma string com exatamente {n} {}.",
                        noun.plural
                    );
                }
                (n, Some(m)) => {
                    return format!("{only}, com {n} a {m} caracteres.");
                }
                (n, None) => return format!("{only}, com pelo menos {n} caracteres."),
            }
        }
        if core.is_empty() {
            return "A expressão aceita apenas a string vazia.".into();
        }
    }
    let body = seq_phrase(tokens, core);
    match (start, end) {
        (true, true) => {
            format!("A expressão aceita somente strings formadas, do início ao fim, por: {body}.")
        }
        (true, false) => format!("A expressão procura textos que comecem com: {body}."),
        (false, true) => format!("A expressão procura textos que terminem com: {body}."),
        (false, false) => format!("A expressão procura, em qualquer parte do texto: {body}."),
    }
}

fn seq_phrase(tokens: &[RegexToken], seq: &[Item]) -> String {
    let parts: Vec<String> = seq.iter().map(|i| item_phrase(tokens, i)).collect();
    if parts.is_empty() {
        "nada (vazio)".into()
    } else {
        parts.join(", seguido de ")
    }
}

fn item_phrase(tokens: &[RegexToken], item: &Item) -> String {
    match &item.atom {
        Atom::Token(i) => {
            let t = &tokens[*i];
            if let Some(noun) = noun(t) {
                return quantified_noun(&noun, item.quant.as_ref());
            }
            let base = match (&t.kind, &t.detail) {
                (RegexTokenKind::Literal, Detail::Literal(s)) => format!("\"{}\"", printable(s)),
                (RegexTokenKind::StartAnchor, _) => "início da linha".into(),
                (RegexTokenKind::EndAnchor, _) => "fim da linha".into(),
                (RegexTokenKind::WordBoundary, _) => "um limite de palavra".into(),
                (RegexTokenKind::NonWordBoundary, _) => {
                    "uma posição fora de limite de palavra".into()
                }
                (RegexTokenKind::TextStart, _) => "início do texto".into(),
                (RegexTokenKind::TextEnd, _) => "fim do texto".into(),
                (RegexTokenKind::Backreference, Detail::Backref(n)) => {
                    format!("o mesmo texto capturado pelo grupo {n}")
                }
                _ => format!("\"{}\"", t.text),
            };
            times(base, item.quant.as_ref())
        }
        Atom::Group(kind, alts) => {
            let inner = alts
                .iter()
                .map(|s| seq_phrase(tokens, s))
                .collect::<Vec<_>>()
                .join(" OU ");
            let base = match kind {
                GroupKind::Capturing(n) => format!("o grupo {n} ({inner})"),
                GroupKind::Named(name, _) => format!("o grupo \"{name}\" ({inner})"),
                GroupKind::Lookahead => format!("(desde que venha a seguir: {inner})"),
                GroupKind::NegativeLookahead => format!("(desde que NÃO venha a seguir: {inner})"),
                GroupKind::Lookbehind => format!("(desde que venha antes: {inner})"),
                GroupKind::NegativeLookbehind => format!("(desde que NÃO venha antes: {inner})"),
                _ => format!("({inner})"),
            };
            times(base, item.quant.as_ref())
        }
    }
}

fn quantified_noun(n: &Noun, q: Option<&Quant>) -> String {
    let um = if n.feminine { "uma" } else { "um" };
    let Some(q) = q else {
        return format!("{um} {}", n.singular);
    };
    let text = match (q.min, q.max) {
        (0, Some(1)) => format!("opcionalmente {um} {}", n.singular),
        (0, None) => format!("zero ou mais {}", n.plural),
        (1, None) => format!("{um} ou mais {}", n.plural),
        (m, None) => format!("{m} ou mais {}", n.plural),
        (1, Some(1)) => format!("{um} {}", n.singular),
        (a, Some(b)) if a == b => format!("exatamente {a} {}", n.plural),
        (a, Some(b)) => format!("de {a} a {b} {}", n.plural),
    };
    if q.lazy {
        format!("{text} (o mínimo possível)")
    } else {
        text
    }
}

fn times(base: String, q: Option<&Quant>) -> String {
    let Some(q) = q else { return base };
    let text = match (q.min, q.max) {
        (0, Some(1)) => format!("opcionalmente {base}"),
        (0, None) => format!("{base} zero ou mais vezes"),
        (1, None) => format!("{base} uma ou mais vezes"),
        (m, None) => format!("{base} pelo menos {m} vezes"),
        (a, Some(b)) if a == b => format!("{base} exatamente {a} vez(es)"),
        (a, Some(b)) => format!("{base} de {a} a {b} vezes"),
    };
    if q.lazy {
        format!("{text} (o mínimo possível)")
    } else {
        text
    }
}

fn printable(s: &str) -> String {
    s.replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

fn noun(t: &RegexToken) -> Option<Noun> {
    match &t.detail {
        Detail::Shorthand(s) => Some(match s {
            'd' => Noun::new("dígito", "dígitos", false),
            'D' => Noun::new(
                "caractere que não é dígito",
                "caracteres que não são dígitos",
                false,
            ),
            'w' => Noun::new("caractere de palavra", "caracteres de palavra", false),
            'W' => Noun::new(
                "caractere que não é de palavra",
                "caracteres que não são de palavra",
                false,
            ),
            's' => Noun::new("espaço em branco", "espaços em branco", false),
            _ => Noun::new(
                "caractere que não é espaço",
                "caracteres que não são espaço",
                false,
            ),
        }),
        Detail::Unicode(name) => Some(Noun::new(
            &format!("caractere da classe Unicode {name}"),
            &format!("caracteres da classe Unicode {name}"),
            false,
        )),
        Detail::Class(c) => Some(class_noun(c)),
        Detail::None if t.kind == RegexTokenKind::AnyChar => Some(Noun::new(
            "caractere qualquer",
            "caracteres quaisquer",
            false,
        )),
        _ => None,
    }
}

fn class_noun(c: &CharClass) -> Noun {
    let mut ranges: Vec<String> = c
        .items
        .iter()
        .map(|i| match i {
            ClassItem::Range(a, b) => format!("{a}-{b}"),
            ClassItem::Char(ch) => ch.to_string(),
            ClassItem::Shorthand(s) => format!("\\{s}"),
            ClassItem::Posix(n) => format!("[:{n}:]"),
        })
        .collect();
    ranges.sort();
    let key = ranges.join(" ");
    let known = match key.as_str() {
        "0-9" | "\\d" | "[:digit:]" => Some(Noun::new("dígito", "dígitos", false)),
        "a-z" | "[:lower:]" => Some(Noun::new("letra minúscula", "letras minúsculas", true)),
        "A-Z" | "[:upper:]" => Some(Noun::new("letra maiúscula", "letras maiúsculas", true)),
        "A-Z a-z" | "[:alpha:]" => Some(Noun::new("letra", "letras", true)),
        "0-9 A-Z a-z" | "[:alnum:]" => {
            Some(Noun::new("letra ou dígito", "letras ou dígitos", true))
        }
        "0-9 A-F a-f" | "0-9 a-f" | "[:xdigit:]" => Some(Noun::new(
            "dígito hexadecimal",
            "dígitos hexadecimais",
            false,
        )),
        "[:space:]" | "\\s" => Some(Noun::new("espaço em branco", "espaços em branco", false)),
        _ => None,
    };
    match (known, c.negated) {
        (Some(n), false) => n,
        (Some(n), true) => Noun::new(
            &format!("caractere que não seja {}", n.singular),
            &format!("caracteres que não sejam {}", n.plural),
            false,
        ),
        (None, false) => {
            let items = class_items_text(c);
            Noun::new(
                &format!("caractere dentre {items}"),
                &format!("caracteres dentre {items}"),
                false,
            )
        }
        (None, true) => {
            let items = class_items_text(c);
            Noun::new(
                &format!("caractere diferente de {items}"),
                &format!("caracteres diferentes de {items}"),
                false,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------

const PROBES: &[&str] = &[
    "123",
    "abc123",
    "hello world",
    "2024-01-15",
    "user@example.com",
    "192.168.0.1",
    "ABC",
    "a1b2",
    "-",
    "ERROR: disk full",
];

fn samples(tokens: &[RegexToken], alts: &Alternatives, m: &Matcher) -> (Vec<String>, Vec<String>) {
    let mut yes: Vec<String> = Vec::new();
    for variant in 0..6 {
        if let Some(s) = gen_alts(tokens, alts, variant)
            && m.is_match(&s)
            && !yes.contains(&s)
        {
            yes.push(s);
        }
        if yes.len() >= MAX_SAMPLES {
            break;
        }
    }

    let base = yes.first().cloned().unwrap_or_default();
    let mut candidates: Vec<String> = vec!["abc".into()];
    if !base.is_empty() {
        candidates.push(format!("{base}abc"));
        candidates.push(format!("abc{base}"));
        let chars: Vec<char> = base.chars().collect();
        if chars.len() >= 2 {
            let mid = chars.len() - 1;
            let mut dotted: String = chars[..mid].iter().collect();
            dotted.push('.');
            dotted.extend(&chars[mid..]);
            candidates.push(dotted);
            candidates.push(chars[..chars.len() - 1].iter().collect());
        }
        candidates.push(base.to_uppercase());
        candidates.push(format!("{base} "));
    }
    candidates.push(String::new());
    candidates.extend(PROBES.iter().map(|s| s.to_string()));

    let mut no: Vec<String> = Vec::new();
    for c in candidates {
        if !m.is_match(&c) && !no.contains(&c) && !yes.contains(&c) {
            no.push(c);
        }
        if no.len() >= MAX_SAMPLES {
            break;
        }
    }
    yes.truncate(MAX_SAMPLES);
    (yes, no)
}

fn gen_alts(tokens: &[RegexToken], alts: &Alternatives, variant: usize) -> Option<String> {
    let seq = &alts[variant % alts.len()];
    let mut out = String::new();
    for item in seq {
        out.push_str(&gen_item(tokens, item, variant)?);
    }
    Some(out)
}

fn gen_item(tokens: &[RegexToken], item: &Item, variant: usize) -> Option<String> {
    let reps = repetitions(item.quant.as_ref(), variant);
    match &item.atom {
        Atom::Token(i) => {
            let t = &tokens[*i];
            match &t.detail {
                Detail::Literal(s) => Some(s.repeat(reps)),
                Detail::Backref(_) => None,
                _ => {
                    let pool = char_pool(t);
                    if pool.is_empty() {
                        return Some(String::new());
                    }
                    Some(pick(&pool, reps, variant))
                }
            }
        }
        Atom::Group(kind, alts) => match kind {
            GroupKind::Lookahead
            | GroupKind::NegativeLookahead
            | GroupKind::Lookbehind
            | GroupKind::NegativeLookbehind => Some(String::new()),
            _ => {
                let mut out = String::new();
                for r in 0..reps {
                    out.push_str(&gen_alts(tokens, alts, variant + r)?);
                }
                Some(out)
            }
        },
    }
}

/// How many times to repeat an item in each sample variant.
fn repetitions(q: Option<&Quant>, variant: usize) -> usize {
    let Some(q) = q else { return 1 };
    let max = q.max.unwrap_or(u32::MAX);
    let n = match variant {
        0 => q.min.max(1).saturating_add(2),
        1 => q.min,
        2 => q.min.saturating_add(5),
        3 => q.min.saturating_add(1),
        _ => q.min.max(1),
    };
    n.min(max).min(12) as usize
}

/// Characters that belong to a class-like token, in a pleasant order.
fn char_pool(t: &RegexToken) -> Vec<char> {
    const DIGITS: &str = "0123456789";
    const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
    const MIXED: &str = "abcxyzABCXYZ0123456789_-.@# ";
    match &t.detail {
        Detail::Shorthand(s) => match s {
            'd' => DIGITS.chars().collect(),
            'w' => LOWER.chars().collect(),
            's' => vec![' '],
            other => MIXED
                .chars()
                .filter(|c| parser::shorthand_contains(*other, *c))
                .collect(),
        },
        Detail::Class(c) => {
            let mut pool: Vec<char> = Vec::new();
            for item in &c.items {
                match item {
                    ClassItem::Range(a, b) if !c.negated => {
                        pool.extend((*a..=*b).take(26));
                    }
                    ClassItem::Char(ch) if !c.negated => pool.push(*ch),
                    _ => {}
                }
            }
            if pool.is_empty() || c.negated {
                pool = MIXED
                    .chars()
                    .chain(LOWER.chars())
                    .filter(|ch| c.contains(*ch))
                    .collect();
            }
            pool.dedup();
            pool
        }
        Detail::Unicode(_) => "aéçbZ".chars().collect(),
        Detail::None if t.kind == RegexTokenKind::AnyChar => "xa7b-Q".chars().collect(),
        _ => Vec::new(),
    }
}

/// Variant 2 walks the pool backwards (`987654`), others start at index 1.
fn pick(pool: &[char], count: usize, variant: usize) -> String {
    let n = pool.len();
    (0..count)
        .map(|k| match variant {
            2 => pool[(n - 1 + n * 16 - k) % n],
            3 => pool[k % n],
            _ => pool[(1 + k) % n],
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Portability notes
// ---------------------------------------------------------------------------

fn portability_notes(tokens: &[RegexToken]) -> Vec<String> {
    let mut notes = Vec::new();
    let has = |f: &dyn Fn(&RegexToken) -> bool| tokens.iter().any(f);
    if has(&|t| t.kind == RegexTokenKind::ShorthandClass) {
        notes.push(
            "grep -E não entende \\d, \\w, \\s em todas as versões: prefira [0-9], [[:alnum:]_], [[:space:]] ou use grep -P.".into(),
        );
    }
    if has(&|t| {
        matches!(
            t.kind,
            RegexTokenKind::Quantifier | RegexTokenKind::Alternation | RegexTokenKind::GroupOpen
        ) && t.text != "*"
    }) {
        notes.push(
            "No grep básico (BRE) +, ?, |, () e {} são literais: use grep -E (ou egrep) para tratá-los como operadores.".into(),
        );
    }
    if has(&|t| {
        matches!(
            &t.detail,
            Detail::Group(
                GroupKind::Lookahead
                    | GroupKind::NegativeLookahead
                    | GroupKind::Lookbehind
                    | GroupKind::NegativeLookbehind
                    | GroupKind::Atomic
            ) | Detail::Backref(_)
        )
    }) {
        notes.push(
            "Lookaround, grupos atômicos e retrorreferências exigem PCRE (grep -P); a crate regex do Rust não os suporta.".into(),
        );
    }
    notes
}

// ---------------------------------------------------------------------------
// Suggestions for partial patterns
// ---------------------------------------------------------------------------

fn suggest(partial: &str) -> Vec<RegexSuggestion> {
    let s = |snippet: &str, description: &str, result: String| RegexSuggestion {
        snippet: snippet.into(),
        description: description.into(),
        result,
    };
    let append =
        |snippet: &str, description: &str| s(snippet, description, format!("{partial}{snippet}"));

    if partial.ends_with('\\') && !partial.ends_with("\\\\") {
        let base = &partial[..partial.len() - 1];
        return [
            ("\\d", "dígito"),
            ("\\w", "caractere de palavra"),
            ("\\s", "espaço em branco"),
            ("\\b", "limite de palavra"),
            ("\\.", "ponto literal"),
            ("\\D", "não dígito"),
            ("\\S", "não espaço"),
        ]
        .iter()
        .map(|(snip, d)| s(snip, d, format!("{base}{snip}")))
        .collect();
    }

    let tokens = parser::parse(partial);
    // Ignore the zero-width "unclosed group" marker.
    let last = tokens.iter().rev().find(|t| !t.text.is_empty());
    let open_class =
        matches!(last, Some(t) if t.kind == RegexTokenKind::Invalid && t.text.starts_with('['));
    if open_class {
        let mut out = vec![append("]", "fecha a classe")];
        if partial.ends_with('[') {
            out.push(append(
                "^",
                "nega a classe: qualquer caractere exceto os listados",
            ));
        }
        out.extend([
            append("0-9]", "dígitos"),
            append("a-z]", "letras minúsculas"),
            append("A-Za-z]", "letras"),
            append("[:space:]]", "espaço em branco (POSIX)"),
        ]);
        return out;
    }
    if partial.ends_with('(') {
        return vec![
            append("?:", "grupo não capturante"),
            append("?P<nome>", "grupo nomeado"),
            append("?i)", "ignora maiúsculas/minúsculas (flag)"),
        ];
    }
    if partial.ends_with('{') {
        return vec![
            append("3}", "exatamente 3 vezes"),
            append("2,}", "2 ou mais vezes"),
            append("2,4}", "entre 2 e 4 vezes"),
        ];
    }

    let mut out = Vec::new();
    if last.is_some_and(|t| t.kind.quantifiable()) {
        out.extend([
            append("+", "um ou mais"),
            append("*", "zero ou mais"),
            append("?", "opcional"),
            append("{2,4}", "entre 2 e 4 vezes"),
        ]);
    }
    let open_groups = tokens
        .iter()
        .rev()
        .find(|t| t.kind == RegexTokenKind::Invalid && t.text.is_empty());
    if open_groups.is_some() {
        out.push(append(")", "fecha o grupo"));
    }
    if partial.starts_with('^') && !partial.ends_with('$') && partial.len() > 1 {
        out.push(append("$", "ancora no fim: a string inteira deve casar"));
    }
    if partial.is_empty() {
        out.extend([
            append("^", "âncora de início"),
            append("\\d+", "um ou mais dígitos"),
            append("[a-z]+", "uma ou mais letras minúsculas"),
            append("\\bpalavra\\b", "palavra inteira"),
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(p: &str) -> RegexAnalysis {
        StandardRegexAnalyzer.analyze(p)
    }

    #[test]
    fn spec_digits_example() {
        let a = analyze("^[0-9]+$");
        let labels: Vec<_> = a.tokens.iter().map(|t| t.label).collect();
        assert_eq!(
            labels,
            [
                "START_ANCHOR",
                "CHARACTER_CLASS",
                "QUANTIFIER",
                "END_ANCHOR"
            ]
        );
        assert_eq!(a.tokens[1].description, "Qualquer dígito entre 0 e 9.");
        assert_eq!(a.tokens[2].description, "Um ou mais.");
        assert_eq!(
            a.interpretation,
            "A expressão aceita uma string composta somente por dígitos, contendo pelo menos um dígito."
        );
        assert_eq!(a.matches[..3], ["123", "1", "987654"]);
        assert_eq!(a.non_matches, ["abc", "123abc", "abc123", "12.3"]);
        assert!(a.error.is_none());
    }

    #[test]
    fn samples_are_verified() {
        for p in [
            r"^\d{3}-\d{4}$",
            r"\bERROR\b",
            r"^(GET|POST) /api",
            r"[^aeiou]+",
            r"^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$",
            r"(ab)*c?",
            r"^$",
            ".",
        ] {
            let a = analyze(p);
            let m = Matcher::compile(p).unwrap();
            for s in &a.matches {
                assert!(m.is_match(s), "{p}: {s:?} deveria casar");
            }
            for s in &a.non_matches {
                assert!(!m.is_match(s), "{p}: {s:?} não deveria casar");
            }
            assert!(!a.matches.is_empty(), "{p}: nenhum exemplo gerado");
        }
    }

    #[test]
    fn interpretations() {
        assert_eq!(
            analyze(r"^\d{3}-\d{4}$").interpretation,
            "A expressão aceita somente strings formadas, do início ao fim, por: exatamente 3 dígitos, seguido de \"-\", seguido de exatamente 4 dígitos."
        );
        assert!(analyze("cat|dog").interpretation.contains("alternativas"));
        assert!(
            analyze(r"^ERROR")
                .interpretation
                .starts_with("A expressão procura textos que comecem com")
        );
        assert!(
            analyze("[a-z]+")
                .interpretation
                .contains("uma ou mais letras minúsculas")
        );
    }

    #[test]
    fn unsupported_patterns_report_error_but_still_explain() {
        let a = analyze(r"foo(?=bar)");
        assert!(a.error.is_some());
        assert!(a.matches.is_empty());
        assert!(a.tokens.iter().any(|t| t.description.contains("Lookahead")));
        assert!(a.notes.iter().any(|n| n.contains("grep -P")));
    }

    #[test]
    fn invalid_patterns_are_flagged() {
        let a = analyze("[abc");
        assert!(a.tokens[0].invalid);
        assert!(a.error.is_some());
    }

    #[test]
    fn explain_single_tokens() {
        let an = StandardRegexAnalyzer;
        assert_eq!(
            an.explain("^").unwrap(),
            "Início da string (ou de cada linha, com a flag m)."
        );
        assert_eq!(an.explain("+").unwrap(), "Um ou mais.");
        assert_eq!(an.explain("{2,4}").unwrap(), "Entre 2 e 4 vezes.");
        assert!(an.explain(r"\d").unwrap().contains("Dígito"));
        assert!(an.explain("(?:").is_some());
        assert!(an.explain("abc+").is_none());
    }

    #[test]
    fn suggestions_depend_on_context() {
        let an = StandardRegexAnalyzer;
        let snippets = |p: &str| {
            an.suggest(p)
                .into_iter()
                .map(|s| s.snippet)
                .collect::<Vec<_>>()
        };
        assert!(snippets("^\\").contains(&"\\d".to_string()));
        assert!(snippets("[").contains(&"0-9]".to_string()));
        assert!(snippets("^[0-9]").contains(&"+".to_string()));
        assert!(snippets("^[0-9]+").contains(&"$".to_string()));
        assert!(snippets("(abc").contains(&")".to_string()));
        let s = an.suggest("^\\");
        assert_eq!(s[0].result, "^\\d");
    }
}
