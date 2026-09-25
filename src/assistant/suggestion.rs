//! What the result list shows: a uniform [`Suggestion`] for entries, command
//! lines, options, analyses and categories.

use crate::document::{Document, Link};
use crate::knowledge::{Entry, EntryKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Command,
    Subcommand,
    Recipe,
    Concept,
    /// A concrete command line (example or recipe step).
    CommandLine,
    Option,
    /// A computed analysis (regex, CIDR, port, command explanation).
    Analysis,
    /// A regex completion snippet.
    Snippet,
    Category,
}

impl SuggestionKind {
    /// Short tag shown in the list.
    pub fn label(self) -> &'static str {
        match self {
            SuggestionKind::Command => "comando",
            SuggestionKind::Subcommand => "subcmd",
            SuggestionKind::Recipe => "receita",
            SuggestionKind::Concept => "conceito",
            SuggestionKind::CommandLine => "exemplo",
            SuggestionKind::Option => "opção",
            SuggestionKind::Analysis => "análise",
            SuggestionKind::Snippet => "regex",
            SuggestionKind::Category => "tema",
        }
    }

    pub fn of(entry: &Entry) -> Self {
        match entry.kind {
            EntryKind::Command if entry.parent.is_some() => SuggestionKind::Subcommand,
            EntryKind::Command => SuggestionKind::Command,
            EntryKind::Recipe => SuggestionKind::Recipe,
            EntryKind::Concept => SuggestionKind::Concept,
        }
    }
}

/// What opening a suggestion shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Link(Link),
    /// An option of a command, by its canonical flag.
    Option {
        entry: String,
        flag: String,
    },
    /// A document computed while answering (analyses).
    Document(Box<Document>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub title: String,
    pub subtitle: String,
    /// The whole input line after pressing Tab, if completion applies.
    pub completion: Option<String>,
    pub target: Target,
    /// Nesting level in the list (commands of an expanded recipe).
    pub indent: u8,
}

impl Suggestion {
    pub fn entry(entry: &Entry) -> Self {
        let completion = (entry.kind == EntryKind::Command).then(|| format!("{} ", entry.name));
        Self {
            kind: SuggestionKind::of(entry),
            title: entry.name.clone(),
            subtitle: entry.summary.clone(),
            completion,
            target: Target::Link(Link::Entry(entry.id.clone())),
            indent: 0,
        }
    }

    pub fn command_line(line: String, description: &str, completion: String) -> Self {
        Self {
            kind: SuggestionKind::CommandLine,
            subtitle: description.to_string(),
            completion: Some(completion),
            target: Target::Link(Link::Command {
                line: line.clone(),
                note: (!description.is_empty()).then(|| description.to_string()),
            }),
            title: line,
            indent: 0,
        }
    }

    pub fn analysis(title: impl Into<String>, subtitle: impl Into<String>, doc: Document) -> Self {
        Self {
            kind: SuggestionKind::Analysis,
            title: title.into(),
            subtitle: subtitle.into(),
            completion: None,
            target: Target::Document(Box::new(doc)),
            indent: 0,
        }
    }

    pub fn indented(mut self, indent: u8) -> Self {
        self.indent = indent;
        self
    }

    /// Identity used to drop duplicates from a result list.
    pub fn same_as(&self, other: &Suggestion) -> bool {
        match (&self.target, &other.target) {
            (Target::Link(Link::Entry(a)), Target::Link(Link::Entry(b))) => a == b,
            _ => self.kind == other.kind && self.title == other.title,
        }
    }
}
