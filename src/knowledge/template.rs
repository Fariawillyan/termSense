//! Placeholders in knowledge commands: `{{name}}` or `{{name:default}}`.
//!
//! `ss -ltnp | grep ':{{port:8080}}'` renders as `... ':5432'` when the query
//! mentions port 5432, and falls back to `8080` otherwise. Only lowercase
//! identifiers are placeholders, so shell braces (`awk '{print $1}'`) and Go
//! templates (`docker inspect -f '{{.State}}'`) are left untouched.

use std::collections::BTreeMap;

/// Values extracted from the user's query (port, host, url...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vars(BTreeMap<&'static str, String>);

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &'static str, value: impl Into<String>) {
        self.0.insert(key, value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// Replaces placeholders with values from `vars`, their default, or their name.
pub fn render(template: &str, vars: &Vars) -> String {
    if !template.contains("{{") {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after
            .find("}}")
            .and_then(|end| parse(&after[..end]).map(|p| (end, p)))
        {
            Some((end, (name, default))) => {
                out.push_str(vars.get(name).or(default).unwrap_or(name));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Renders a template with no variables (defaults only).
pub fn render_default(template: &str) -> String {
    render(template, &Vars::default())
}

fn parse(inner: &str) -> Option<(&str, Option<&str>)> {
    let (name, default) = match inner.split_once(':') {
        Some((n, d)) => (n, Some(d)),
        None => (inner, None),
    };
    let valid = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    valid.then_some((name, default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_value_then_default_then_name() {
        let mut vars = Vars::new();
        vars.set("port", "5432");
        assert_eq!(render("grep ':{{port:8080}}'", &vars), "grep ':5432'");
        assert_eq!(
            render("ping {{host:example.com}}", &vars),
            "ping example.com"
        );
        assert_eq!(render("ssh {{host}}", &vars), "ssh host");
    }

    #[test]
    fn leaves_shell_and_go_templates_alone() {
        let vars = Vars::new();
        let awk = "awk '{print $1}' file";
        assert_eq!(render(awk, &vars), awk);
        let docker = "docker inspect -f '{{.State.Status}}' app";
        assert_eq!(render(docker, &vars), docker);
        let json = r#"curl -d '{"a":{"b":1}}' url"#;
        assert_eq!(render(json, &vars), json);
    }

    #[test]
    fn unterminated_placeholder_is_literal() {
        assert_eq!(render_default("echo {{port"), "echo {{port");
    }
}
