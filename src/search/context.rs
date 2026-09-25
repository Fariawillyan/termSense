//! Contextual understanding of a command line.
//!
//! [`analyze`] combines the syntactic tokens with the knowledge base to give
//! every token a semantic [`Role`]: command, subcommand, known option, option
//! value, positional argument (with its declared kind), pipe, redirect...
//! The result drives both the contextual autocomplete and the explainer.
//! Nothing is ever executed.

use std::ops::Range;

use super::tokenizer::{Token, TokenKind, tokenize};
use crate::knowledge::template::render_default;
use crate::knowledge::{Argument, CommandOption, Entry, Example, Repository};

/// Semantic role of a token.
#[derive(Debug, Clone)]
pub enum Role<'r> {
    Command(&'r Entry),
    UnknownCommand,
    Subcommand(&'r Entry),
    /// A known option; `inline` holds an attached value (`--port=22`, `-p22`).
    Option {
        option: &'r CommandOption,
        inline: Option<String>,
    },
    /// Combined short flags (`-rin`); unknown letters map to `None`.
    OptionCluster(Vec<(char, Option<&'r CommandOption>)>),
    UnknownOption,
    /// Value consumed by the preceding option (`POST` in `-X POST`).
    OptionValue(&'r CommandOption),
    Argument(Option<&'r Argument>),
    /// Shell grammar: `if`, `then`, `do`, `done`, `in`, `;;`, `]]`, `((…))`,
    /// test operators... `construct` is the knowledge entry of the construct
    /// the word belongs to (`for` for `do`), when the base has one.
    Keyword {
        construct: Option<&'r Entry>,
    },
    /// `NAME` in `for NAME in ...`.
    LoopVariable,
    /// A `case` pattern (`start)`, `*)`).
    CasePattern,
    /// The name in a function definition (`deploy()`, `function deploy`).
    FunctionName,
    EndOfOptions,
    Assignment,
    Pipe,
    Operator,
    Redirect,
    RedirectTarget,
}

/// One simple command inside a pipeline or list.
#[derive(Debug, Clone)]
pub struct Segment<'r> {
    /// Token indices belonging to this command.
    pub tokens: Range<usize>,
    /// Resolved command followed by its subcommands (`git`, `git stash`).
    pub chain: Vec<&'r Entry>,
    /// Commands that wrap the resolved one (`sudo`, `nohup`, `xargs`).
    pub wrappers: Vec<&'r Entry>,
    /// Canonical keys (`-r`, `--color`) of the options used.
    pub flags: Vec<String>,
    pub positionals: usize,
    /// Set when the first positional word should have been a subcommand of
    /// this entry but matched none (`git s` while typing `git status`).
    pub unknown_subcommand: Option<&'r Entry>,
    has_command: bool,
}

impl<'r> Segment<'r> {
    fn new(start: usize) -> Self {
        Self {
            tokens: start..start,
            chain: Vec::new(),
            wrappers: Vec::new(),
            flags: Vec::new(),
            positionals: 0,
            unknown_subcommand: None,
            has_command: false,
        }
    }

    /// Deepest resolved entry (`git stash` for `git stash pop`).
    pub fn entry(&self) -> Option<&'r Entry> {
        self.chain.last().copied()
    }

    fn find_option(&self, flag: &str) -> Option<&'r CommandOption> {
        self.chain.iter().rev().find_map(|e| e.find_option(flag))
    }

    fn find_short(&self, c: char) -> Option<&'r CommandOption> {
        self.chain.iter().rev().find_map(|e| e.find_short(c))
    }

    /// Longest prefix option starting `text`: `-Xmx` for `-Xmx512m`.
    fn find_prefix(&self, text: &str) -> Option<&'r CommandOption> {
        self.chain
            .iter()
            .rev()
            .flat_map(|e| e.options.iter())
            .filter(|o| o.prefix && text.len() > o.key().len() && text.starts_with(o.key()))
            .max_by_key(|o| o.key().len())
    }

    /// Nearest entry of the chain whose subcommands may repeat (`mvn`).
    fn phases_owner(&self) -> Option<&'r Entry> {
        self.chain.iter().rev().find(|e| e.phases).copied()
    }
}

/// What the user is typing at the end of the line.
#[derive(Debug, Clone)]
pub enum Cursor<'r> {
    /// Nothing yet, or the line ends with whitespace.
    Empty,
    Command,
    Subcommand {
        parent: &'r Entry,
        prefix: String,
    },
    Option(String),
    Argument,
}

/// A fully analyzed command line.
#[derive(Debug, Clone)]
pub struct LineAnalysis<'r> {
    pub tokens: Vec<Token>,
    pub roles: Vec<Role<'r>>,
    pub segments: Vec<Segment<'r>>,
    pub trailing_space: bool,
}

impl<'r> LineAnalysis<'r> {
    /// The command being typed (last segment).
    pub fn current(&self) -> &Segment<'r> {
        self.segments.last().expect("analysis always has a segment")
    }

    /// First command of the line, if it is known.
    pub fn first_command(&self) -> Option<&'r Entry> {
        self.segments.first().and_then(|s| s.chain.first().copied())
    }

    /// The line starts with shell grammar (`for`, `if`, `[[`...).
    pub fn starts_with_keyword(&self) -> bool {
        matches!(
            self.roles.first(),
            Some(Role::Keyword { .. } | Role::FunctionName)
        )
    }

    /// Shell constructs used in the line (`for`, `if`), without repetition.
    pub fn constructs(&self) -> Vec<&'r Entry> {
        let mut out: Vec<&'r Entry> = Vec::new();
        for role in &self.roles {
            if let Role::Keyword { construct: Some(e) } = role
                && !out.iter().any(|x| x.id == e.id)
            {
                out.push(e);
            }
        }
        out
    }

    /// Byte offset where a completion should be inserted.
    pub fn completion_start(&self, line: &str) -> usize {
        match self.tokens.last() {
            Some(t) if !self.trailing_space => t.start,
            _ => line.len(),
        }
    }

    /// Byte offset where the current segment starts.
    pub fn segment_start(&self, line: &str) -> usize {
        let seg = self.current();
        self.tokens
            .get(seg.tokens.start)
            .map_or(line.len(), |t| t.start)
    }

    pub fn cursor(&self) -> Cursor<'r> {
        let Some(last) = self.tokens.last() else {
            return Cursor::Empty;
        };
        if self.trailing_space {
            return Cursor::Empty;
        }
        let seg = self.current();
        // A lone "-" is a word to the tokenizer (stdin), but while typing
        // right after a known command it is the start of an option.
        let is_value = matches!(self.roles.last(), Some(Role::OptionValue(_)));
        if last.text.starts_with('-') && seg.entry().is_some() && !is_value {
            return Cursor::Option(last.text.clone());
        }
        match &self.roles[self.roles.len() - 1] {
            Role::Command(_) | Role::UnknownCommand => Cursor::Command,
            Role::Subcommand(e) if seg.chain.len() >= 2 => Cursor::Subcommand {
                // The real parent: with phases the chain is `mvn clean install`.
                parent: seg
                    .chain
                    .iter()
                    .find(|p| e.parent.as_deref() == Some(p.id.as_str()))
                    .copied()
                    .unwrap_or(seg.chain[seg.chain.len() - 2]),
                prefix: last.value.clone(),
            },
            Role::Option { .. } | Role::OptionCluster(_) | Role::UnknownOption => {
                Cursor::Option(last.text.clone())
            }
            Role::Argument(_) if seg.positionals == 1 => match seg.unknown_subcommand {
                Some(parent) => Cursor::Subcommand {
                    parent,
                    prefix: last.value.clone(),
                },
                None => Cursor::Argument,
            },
            _ => Cursor::Argument,
        }
    }

    /// Canonical text of the current segment (`grep -r "x"`).
    pub fn segment_text(&self) -> String {
        let seg = self.current();
        self.tokens[seg.tokens.clone()]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Knowledge ids of the shell constructs the grammar links to.
mod construct {
    pub const IF: &str = "if";
    pub const FOR: &str = "for";
    pub const WHILE: &str = "while";
    pub const CASE: &str = "case";
    pub const FUNCTION: &str = "shell-function";
    pub const GROUP: &str = "command-grouping";
    pub const ARITHMETIC: &str = "arithmetic";
    pub const NEGATION: &str = "exit-code";
    pub const DOUBLE_BRACKET: &str = "double-bracket";
    pub const BRACKET: &str = "test-bracket";
    pub const TEST: &str = "test";
}

/// What the grammar expects next, after `for`, `case` or `function`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Nothing,
    LoopVariable,
    LoopIn,
    CaseWord,
    CaseIn,
    FunctionName,
}

/// Where the next token stands after a grammar word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// Same simple command (test operators, closing `]]`).
    Stay,
    /// A new command starts (after `then`, `do`, `!`, `a)` in a case).
    Command,
    /// A header follows, not a command (`for NAME in LIST`, `case WORD in`).
    Header,
}

/// Shell grammar state carried across the tokens of a line.
struct Grammar<'r> {
    repo: &'r Repository,
    /// Open constructs, innermost last (`for` until its `done`).
    open: Vec<&'static str>,
    expect: Expect,
    /// Inside `case ... in`, at a pattern position.
    case_pattern: bool,
    /// Inside a test (`[[`, `[`, `test`), with its entry id.
    test: Option<&'static str>,
}

impl<'r> Grammar<'r> {
    fn entry(&self, id: &str) -> Option<&'r Entry> {
        self.repo.get(id)
    }

    fn keyword(&self, id: &str) -> Role<'r> {
        Role::Keyword {
            construct: self.entry(id),
        }
    }

    /// Construct of the innermost open loop (`for` or `while`).
    fn innermost(&self, candidates: &[&'static str]) -> &'static str {
        self.open
            .iter()
            .rev()
            .find(|c| candidates.contains(c))
            .copied()
            .unwrap_or(candidates[0])
    }

    fn close(&mut self, construct: &str) {
        if let Some(pos) = self.open.iter().rposition(|c| *c == construct) {
            self.open.truncate(pos);
        }
    }

    /// A reserved word in command position, with where the next token stands.
    fn reserved(&mut self, word: &str) -> Option<(Role<'r>, Next)> {
        use construct::*;
        let next = match word {
            "for" | "select" | "case" | "function" => Next::Header,
            _ => Next::Command,
        };
        let role = match word {
            "if" | "while" | "until" | "for" | "select" | "case" | "function" | "{" => {
                let id = match word {
                    "if" => IF,
                    "while" | "until" => WHILE,
                    "for" | "select" => FOR,
                    "case" => CASE,
                    "function" => FUNCTION,
                    _ => GROUP,
                };
                if word != "function" {
                    self.open.push(id);
                }
                self.expect = match word {
                    "for" | "select" => Expect::LoopVariable,
                    "case" => Expect::CaseWord,
                    "function" => Expect::FunctionName,
                    _ => Expect::Nothing,
                };
                self.keyword(id)
            }
            "then" | "elif" | "else" => self.keyword(IF),
            "fi" => {
                self.close(IF);
                self.keyword(IF)
            }
            "do" => self.keyword(self.innermost(&[FOR, WHILE])),
            "done" => {
                let id = self.innermost(&[FOR, WHILE]);
                self.close(id);
                self.keyword(id)
            }
            "esac" => {
                self.close(CASE);
                self.case_pattern = false;
                self.keyword(CASE)
            }
            "}" => {
                self.close(GROUP);
                self.keyword(GROUP)
            }
            "!" => self.keyword(NEGATION),
            w if w.starts_with("((") => self.keyword(ARITHMETIC),
            _ => return None,
        };
        Some((role, next))
    }
}

/// Ends the current simple command before token `i`; the next one starts
/// after it. Empty segments are dropped.
fn restart<'r>(seg: &mut Segment<'r>, segments: &mut Vec<Segment<'r>>, i: usize) {
    seg.tokens.end = i;
    let old = std::mem::replace(seg, Segment::new(i + 1));
    if !old.tokens.is_empty() {
        segments.push(old);
    }
}

/// Resolves every token of `line` against the knowledge base.
pub fn analyze<'r>(repo: &'r Repository, line: &str) -> LineAnalysis<'r> {
    let tokens = tokenize(line);
    let mut roles = Vec::with_capacity(tokens.len());
    let mut segments = Vec::new();
    let mut seg = Segment::new(0);
    let mut expect_value: Option<&'r CommandOption> = None;
    let mut after_redirect = false;
    let mut end_of_options = false;
    let mut subcommand_allowed = true;
    let mut g = Grammar {
        repo,
        open: Vec::new(),
        expect: Expect::Nothing,
        case_pattern: false,
        test: None,
    };

    for (i, t) in tokens.iter().enumerate() {
        // `&&` and `||` inside `[[ ]]` combine conditions; they do not end
        // the command. `|` inside a case pattern separates alternatives.
        let in_double_bracket = g.test == Some(construct::DOUBLE_BRACKET);
        if (in_double_bracket && matches!(t.text.as_str(), "&&" | "||"))
            || (g.case_pattern && t.kind == TokenKind::Pipe)
        {
            let id = if g.case_pattern {
                construct::CASE
            } else {
                construct::DOUBLE_BRACKET
            };
            roles.push(g.keyword(id));
            continue;
        }
        if t.kind.is_separator() {
            let case_end = t.text == ";;" && g.open.last() == Some(&construct::CASE);
            roles.push(if case_end {
                g.case_pattern = true;
                g.keyword(construct::CASE)
            } else if t.kind == TokenKind::Pipe {
                Role::Pipe
            } else {
                Role::Operator
            });
            g.test = None;
            seg.tokens.end = i;
            segments.push(std::mem::replace(&mut seg, Segment::new(i + 1)));
            expect_value = None;
            after_redirect = false;
            end_of_options = false;
            subcommand_allowed = true;
            continue;
        }
        if let Some((role, next)) = grammar_role(&mut g, &seg, t) {
            roles.push(role);
            if next != Next::Stay {
                restart(&mut seg, &mut segments, i);
                // Words of a `for`/`case` header are not a command.
                seg.has_command = next == Next::Header;
                expect_value = None;
                end_of_options = false;
                subcommand_allowed = true;
            }
            continue;
        }
        if t.kind == TokenKind::Redirect {
            roles.push(Role::Redirect);
            after_redirect = !t.text.contains('&') || t.text.ends_with('>');
            continue;
        }
        if after_redirect {
            after_redirect = false;
            roles.push(Role::RedirectTarget);
            continue;
        }
        if let Some(option) = expect_value.take() {
            roles.push(Role::OptionValue(option));
            continue;
        }
        if t.kind == TokenKind::Assignment && !seg.has_command {
            roles.push(Role::Assignment);
            continue;
        }
        if !seg.has_command {
            seg.has_command = true;
            roles.push(resolve_command(repo, &mut seg, t));
            g.test = seg.entry().and_then(|e| {
                [
                    construct::DOUBLE_BRACKET,
                    construct::BRACKET,
                    construct::TEST,
                ]
                .into_iter()
                .find(|id| *id == e.id)
            });
            continue;
        }
        if t.kind == TokenKind::EndOfOptions && !end_of_options {
            end_of_options = true;
            roles.push(Role::EndOfOptions);
            continue;
        }
        // Inside a test, `-f` after `&&` is an operator even though the
        // lexer sees a new command position there.
        let dash_in_test = g.test.is_some() && t.text.len() > 1 && t.text.starts_with('-');
        if (t.kind == TokenKind::Option || dash_in_test) && !end_of_options {
            let role = resolve_option(repo, &mut seg, t);
            if let Role::Option {
                option,
                inline: None,
            } = &role
                && option.takes_value()
            {
                expect_value = Some(option);
            }
            if let Role::OptionCluster(flags) = &role
                && let Some((_, Some(last))) = flags.last()
                && last.takes_value()
            {
                expect_value = Some(last);
            }
            roles.push(role);
            continue;
        }
        // Positional word: subcommand, wrapped command or argument.
        if let Some(entry) = seg.entry() {
            // `mvn clean install`: after a phase, another phase of the same
            // parent is still a subcommand.
            let child = repo.child(&entry.id, &t.value).or_else(|| {
                seg.phases_owner()
                    .and_then(|owner| repo.child(&owner.id, &t.value))
            });
            if subcommand_allowed
                && t.kind == TokenKind::Word
                && let Some(child) = child
            {
                seg.chain.push(child);
                roles.push(Role::Subcommand(child));
                continue;
            }
            if entry.wrapper && seg.positionals == 0 && seg.wrappers.len() < 4 {
                seg.wrappers.push(entry);
                seg.chain.clear();
                seg.flags.clear();
                roles.push(resolve_command(repo, &mut seg, t));
                subcommand_allowed = true;
                continue;
            }
        }
        if subcommand_allowed && seg.positionals == 0 && t.kind == TokenKind::Word {
            seg.unknown_subcommand = seg
                .phases_owner()
                .or_else(|| seg.entry().filter(|e| repo.has_children(&e.id)));
        }
        subcommand_allowed = false;
        let spec = seg.entry().and_then(|e| e.argument_at(seg.positionals));
        seg.positionals += 1;
        roles.push(Role::Argument(spec));
    }
    seg.tokens.end = tokens.len();
    segments.push(seg);

    LineAnalysis {
        trailing_space: line.ends_with(char::is_whitespace),
        tokens,
        roles,
        segments,
    }
}

/// Roles decided by the shell grammar rather than by the knowledge base:
/// reserved words, `for` and `case` headers, test operators, patterns.
fn grammar_role<'r>(g: &mut Grammar<'r>, seg: &Segment<'r>, t: &Token) -> Option<(Role<'r>, Next)> {
    let word = t.text.as_str();
    match g.expect {
        Expect::LoopVariable if word.starts_with("((") => {
            // C-style header: for ((i=0; i<10; i++))
            g.expect = Expect::Nothing;
            return Some((g.keyword(construct::ARITHMETIC), Next::Command));
        }
        Expect::LoopVariable => {
            g.expect = Expect::LoopIn;
            return Some((Role::LoopVariable, Next::Header));
        }
        Expect::LoopIn => {
            g.expect = Expect::Nothing;
            if word == "in" {
                return Some((g.keyword(construct::FOR), Next::Header));
            }
        }
        Expect::CaseWord => {
            g.expect = Expect::CaseIn;
            return Some((Role::Argument(None), Next::Header));
        }
        Expect::CaseIn => {
            g.expect = Expect::Nothing;
            if word == "in" {
                g.case_pattern = true;
                return Some((g.keyword(construct::CASE), Next::Header));
            }
        }
        Expect::FunctionName => {
            g.expect = Expect::Nothing;
            return Some((Role::FunctionName, Next::Command));
        }
        Expect::Nothing => {}
    }
    if g.case_pattern && word != "esac" {
        if word.ends_with(')') {
            g.case_pattern = false;
            return Some((Role::CasePattern, Next::Command));
        }
        return Some((Role::CasePattern, Next::Header));
    }
    if let Some(test) = g.test.filter(|_| seg.has_command) {
        let closing = match test {
            construct::DOUBLE_BRACKET => "]]",
            construct::BRACKET => "]",
            _ => "",
        };
        if word == closing {
            g.test = None;
            return Some((g.keyword(test), Next::Stay));
        }
        if matches!(word, "!" | "==" | "=" | "!=" | "=~" | "(" | ")") {
            return Some((g.keyword(test), Next::Stay));
        }
        return None;
    }
    if seg.has_command || t.kind == TokenKind::String {
        return None;
    }
    if word.len() > 2 && word.ends_with("()") {
        return Some((Role::FunctionName, Next::Command));
    }
    g.reserved(word)
}

fn resolve_command<'r>(repo: &'r Repository, seg: &mut Segment<'r>, t: &Token) -> Role<'r> {
    match repo.command(&t.value) {
        Some(entry) => {
            seg.chain = vec![entry];
            Role::Command(entry)
        }
        None => Role::UnknownCommand,
    }
}

fn resolve_option<'r>(repo: &'r Repository, seg: &mut Segment<'r>, t: &Token) -> Role<'r> {
    let text = t.value.as_str();
    if seg.chain.is_empty() {
        // Flags of a known family identify a program outside the base:
        // `./build/testes --gtest_filter=X` is a GoogleTest binary.
        match repo.flag_owner(text) {
            Some(owner) => seg.chain = vec![owner],
            None => return Role::UnknownOption,
        }
    }
    if text.starts_with("--") {
        let (name, inline) = match text.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (text, None),
        };
        return match seg.find_option(name) {
            Some(option) => {
                seg.flags.push(option.key().to_string());
                Role::Option { option, inline }
            }
            None => Role::UnknownOption,
        };
    }
    // Single dash: whole-token options first (`find -name`, `-mtime`).
    if let Some(option) = seg.find_option(text) {
        seg.flags.push(option.key().to_string());
        return Role::Option {
            option,
            inline: None,
        };
    }
    // gcc style: `-std=c++17`, `-fsanitize=address`. Before prefixes, so
    // `-fsanitize=` wins over the generic `-f`.
    if let Some((name, value)) = text.split_once('=')
        && let Some(option) = seg.find_option(name)
    {
        seg.flags.push(option.key().to_string());
        return Role::Option {
            option,
            inline: Some(value.to_string()),
        };
    }
    // A flag glued to its value: `-Xmx512m`, `-XX:+UseG1GC`, `-Wl,-rpath`.
    if let Some(option) = seg.find_prefix(text) {
        seg.flags.push(option.key().to_string());
        return Role::Option {
            option,
            inline: Some(text[option.key().len()..].to_string()),
        };
    }
    let letters: Vec<char> = text[1..].chars().collect();
    if letters.len() < 2 {
        return Role::UnknownOption;
    }
    // `-p2222`: the first letter takes a value, the rest is that value.
    if let Some(option) = seg.find_short(letters[0]).filter(|o| o.takes_value()) {
        seg.flags.push(option.key().to_string());
        return Role::Option {
            option,
            inline: Some(letters[1..].iter().collect()),
        };
    }
    let flags: Vec<(char, Option<&'r CommandOption>)> =
        letters.iter().map(|&c| (c, seg.find_short(c))).collect();
    if flags.iter().all(|(_, o)| o.is_none()) {
        return Role::UnknownOption;
    }
    for (_, o) in &flags {
        if let Some(o) = o {
            seg.flags.push(o.key().to_string());
        }
    }
    Role::OptionCluster(flags)
}

// ---------------------------------------------------------------------------
// Completion candidates
// ---------------------------------------------------------------------------

/// Options of the segment's command whose flag starts with `prefix`.
pub fn option_candidates<'r>(seg: &Segment<'r>, prefix: &str) -> Vec<&'r CommandOption> {
    let mut out: Vec<&'r CommandOption> = Vec::new();
    for entry in seg.chain.iter().rev() {
        for o in &entry.options {
            let hit = prefix == "-"
                || o.short.as_deref().is_some_and(|s| s.starts_with(prefix))
                || o.long.as_deref().is_some_and(|l| l.starts_with(prefix));
            if hit && !out.iter().any(|x| std::ptr::eq(*x, o)) {
                out.push(o);
            }
        }
    }
    out
}

/// Short flags (without a value) that can still be appended to a cluster
/// such as `-ri`.
pub fn cluster_extensions<'r>(seg: &Segment<'r>, cluster: &str) -> Vec<&'r CommandOption> {
    let used: Vec<char> = cluster.chars().skip(1).collect();
    let Some(entry) = seg.entry() else {
        return Vec::new();
    };
    entry
        .options
        .iter()
        .filter(|o| !o.takes_value())
        .filter(|o| o.short_char().is_some_and(|c| !used.contains(&c)))
        .collect()
}

/// Subcommands of `parent` starting with `prefix`.
pub fn subcommand_candidates<'r>(
    repo: &'r Repository,
    parent: &Entry,
    prefix: &str,
) -> Vec<&'r Entry> {
    repo.children(&parent.id)
        .filter(|e| e.leaf_name().starts_with(prefix))
        .collect()
}

/// How an example relates to what the user typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExampleMatch {
    /// The example continues exactly what was typed.
    StartsWith,
    /// The example uses every flag typed so far.
    HasFlags,
    /// Nothing specific typed yet: every example applies.
    Any,
}

/// Examples of the current command relevant to the typed line, best first.
pub fn example_candidates<'r>(
    repo: &'r Repository,
    analysis: &LineAnalysis<'r>,
) -> Vec<(&'r Example, ExampleMatch)> {
    let seg = analysis.current();
    let Some(entry) = seg.entry() else {
        return Vec::new();
    };
    let typed = analysis.segment_text();
    let typed_more = seg.tokens.len() > seg.chain.len();
    let mut out: Vec<(&'r Example, ExampleMatch)> = Vec::new();
    for example in &entry.examples {
        let rendered = render_default(&example.command);
        let ex = analyze(repo, &rendered);
        let Some(ex_seg) = ex
            .segments
            .iter()
            .find(|s| s.entry().is_some_and(|e| e.id == entry.id))
        else {
            continue;
        };
        let ex_text = ex.tokens[ex_seg.tokens.clone()]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let kind = if typed_more && ex_text.starts_with(&typed) {
            ExampleMatch::StartsWith
        } else if seg.flags.is_empty() {
            ExampleMatch::Any
        } else if seg.flags.iter().all(|f| ex_seg.flags.contains(f)) {
            ExampleMatch::HasFlags
        } else {
            continue;
        };
        out.push((example, kind));
    }
    out.sort_by_key(|(_, k)| *k);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn repo() -> &'static Repository {
        static R: OnceLock<Repository> = OnceLock::new();
        R.get_or_init(|| Repository::embedded().unwrap())
    }

    fn role_names(line: &str) -> Vec<String> {
        analyze(repo(), line)
            .roles
            .iter()
            .map(|r| match r {
                Role::Command(e) => format!("cmd:{}", e.id),
                Role::UnknownCommand => "cmd:?".into(),
                Role::Subcommand(e) => format!("sub:{}", e.id),
                Role::Option { option, inline } => {
                    format!(
                        "opt:{}{}",
                        option.key(),
                        inline
                            .as_deref()
                            .map(|v| format!("={v}"))
                            .unwrap_or_default()
                    )
                }
                Role::OptionCluster(f) => format!(
                    "cluster:{}",
                    f.iter()
                        .map(|(c, o)| if o.is_some() { *c } else { '?' })
                        .collect::<String>()
                ),
                Role::UnknownOption => "opt:?".into(),
                Role::OptionValue(o) => format!("val:{}", o.key()),
                Role::Argument(Some(a)) => format!("arg:{}", a.name),
                Role::Argument(None) => "arg".into(),
                Role::Keyword { construct } => {
                    format!("kw:{}", construct.map_or("?", |e| e.id.as_str()))
                }
                Role::LoopVariable => "loopvar".into(),
                Role::CasePattern => "pattern".into(),
                Role::FunctionName => "fn".into(),
                Role::EndOfOptions => "--".into(),
                Role::Assignment => "assign".into(),
                Role::Pipe => "|".into(),
                Role::Operator => "op".into(),
                Role::Redirect => ">".into(),
                Role::RedirectTarget => "target".into(),
            })
            .collect()
    }

    #[test]
    fn resolves_options_clusters_and_arguments() {
        assert_eq!(
            role_names(r#"grep -rin "ERROR" ./logs"#),
            ["cmd:grep", "cluster:rin", "arg:PADRÃO", "arg:ARQUIVO"]
        );
    }

    #[test]
    fn option_values_are_consumed() {
        let roles = role_names(
            r#"curl -X POST -H "Content-Type: application/json" -d '{"a":1}' http://localhost:8080/api"#,
        );
        assert_eq!(
            roles,
            [
                "cmd:curl", "opt:-X", "val:-X", "opt:-H", "val:-H", "opt:-d", "val:-d", "arg:URL"
            ]
        );
        assert_eq!(role_names("ssh -p2222 user@host")[1], "opt:-p=2222");
        assert_eq!(
            role_names("find . -mtime -1")[2..],
            ["opt:-mtime", "val:-mtime"]
        );
    }

    #[test]
    fn subcommands_and_pipelines() {
        assert_eq!(
            role_names("git stash pop"),
            ["cmd:git", "sub:git-stash", "arg"]
        );
        let a = analyze(repo(), "ss -ltnp | grep ':8080'");
        assert_eq!(a.segments.len(), 2);
        assert_eq!(a.segments[0].flags, ["-l", "-t", "-n", "-p"]);
        assert_eq!(a.current().entry().unwrap().id, "grep");
    }

    #[test]
    fn shell_grammar() {
        assert_eq!(
            role_names("for f in *.log; do gzip $f; done"),
            [
                "kw:for", "loopvar", "kw:for", "arg", "op", "kw:for", "cmd:gzip", "arg", "op",
                "kw:for"
            ]
        );
        assert_eq!(
            role_names("if [[ -f app.log && ! -s app.log ]]; then rm app.log; fi"),
            [
                "kw:if",
                "cmd:double-bracket",
                "opt:-f",
                "val:-f",
                "kw:double-bracket",
                "kw:double-bracket",
                "opt:-s",
                "val:-s",
                "kw:double-bracket",
                "op",
                "kw:if",
                "cmd:rm",
                "arg:ARQUIVO",
                "op",
                "kw:if"
            ]
        );
        assert_eq!(
            role_names("case $1 in start|stop) echo ok;; *) exit 1;; esac"),
            [
                "kw:case", "arg", "kw:case", "pattern", "kw:case", "pattern", "cmd:echo", "arg",
                "kw:case", "pattern", "cmd:exit", "arg", "kw:case", "kw:case"
            ]
        );
        assert_eq!(
            role_names("while true; do sleep 1; done"),
            [
                "kw:while",
                "cmd:?",
                "op",
                "kw:while",
                "cmd:sleep",
                "arg",
                "op",
                "kw:while"
            ]
        );
        assert_eq!(role_names("deploy() { git pull; }")[0], "fn");
        assert_eq!(role_names("echo done")[1], "arg", "done como argumento");
        let a = analyze(repo(), "for f in *; do grep -");
        assert_eq!(a.current().entry().unwrap().id, "grep");
        assert!(matches!(a.cursor(), Cursor::Option(p) if p == "-"));
        assert!(a.starts_with_keyword());
    }

    #[test]
    fn build_tools_and_clusters() {
        assert_eq!(
            role_names("mvn clean install -DskipTests"),
            [
                "cmd:mvn",
                "sub:mvn-clean",
                "sub:mvn-install",
                "opt:-D=skipTests"
            ]
        );
        assert_eq!(role_names("./mvnw test")[..2], ["cmd:mvn", "sub:mvn-test"]);
        assert_eq!(
            role_names("java -Xmx512m -XX:+UseG1GC -jar app.jar"),
            [
                "cmd:java",
                "opt:-Xmx=512m",
                "opt:-XX:=+UseG1GC",
                "opt:-jar",
                "val:-jar"
            ]
        );
        assert_eq!(
            role_names("g++ -std=c++17 -fsanitize=address -O2 -o app main.cpp -lpthread"),
            [
                "cmd:gpp",
                "opt:-std=c++17",
                "opt:-fsanitize=address",
                "opt:-O=2",
                "opt:-o",
                "val:-o",
                "arg:ARQUIVOS",
                "opt:-l=pthread"
            ]
        );
        assert_eq!(
            role_names("./build/testes --gtest_filter=A.* --gtest_repeat=3"),
            ["cmd:?", "opt:--gtest_filter=A.*", "opt:--gtest_repeat=3"]
        );
        assert_eq!(
            role_names("kubectl logs -f deploy/api"),
            ["cmd:oc", "sub:oc-logs", "opt:-f", "arg:RECURSO"]
        );
        assert_eq!(
            role_names("oc rollout restart deployment/api")[..3],
            ["cmd:oc", "sub:oc-rollout", "sub:oc-rollout-restart"]
        );
        let a = analyze(repo(), "mvn clean ins");
        assert!(
            matches!(a.cursor(), Cursor::Subcommand { parent, prefix } if parent.id == "mvn" && prefix == "ins")
        );
    }

    #[test]
    fn wrappers_expose_the_inner_command() {
        let a = analyze(repo(), "sudo ss -tulnp");
        assert_eq!(a.current().entry().unwrap().id, "ss");
        assert_eq!(a.current().wrappers[0].id, "sudo");
    }

    #[test]
    fn cursor_states() {
        let r = repo();
        assert!(matches!(analyze(r, "gr").cursor(), Cursor::Command));
        assert!(matches!(analyze(r, "grep -").cursor(), Cursor::Option(p) if p == "-"));
        assert!(
            matches!(analyze(r, "git s").cursor(), Cursor::Subcommand { parent, prefix } if parent.id == "git" && prefix == "s")
        );
        assert!(matches!(analyze(r, "grep ").cursor(), Cursor::Empty));
        assert!(matches!(
            analyze(r, "grep -r ERR").cursor(),
            Cursor::Argument
        ));
    }

    #[test]
    fn option_completion() {
        let a = analyze(repo(), "grep -");
        let opts = option_candidates(a.current(), "-");
        let keys: Vec<&str> = opts.iter().map(|o| o.key()).collect();
        for k in ["-i", "-r", "-n", "-v", "-E", "-o", "-c"] {
            assert!(keys.contains(&k), "{keys:?}");
        }
        let a = analyze(repo(), "grep --ig");
        let opts = option_candidates(a.current(), "--ig");
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].long.as_deref(), Some("--ignore-case"));
    }

    #[test]
    fn subcommand_completion() {
        let r = repo();
        let git = r.command("git").unwrap();
        let names: Vec<&str> = subcommand_candidates(r, git, "s")
            .iter()
            .map(|e| e.leaf_name())
            .collect();
        assert_eq!(names, ["show", "stash", "status", "switch"]);
    }

    #[test]
    fn examples_follow_typed_flags() {
        let r = repo();
        let a = analyze(r, "grep -r");
        let ex = example_candidates(r, &a);
        assert!(!ex.is_empty());
        for (e, _) in &ex {
            let x = analyze(r, &e.command);
            assert!(
                x.segments[0].flags.contains(&"-r".to_string()),
                "{}",
                e.command
            );
        }
        assert!(ex.iter().any(|(e, _)| e.command.starts_with("grep -rin")));
    }
}
