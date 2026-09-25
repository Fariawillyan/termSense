//! Knowledge model: the data types every knowledge file deserializes into.
//!
//! The model is deliberately generic — commands, concepts and task recipes all
//! share the same [`Entry`] shape, so new categories are pure data and need no
//! code changes.

use serde::Deserialize;

/// What an entry describes. Also used as a deterministic tie-breaker in
/// ranking: commands come before recipes, which come before concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// An executable command or subcommand (`grep`, `git status`).
    Command,
    /// A task-oriented answer ("quem usa a porta 8080") made of commands.
    Recipe,
    /// A concept to learn (`TCP`, `pipe`, `CIDR`).
    Concept,
}

impl EntryKind {
    pub fn label(self) -> &'static str {
        match self {
            EntryKind::Command => "comando",
            EntryKind::Recipe => "receita",
            EntryKind::Concept => "conceito",
        }
    }

    /// Tie-break order used by the ranking (lower first).
    pub fn rank(self) -> u8 {
        match self {
            EntryKind::Command => 0,
            EntryKind::Recipe => 1,
            EntryKind::Concept => 2,
        }
    }
}

/// A command-line option such as `-i, --ignore-case`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOption {
    #[serde(default)]
    pub short: Option<String>,
    #[serde(default)]
    pub long: Option<String>,
    /// Name of the value the option takes (`MÉTODO` for `-X`), if any.
    #[serde(default)]
    pub arg: Option<String>,
    /// Shape of the value, when the explainer can say more about it
    /// (`sed -e SCRIPT` is a sed program, `find -perm MODO` a mode).
    #[serde(default)]
    pub kind: Option<ArgKind>,
    /// The flag is a prefix glued to its value: `-Xmx` in `-Xmx512m`,
    /// `-XX:` in `-XX:+UseG1GC`.
    #[serde(default)]
    pub prefix: bool,
    pub description: String,
}

impl CommandOption {
    /// `-X, --request MÉTODO`
    pub fn display(&self) -> String {
        let mut out = match (&self.short, &self.long) {
            (Some(s), Some(l)) => format!("{s}, {l}"),
            (Some(s), None) => s.clone(),
            (None, Some(l)) => l.clone(),
            (None, None) => String::new(),
        };
        if let Some(arg) = &self.arg {
            out.push(' ');
            out.push_str(arg);
        }
        out
    }

    /// Canonical flag used to compare options across command lines.
    pub fn key(&self) -> &str {
        self.short
            .as_deref()
            .or(self.long.as_deref())
            .unwrap_or_default()
    }

    /// Matches the flag exactly (`-i`, `--ignore-case`, `-name`).
    pub fn matches(&self, flag: &str) -> bool {
        self.short.as_deref() == Some(flag) || self.long.as_deref() == Some(flag)
    }

    /// The letter of a POSIX short option (`-i` → `i`). Options written with a
    /// single dash but several letters (`find -name`) have no short char.
    pub fn short_char(&self) -> Option<char> {
        let s = self.short.as_deref()?;
        let mut chars = s.strip_prefix('-')?.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c != '-' => Some(c),
            _ => None,
        }
    }

    pub fn takes_value(&self) -> bool {
        self.arg.is_some()
    }
}

/// Shape of a positional argument; drives how the explainer annotates it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgKind {
    #[default]
    Text,
    Regex,
    Path,
    Host,
    Url,
    Port,
    Command,
    Number,
    User,
    /// Permission mode for chmod (`755`, `u+x`).
    Mode,
    /// File-creation mask (`022`).
    Umask,
    /// A sed script (`s/a/b/g`).
    Sed,
    /// An awk program (`{print $1}`).
    Awk,
    /// A Kubernetes/OpenShift resource: `pods`, `deployment/app`.
    Resource,
}

impl ArgKind {
    pub fn label(self) -> &'static str {
        match self {
            ArgKind::Text => "argumento",
            ArgKind::Regex => "padrão (regex)",
            ArgKind::Path => "caminho",
            ArgKind::Host => "host",
            ArgKind::Url => "URL",
            ArgKind::Port => "porta",
            ArgKind::Command => "comando",
            ArgKind::Number => "número",
            ArgKind::User => "usuário",
            ArgKind::Mode => "permissões",
            ArgKind::Umask => "máscara",
            ArgKind::Sed => "script sed",
            ArgKind::Awk => "programa awk",
            ArgKind::Resource => "recurso",
        }
    }
}

/// A positional argument (`PADRÃO`, `ARQUIVO...`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub name: String,
    #[serde(default)]
    pub kind: ArgKind,
    #[serde(default)]
    pub description: String,
    /// The last argument may absorb every remaining positional value.
    #[serde(default)]
    pub repeat: bool,
}

/// A concrete, copyable example. `command` may contain `{{var:default}}`
/// placeholders (see [`crate::knowledge::template`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub command: String,
    #[serde(default)]
    pub description: String,
}

/// One step of a guided procedure (troubleshooting sequences).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub command: Option<String>,
    /// Why this step matters — the learning part.
    #[serde(default)]
    pub why: String,
}

/// Free-form documentation block (characteristics, comparison tables...).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub title: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub items: Vec<String>,
    /// Aligned rows; the last cell is the description.
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
}

/// A unit of knowledge: command, concept or recipe.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Unique id. Derived from `name` when omitted (`git status` → `git-status`).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    /// Category id. Defaults to the file-level category.
    #[serde(default)]
    pub category: String,
    /// One-line description shown in lists.
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub usage: Option<String>,
    /// Id of the parent command for subcommands (`git-status` → `git`).
    #[serde(default)]
    pub parent: Option<String>,
    /// The command runs another command (`sudo`, `nohup`, `xargs`).
    #[serde(default)]
    pub wrapper: bool,
    /// Shell builtin or keyword (`cd`, `export`, `for`): part of the shell,
    /// not an executable in `PATH`.
    #[serde(default)]
    pub builtin: bool,
    /// Other executable names for the same command (`mvnw` for `mvn`).
    #[serde(default)]
    pub names: Vec<String>,
    /// Subcommands may follow one another: `mvn clean install`.
    #[serde(default)]
    pub phases: bool,
    /// Flags starting with this prefix identify the program even when the
    /// executable is not in the base: `--gtest_` in `./testes --gtest_filter=X`.
    #[serde(default)]
    pub flag_prefix: Option<String>,
    /// Synonyms and natural-language phrases, in any language.
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: Vec<CommandOption>,
    #[serde(default)]
    pub arguments: Vec<Argument>,
    #[serde(default)]
    pub examples: Vec<Example>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub sections: Vec<Section>,
    /// Ids of related entries (rendered as a relationship tree).
    #[serde(default)]
    pub related: Vec<String>,
    /// Installation hint when the command is not always present.
    #[serde(default)]
    pub install: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Entry {
    /// Finds an option by its exact flag.
    pub fn find_option(&self, flag: &str) -> Option<&CommandOption> {
        self.options.iter().find(|o| o.matches(flag))
    }

    /// Finds a POSIX short option by letter.
    pub fn find_short(&self, c: char) -> Option<&CommandOption> {
        self.options.iter().find(|o| o.short_char() == Some(c))
    }

    /// Last word of the name: `status` for `git status`.
    pub fn leaf_name(&self) -> &str {
        self.name.split_whitespace().last().unwrap_or(&self.name)
    }

    /// Executable that must exist for a command entry (`git` for `git status`).
    /// Builtins and keywords live inside the shell and have none.
    pub fn binary(&self) -> Option<&str> {
        match self.kind {
            EntryKind::Command if !self.builtin => self.name.split_whitespace().next(),
            _ => None,
        }
    }

    /// Positional argument spec for the n-th positional value.
    pub fn argument_at(&self, n: usize) -> Option<&Argument> {
        self.arguments
            .get(n)
            .or_else(|| self.arguments.last().filter(|a| a.repeat))
    }
}

/// Display metadata for a category id.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Top-level shape of a knowledge JSON file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeFile {
    /// Default category for entries that do not set one.
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(short: Option<&str>, long: Option<&str>, arg: Option<&str>) -> CommandOption {
        CommandOption {
            short: short.map(str::to_string),
            long: long.map(str::to_string),
            arg: arg.map(str::to_string),
            kind: None,
            prefix: false,
            description: String::new(),
        }
    }

    #[test]
    fn option_display_and_key() {
        let o = opt(Some("-X"), Some("--request"), Some("MÉTODO"));
        assert_eq!(o.display(), "-X, --request MÉTODO");
        assert_eq!(o.key(), "-X");
        assert!(o.matches("--request"));
        assert_eq!(o.short_char(), Some('X'));
        assert!(o.takes_value());
    }

    #[test]
    fn single_dash_long_options_have_no_short_char() {
        let o = opt(Some("-name"), None, Some("PADRÃO"));
        assert_eq!(o.short_char(), None);
        assert!(o.matches("-name"));
    }

    #[test]
    fn repeat_argument_absorbs_rest() {
        let json = r#"{"name":"grep","kind":"command","summary":"s",
            "arguments":[{"name":"PADRÃO","kind":"regex"},{"name":"ARQUIVO","kind":"path","repeat":true}]}"#;
        let e: Entry = serde_json::from_str(json).unwrap();
        assert_eq!(e.argument_at(0).unwrap().kind, ArgKind::Regex);
        assert_eq!(e.argument_at(5).unwrap().name, "ARQUIVO");
    }
}
