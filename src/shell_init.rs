//! Shell integration: `eval "$(ts --init bash)"` binds a key (Alt+H by
//! default) that opens `ts` with the line being typed. Leaving with Esc puts
//! the edited line back at the prompt; Ctrl+C keeps the original. The shell
//! runs the line only if the user presses Enter there: `ts` never executes.

/// Shells with an integration script.
pub const SHELLS: &[&str] = &["bash", "zsh", "fish"];

/// Integration script for `shell`, calling the binary at `exe`.
pub fn script(shell: &str, exe: &str) -> Option<String> {
    let exe = quote(exe);
    let text = match shell {
        "bash" => format!(
            r#"# TermSense: Alt+H abre o ts com a linha atual.
# Esc devolve a linha editada ao prompt; Ctrl+C mantém a original.
# Outra tecla: TERMSENSE_KEY='\C-g' antes do eval.
__termsense_widget() {{
    local line
    line=$({exe} --widget -- "$READLINE_LINE") || return
    READLINE_LINE=$line
    READLINE_POINT=${{#READLINE_LINE}}
}}
bind -x "\"${{TERMSENSE_KEY:-\\eh}}\": __termsense_widget"
"#
        ),
        "zsh" => format!(
            r#"# TermSense: Alt+H abre o ts com a linha atual.
# Esc devolve a linha editada ao prompt; Ctrl+C mantém a original.
# Outra tecla: TERMSENSE_KEY='^G' antes do eval.
__termsense_widget() {{
    local line
    line=$({exe} --widget -- "$BUFFER" </dev/tty) && {{
        BUFFER=$line
        CURSOR=${{#BUFFER}}
    }}
    zle reset-prompt
}}
zle -N __termsense_widget
bindkey "${{TERMSENSE_KEY:-\eh}}" __termsense_widget
"#
        ),
        "fish" => format!(
            r#"# TermSense: Alt+H abre o ts com a linha atual.
# Esc devolve a linha editada ao prompt; Ctrl+C mantém a original.
function __termsense_widget
    set -l line ({exe} --widget -- (commandline | string collect) | string collect)
    and commandline -r -- $line
    and commandline -C (string length -- $line)
    commandline -f repaint
end
bind \eh __termsense_widget
bind -M insert \eh __termsense_widget 2>/dev/null
"#
        ),
        _ => return None,
    };
    Some(text)
}

/// Single-quotes a path for any of the supported shells.
fn quote(s: &str) -> String {
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-+".contains(&b))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_bind_the_widget() {
        let bash = script("bash", "/home/u/.local/bin/ts").unwrap();
        assert!(bash.contains("bind -x"));
        assert!(bash.contains("/home/u/.local/bin/ts --widget -- \"$READLINE_LINE\""));
        let zsh = script("zsh", "ts").unwrap();
        assert!(zsh.contains("zle -N __termsense_widget"));
        assert!(zsh.contains("bindkey"));
        let fish = script("fish", "ts").unwrap();
        assert!(fish.contains("commandline -r"));
        assert!(script("tcsh", "ts").is_none());
    }

    #[test]
    fn paths_are_quoted() {
        assert_eq!(quote("/usr/bin/ts"), "/usr/bin/ts");
        assert_eq!(quote("/home/a b/ts"), "'/home/a b/ts'");
    }
}
