//! Thin wrapper over the `regex` crate. TermSense never implements its own
//! matching engine; this is the only place that compiles patterns.

use regex::{Regex, RegexBuilder};

/// Upper bound for compiled program size: keeps pathological patterns cheap.
const SIZE_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct Matcher {
    regex: Regex,
}

impl Matcher {
    /// Compiles `pattern`, returning a one-line error message on failure.
    pub fn compile(pattern: &str) -> Result<Self, String> {
        RegexBuilder::new(pattern)
            .size_limit(SIZE_LIMIT)
            .dfa_size_limit(SIZE_LIMIT)
            .build()
            .map(|regex| Self { regex })
            .map_err(|e| compact_error(&e))
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

/// The crate prints a multi-line diagnostic; keep only the message.
fn compact_error(e: &regex::Error) -> String {
    match e {
        regex::Error::Syntax(s) => s
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix("error:").map(str::trim))
            .unwrap_or_else(|| s.lines().last().unwrap_or_default().trim())
            .to_string(),
        regex::Error::CompiledTooBig(_) => "padrão grande demais para compilar".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches() {
        let m = Matcher::compile("^[0-9]+$").unwrap();
        assert!(m.is_match("123"));
        assert!(!m.is_match("12.3"));
    }

    #[test]
    fn unsupported_features_report_a_short_error() {
        let err = Matcher::compile("(?=x)").unwrap_err();
        assert!(!err.contains('\n'));
        assert!(!err.is_empty());
        assert!(Matcher::compile("(a").is_err());
    }
}
