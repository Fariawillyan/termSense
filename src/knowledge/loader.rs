//! Loads knowledge files.
//!
//! The base knowledge is embedded in the binary (`include_str!`), so an
//! installed `ts` needs no data directory. Extra JSON files from the user's
//! knowledge directories are merged on top: an entry with an existing id
//! replaces the built-in one, new ids are added.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{Category, Entry, KnowledgeFile};
use crate::search::tokenizer::normalize;

/// Knowledge files compiled into the binary, in load order.
pub const EMBEDDED: &[(&str, &str)] = &[
    (
        "categories.json",
        include_str!("../../knowledge/categories.json"),
    ),
    ("linux.json", include_str!("../../knowledge/linux.json")),
    ("text.json", include_str!("../../knowledge/text.json")),
    ("shell.json", include_str!("../../knowledge/shell.json")),
    (
        "processes.json",
        include_str!("../../knowledge/processes.json"),
    ),
    ("regex.json", include_str!("../../knowledge/regex.json")),
    (
        "networking.json",
        include_str!("../../knowledge/networking.json"),
    ),
    (
        "network-concepts.json",
        include_str!("../../knowledge/network-concepts.json"),
    ),
    ("http.json", include_str!("../../knowledge/http.json")),
    ("ssh.json", include_str!("../../knowledge/ssh.json")),
    ("git.json", include_str!("../../knowledge/git.json")),
    ("docker.json", include_str!("../../knowledge/docker.json")),
];

/// A knowledge file that could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub source: String,
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}

impl std::error::Error for LoadError {}

/// Result of loading every source.
#[derive(Debug, Default)]
pub struct Loaded {
    pub entries: Vec<Entry>,
    pub categories: Vec<Category>,
    /// Non-fatal problems (invalid user files, duplicated ids).
    pub warnings: Vec<String>,
}

/// Parses one knowledge file and fills derived fields (id, category).
pub fn parse(source: &str, content: &str) -> Result<KnowledgeFile, LoadError> {
    let mut file: KnowledgeFile = serde_json::from_str(content).map_err(|e| LoadError {
        source: source.to_string(),
        message: e.to_string(),
    })?;
    let default_category = file.category.clone().unwrap_or_else(|| "geral".to_string());
    for entry in &mut file.entries {
        entry.name = entry.name.trim().to_string();
        if entry.id.trim().is_empty() {
            entry.id = slug(&entry.name);
        }
        if entry.category.is_empty() {
            entry.category = default_category.clone();
        }
        if entry.name.is_empty() || entry.summary.trim().is_empty() {
            return Err(LoadError {
                source: source.to_string(),
                message: format!("entrada '{}' sem nome ou resumo", entry.id),
            });
        }
    }
    Ok(file)
}

/// Loads only the built-in knowledge. Built-in files must always parse.
pub fn load_embedded() -> Result<Loaded, LoadError> {
    let mut loaded = Loaded::default();
    for (name, content) in EMBEDDED {
        let file = parse(&format!("knowledge/{name}"), content)?;
        merge(&mut loaded, file, name, true);
    }
    Ok(loaded)
}

/// Loads the built-in knowledge plus every `*.json` file found in `dirs`.
/// Problems in user files become warnings instead of errors.
pub fn load(dirs: &[PathBuf]) -> Result<Loaded, LoadError> {
    let mut loaded = load_embedded()?;
    for dir in dirs {
        for path in json_files(dir) {
            let source = path.display().to_string();
            match fs::read_to_string(&path) {
                Ok(content) => match parse(&source, &content) {
                    Ok(file) => merge(&mut loaded, file, &source, false),
                    Err(e) => loaded.warnings.push(e.to_string()),
                },
                Err(e) => loaded.warnings.push(format!("{source}: {e}")),
            }
        }
    }
    Ok(loaded)
}

/// `git status` → `git-status`, `Conexão TCP` → `conexao-tcp`.
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in normalize(name).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Sorted for deterministic override order.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

fn merge(loaded: &mut Loaded, file: KnowledgeFile, source: &str, warn_duplicates: bool) {
    for category in file.categories {
        match loaded.categories.iter_mut().find(|c| c.id == category.id) {
            Some(existing) => *existing = category,
            None => loaded.categories.push(category),
        }
    }
    for entry in file.entries {
        match loaded.entries.iter().position(|e| e.id == entry.id) {
            Some(i) => {
                if warn_duplicates {
                    loaded
                        .warnings
                        .push(format!("{source}: id duplicado '{}'", entry.id));
                }
                loaded.entries[i] = entry;
            }
            None => loaded.entries.push(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_folds_accents_and_spaces() {
        assert_eq!(slug("git status"), "git-status");
        assert_eq!(slug("Conexão TCP"), "conexao-tcp");
        assert_eq!(slug("404 Not Found"), "404-not-found");
        assert_eq!(slug("ssh-keygen"), "ssh-keygen");
    }

    #[test]
    fn parse_fills_id_and_category() {
        let file = parse(
            "t.json",
            r#"{"category":"text","entries":[{"name":"grep","kind":"command","summary":"Busca"}]}"#,
        )
        .unwrap();
        assert_eq!(file.entries[0].id, "grep");
        assert_eq!(file.entries[0].category, "text");
    }

    #[test]
    fn parse_rejects_unknown_fields_and_empty_summary() {
        let unknown =
            r#"{"entries":[{"name":"x","kind":"command","summary":"s","sumary":"typo"}]}"#;
        assert!(parse("t.json", unknown).is_err());
        let empty = r#"{"entries":[{"name":"x","kind":"command","summary":"  "}]}"#;
        assert!(parse("t.json", empty).is_err());
        assert!(parse("t.json", "{ not json").is_err());
    }

    #[test]
    fn embedded_knowledge_loads_without_warnings() {
        let loaded = load_embedded().expect("embedded knowledge must parse");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert!(
            loaded.entries.len() > 150,
            "only {} entries",
            loaded.entries.len()
        );
    }

    #[test]
    fn user_files_override_and_extend() {
        let dir = std::env::temp_dir().join(format!("termsense-loader-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("a.json"),
            r#"{"category":"custom","entries":[
                {"name":"grep","kind":"command","summary":"Minha versão"},
                {"name":"meucmd","kind":"command","summary":"Novo"}]}"#,
        )
        .unwrap();
        fs::write(dir.join("b.json"), "{ quebrado").unwrap();

        let loaded = load(std::slice::from_ref(&dir)).unwrap();
        let grep = loaded.entries.iter().find(|e| e.id == "grep").unwrap();
        assert_eq!(grep.summary, "Minha versão");
        assert!(loaded.entries.iter().any(|e| e.id == "meucmd"));
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(loaded.warnings[0].contains("b.json"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
