//! Regex as a first-class feature: parsing into an intermediate
//! representation, explanation, interpretation and verified samples.

pub mod analyzer;
pub mod matcher;
pub mod parser;

pub use analyzer::{RegexAnalysis, RegexSuggestion, StandardRegexAnalyzer};
pub use parser::{RegexToken, RegexTokenKind};

/// Regex analysis abstraction. The UI and the assistant depend on this trait,
/// not on a concrete implementation, so another engine (PCRE semantics, a
/// future AI-backed explainer) can be plugged in.
pub trait RegexAnalyzer {
    /// Pattern → intermediate representation (`START_ANCHOR`, ...).
    fn parse(&self, pattern: &str) -> Vec<RegexToken>;
    /// Full analysis: tokens, interpretation, verified samples.
    fn analyze(&self, pattern: &str) -> RegexAnalysis;
    /// Completions for a partially typed pattern.
    fn suggest(&self, partial: &str) -> Vec<RegexSuggestion>;
    /// Explanation of a single token (`\d`, `+`, `[^0-9]`).
    fn explain(&self, token: &str) -> Option<String>;
}
