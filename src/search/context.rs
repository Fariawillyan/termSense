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
            Role::Subcommand(_) if seg.chain.len() >= 2 => Cursor::Subcommand {
                parent: seg.chain[seg.chain.len() - 2],
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

    for (i, t) in tokens.iter().enumerate() {
        if t.kind.is_separator() {
            roles.push(if t.kind == TokenKind::Pipe {
                Role::Pipe
            } else {
                Role::Operator
            });
            seg.tokens.end = i;
            segments.push(std::mem::replace(&mut seg, Segment::new(i + 1)));
            expect_value = None;
            after_redirect = false;
            end_of_options = false;
            subcommand_allowed = true;
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
            continue;
        }
        if t.kind == TokenKind::EndOfOptions && !end_of_options {
            end_of_options = true;
            roles.push(Role::EndOfOptions);
            continue;
        }
        if t.kind == TokenKind::Option && !end_of_options {
            let role = resolve_option(&mut seg, t);
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
            if subcommand_allowed
                && t.kind == TokenKind::Word
                && let Some(child) = repo.child(&entry.id, &t.value)
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
            seg.unknown_subcommand = seg.entry().filter(|e| repo.has_children(&e.id));
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

fn resolve_command<'r>(repo: &'r Repository, seg: &mut Segment<'r>, t: &Token) -> Role<'r> {
    match repo.command(&t.value) {
        Some(entry) => {
            seg.chain = vec![entry];
            Role::Command(entry)
        }
        None => Role::UnknownCommand,
    }
}

fn resolve_option<'r>(seg: &mut Segment<'r>, t: &Token) -> Role<'r> {
    let text = t.value.as_str();
    if seg.chain.is_empty() {
        return Role::UnknownOption;
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
        assert_eq!(names, ["stash", "status", "switch"]);
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
