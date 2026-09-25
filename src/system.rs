//! Read-only facts about the local system.
//!
//! The only thing TermSense asks the system is whether an executable exists
//! in `PATH` (a `stat` per directory). Nothing is ever executed.

use std::env;
use std::path::{Path, PathBuf};

/// Finds `name` in `PATH`, like `command -v`, without running anything.
pub fn which(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sh_and_rejects_nonsense() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-command-termsense").is_none());
        assert!(which("../sh").is_none());
        assert!(which("").is_none());
    }
}
