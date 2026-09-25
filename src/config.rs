//! Configuration.
//!
//! The MVP has no configuration file. It only resolves the configuration
//! directory (`$XDG_CONFIG_HOME/termsense` or `~/.config/termsense`) and
//! loads extra knowledge from its `knowledge/` subdirectory when present.
//! `config.toml` (theme, key bindings, ranking, layout) is reserved for a
//! future version; see ARCHITECTURE.md.

use std::env;
use std::path::PathBuf;

/// Maximum number of search hits considered per query.
pub const DEFAULT_MAX_RESULTS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Directories with extra `*.json` knowledge files, in load order.
    pub knowledge_dirs: Vec<PathBuf>,
    pub max_results: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            knowledge_dirs: Vec::new(),
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

impl Config {
    /// Defaults plus the user's knowledge directory, if it exists.
    pub fn load() -> Self {
        let mut config = Self::default();
        if let Some(dir) = config_dir().map(|d| d.join("knowledge"))
            && dir.is_dir()
        {
            config.knowledge_dirs.push(dir);
        }
        config
    }
}

/// `$XDG_CONFIG_HOME/termsense`, falling back to `$HOME/.config/termsense`.
pub fn config_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("termsense"))
}
