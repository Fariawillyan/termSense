//! Consistency rules for knowledge files, shared by the test suite and by
//! `ts --check` (which validates the user's own files).

use std::path::PathBuf;

use super::loader::{self, FileReport, LoadError};
use super::model::EntryKind;
use super::repository::Repository;

/// Problems in the loaded knowledge, one message per problem.
pub fn validate(repo: &Repository) -> Vec<String> {
    let mut out = Vec::new();
    for e in repo.entries() {
        let id = &e.id;
        for r in &e.related {
            if repo.get(r).is_none() {
                out.push(format!("{id}: related '{r}' não existe"));
            }
        }
        if e.related.contains(id) {
            out.push(format!("{id}: relacionado a si mesmo"));
        }
        if let Some(p) = &e.parent
            && repo.get(p).is_none()
        {
            out.push(format!("{id}: parent '{p}' não existe"));
        }
        if repo.category(&e.category).is_none() {
            out.push(format!("{id}: categoria '{}' não declarada", e.category));
        }
        if e.summary.ends_with('.') {
            out.push(format!("{id}: o resumo não deve terminar com ponto"));
        }
        for o in &e.options {
            if o.short.is_none() && o.long.is_none() {
                out.push(format!("{id}: opção sem flag ({})", o.description));
            }
            for flag in o.short.iter().chain(&o.long) {
                if !flag.starts_with('-') {
                    out.push(format!("{id}: flag '{flag}' sem hífen"));
                }
            }
            if o.kind.is_some() && o.arg.is_none() {
                out.push(format!("{id}: opção {} tem kind mas não tem arg", o.key()));
            }
        }
        if let Some(p) = &e.flag_prefix
            && !p.starts_with('-')
        {
            out.push(format!("{id}: flag_prefix '{p}' sem hífen"));
        }
        for n in &e.names {
            if n.is_empty() || n.contains(char::is_whitespace) {
                out.push(format!("{id}: nome alternativo inválido '{n}'"));
            }
        }
        if e.kind == EntryKind::Recipe && e.examples.is_empty() && e.steps.is_empty() {
            out.push(format!("{id}: receita sem examples nem steps"));
        }
    }
    out
}

/// Result of `ts --check`.
#[derive(Debug)]
pub struct Report {
    /// Built-in entries.
    pub embedded: usize,
    /// Each user file with its entry count or its parse error.
    pub files: Vec<FileReport>,
    pub problems: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.files.iter().all(|f| f.result.is_ok())
    }
}

/// Loads the built-in knowledge plus `files` and validates everything.
pub fn check(files: &[PathBuf]) -> Result<Report, LoadError> {
    let embedded = loader::load_embedded()?.entries.len();
    let (loaded, reports) = loader::load_files(files)?;
    let mut problems: Vec<String> = loaded
        .warnings
        .iter()
        .filter(|w| !reports.iter().any(|f| w.starts_with(f.source.as_str())))
        .cloned()
        .collect();
    problems.extend(validate(&Repository::new(loaded)));
    Ok(Report {
        embedded,
        files: reports,
        problems,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn embedded_knowledge_is_consistent() {
        let repo = Repository::embedded().unwrap();
        let problems = validate(&repo);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn check_reports_bad_user_files() {
        let dir = std::env::temp_dir().join(format!("termsense-check-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let good = dir.join("bom.json");
        let broken = dir.join("quebrado.json");
        let inconsistent = dir.join("inconsistente.json");
        fs::write(
            &good,
            r#"{"category":"linux","entries":[{"name":"meucmd","kind":"command","summary":"Faz algo"}]}"#,
        )
        .unwrap();
        fs::write(&broken, "{ nope").unwrap();
        fs::write(
            &inconsistent,
            r#"{"category":"linux","entries":[{"name":"outro","kind":"command","summary":"Termina com ponto.","related":["nao-existe"]}]}"#,
        )
        .unwrap();

        let report = check(&[good.clone(), broken.clone(), inconsistent]).unwrap();
        assert!(!report.ok());
        assert_eq!(report.files[0].result, Ok(1));
        assert!(report.files[1].result.is_err());
        assert!(report.problems.iter().any(|p| p.contains("nao-existe")));
        assert!(report.problems.iter().any(|p| p.contains("ponto")));

        let report = check(&[good]).unwrap();
        assert!(report.ok(), "{:?}", report.problems);

        // Replacing a built-in entry is allowed, and reported.
        let custom = dir.join("meu-grep.json");
        fs::write(
            &custom,
            r#"{"category":"text","entries":[{"name":"grep","kind":"command","summary":"Minha versão"}]}"#,
        )
        .unwrap();
        let report = check(&[custom]).unwrap();
        assert!(report.ok());
        assert_eq!(report.files[0].overrides, ["grep"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
