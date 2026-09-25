//! TermSense (`ts`): terminal knowledge assistant.
//!
//! Event loop: key event → update input → tokenize → search/analyze → rank →
//! update state → render. Blocking reads: no CPU is used while idle.

mod analysis;
mod app;
mod assistant;
mod config;
mod document;
mod input;
mod knowledge;
mod networking;
mod regex;
mod search;
mod shell_init;
mod system;
mod ui;

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crossterm::cursor::Show;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;
use assistant::Assistant;
use config::Config;
use input::{Action, map_key};
use knowledge::Repository;

const USAGE: &str = "\
TermSense — assistente de conhecimento para o terminal

USO:
    ts [CONSULTA...]          abre a interface interativa (opcionalmente já com a consulta)
    ts -p, --print CONSULTA   imprime sugestões e a explicação principal, sem interface
    ts --check [ARQUIVO...]   valida a base e os seus JSON (padrão: ~/.config/termsense/knowledge)
    ts --init bash|zsh|fish   imprime a integração com o shell (Alt+H abre o ts com a linha atual)
    ts -h, --help             mostra esta ajuda
    ts -V, --version          mostra a versão

EXEMPLOS:
    ts
    ts grep -r
    ts --print \"quem usa a porta 8080\"
    ts -p \"ss -ltnp | grep ':8080'\"
    ts -p regex '^[0-9]+$'
    ts -p '*/5 * * * *'
    ts -p exit 137
    eval \"$(ts --init bash)\"      # no ~/.bashrc

Conhecimento extra: arquivos *.json em ~/.config/termsense/knowledge/
O TermSense apenas consulta: nunca executa comandos.
";

enum Command {
    Interactive(String),
    Print(String),
    /// Opened by the shell integration: the edited line goes to stdout.
    Widget(String),
    Check(Vec<PathBuf>),
    Init(String),
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        Some("--check") => {
            return Ok(Command::Check(
                args[1..].iter().map(PathBuf::from).collect(),
            ));
        }
        Some("--init") => {
            return match args.get(1) {
                Some(shell) if shell_init::SHELLS.contains(&shell.as_str()) => {
                    Ok(Command::Init(shell.clone()))
                }
                _ => Err(format!(
                    "--init precisa de um shell: {}",
                    shell_init::SHELLS.join(", ")
                )),
            };
        }
        Some("--widget") => {
            // Everything after it is the line, verbatim (it may start with -).
            let rest = args[1..]
                .strip_prefix(&["--".to_string()])
                .unwrap_or(&args[1..]);
            return Ok(Command::Widget(rest.join(" ")));
        }
        _ => {}
    }
    let mut print = false;
    let mut words: Vec<&str> = Vec::new();
    let mut only_words = false;
    for arg in args {
        if only_words {
            words.push(arg);
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-p" | "--print" => print = true,
            "--" => only_words = true,
            flag if flag.starts_with("--") && words.is_empty() => {
                return Err(format!("opção desconhecida: {flag}"));
            }
            word => words.push(word),
        }
    }
    let query = words.join(" ");
    if print {
        if query.trim().is_empty() {
            return Err("--print precisa de uma consulta".into());
        }
        Ok(Command::Print(query))
    } else {
        Ok(Command::Interactive(query))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ts: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("ts (TermSense) {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Print(query) => match load() {
            // A closed pipe (`ts -p ... | head`) is a normal way to stop.
            Ok(assistant) => match print_answer(assistant, &query) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => fail(&e.to_string()),
            },
            Err(e) => fail(&e),
        },
        Command::Check(files) => check(files),
        Command::Init(shell) => {
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(str::to_string))
                .unwrap_or_else(|| "ts".to_string());
            match shell_init::script(&shell, &exe) {
                Some(script) => {
                    print!("{script}");
                    ExitCode::SUCCESS
                }
                None => fail(&format!("shell não suportado: {shell}")),
            }
        }
        Command::Widget(line) => {
            let assistant = match load() {
                Ok(a) => a,
                Err(e) => return fail(&e),
            };
            // stdout carries the answer back to the shell, so the interface
            // is drawn on the terminal device itself.
            let tty = match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
                Ok(tty) => tty,
                Err(e) => return fail(&format!("sem acesso ao terminal (/dev/tty): {e}")),
            };
            let mut app = App::new(assistant, &line).widget();
            match run(&mut app, tty) {
                Ok(()) if app.accepted => {
                    print!("{}", app.editor.text());
                    ExitCode::SUCCESS
                }
                Ok(()) => ExitCode::FAILURE,
                Err(e) => fail(&e.to_string()),
            }
        }
        Command::Interactive(query) => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                eprintln!(
                    "ts: a interface precisa de um terminal interativo; use `ts --print CONSULTA`."
                );
                return ExitCode::from(2);
            }
            let assistant = match load() {
                Ok(a) => a,
                Err(e) => return fail(&e),
            };
            match run(&mut App::new(assistant, &query), io::stdout()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e.to_string()),
            }
        }
    }
}

/// `ts --check`: validates the built-in knowledge plus the user's files.
fn check(files: Vec<PathBuf>) -> ExitCode {
    let files = if files.is_empty() {
        Config::load()
            .knowledge_dirs
            .iter()
            .flat_map(|d| knowledge::loader::json_files(d))
            .collect()
    } else {
        files
    };
    let report = match knowledge::validate::check(&files) {
        Ok(r) => r,
        Err(e) => return fail(&format!("erro na base embutida: {e}")),
    };
    println!("TermSense · verificação da base de conhecimento\n");
    println!("  ✓ base embutida: {} entradas", report.embedded);
    if files.is_empty() {
        let dir = config::config_dir().map_or_else(
            || "~/.config/termsense/knowledge".to_string(),
            |d| d.join("knowledge").display().to_string(),
        );
        println!("  · nenhum arquivo do usuário em {dir}");
    }
    for (source, result) in &report.files {
        match result {
            Ok(n) => println!("  ✓ {source}: {n} entradas"),
            Err(e) => println!("  ✗ {e}"),
        }
    }
    if !report.problems.is_empty() {
        println!("\nPROBLEMAS");
        for p in &report.problems {
            println!("  ✗ {p}");
        }
    }
    if report.ok() {
        println!("\nTudo certo.");
        ExitCode::SUCCESS
    } else {
        let invalid = report.files.iter().filter(|(_, r)| r.is_err()).count();
        println!(
            "\n{invalid} arquivo(s) inválido(s), {} problema(s).",
            report.problems.len()
        );
        ExitCode::FAILURE
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("ts: {message}");
    ExitCode::FAILURE
}

fn load() -> Result<Assistant, String> {
    let config = Config::load();
    let repo =
        Repository::load(&config).map_err(|e| format!("erro na base de conhecimento: {e}"))?;
    if repo.is_empty() {
        return Err("a base de conhecimento está vazia".into());
    }
    Ok(Assistant::new(repo, &config))
}

/// Runs the interface drawing on `out` (stdout, or `/dev/tty` in widget mode).
fn run<W: Write>(app: &mut App, out: W) -> io::Result<()> {
    set_panic_hook();
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;
    // Paste arrives as a single event instead of one key per character.
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableBracketedPaste
    )?;
    let result = event_loop(&mut terminal, app);
    let restored = disable_raw_mode().and_then(|()| {
        execute!(
            terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        )
    });
    result.and(restored)
}

/// Leaves the terminal usable if the program panics.
fn set_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let restore = (DisableBracketedPaste, LeaveAlternateScreen, Show);
        match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            Ok(mut tty) => {
                let _ = execute!(tty, restore.0, restore.1, restore.2);
            }
            Err(_) => {
                let _ = execute!(io::stdout(), restore.0, restore.1, restore.2);
            }
        }
        hook(info);
    }));
}

fn event_loop<W: Write>(
    terminal: &mut Terminal<CrosstermBackend<W>>,
    app: &mut App,
) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => app.handle(map_key(key)),
            Event::Paste(text) => app.handle(Action::Paste(text)),
            _ => {}
        }
    }
    Ok(())
}

/// Non-interactive answer: the suggestion list and the first document.
fn print_answer(mut assistant: Assistant, query: &str) -> io::Result<()> {
    let width = crossterm::terminal::size()
        .ok()
        .filter(|_| io::stdout().is_terminal())
        .map_or(100, |(w, _)| (w as usize).clamp(40, 120));
    let response = assistant.respond(query);
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "TermSense · modo {} · \"{}\"\n",
        response.mode.label(),
        query.trim()
    )?;
    if response.suggestions.is_empty() {
        return writeln!(out, "Nenhum resultado.");
    }
    for (i, s) in response.suggestions.iter().take(15).enumerate() {
        let indent = if s.indent > 0 { "  ↳ " } else { "" };
        let line = format!(
            "{:>3}. {:<8} {indent}{}  — {}",
            i + 1,
            s.kind.label(),
            s.title,
            s.subtitle
        );
        writeln!(out, "{}", ui::truncate(&line, width))?;
    }
    if response.suggestions.len() > 15 {
        writeln!(
            out,
            "     … mais {} sugestões",
            response.suggestions.len() - 15
        )?;
    }
    writeln!(out, "\n{}", "─".repeat(width.min(80)))?;
    let doc = assistant.preview(&response.suggestions[0]);
    for line in ui::document_text(&doc, width) {
        writeln!(out, "{line}")?;
    }
    // The process ends right after this: freeing the knowledge base and its
    // index allocation by allocation would only delay the exit.
    std::mem::forget(assistant);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn argument_parsing() {
        assert!(matches!(parse_args(&args(&[])), Ok(Command::Interactive(q)) if q.is_empty()));
        assert!(
            matches!(parse_args(&args(&["grep", "-r"])), Ok(Command::Interactive(q)) if q == "grep -r")
        );
        assert!(
            matches!(parse_args(&args(&["-p", "porta", "8080"])), Ok(Command::Print(q)) if q == "porta 8080")
        );
        assert!(matches!(parse_args(&args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(parse_args(&args(&["-V"])), Ok(Command::Version)));
        assert!(parse_args(&args(&["--nope"])).is_err());
        assert!(parse_args(&args(&["--print"])).is_err());
        assert!(
            matches!(parse_args(&args(&["--", "--help"])), Ok(Command::Interactive(q)) if q == "--help")
        );
        assert!(matches!(parse_args(&args(&["--check"])), Ok(Command::Check(f)) if f.is_empty()));
        assert!(
            matches!(parse_args(&args(&["--init", "zsh"])), Ok(Command::Init(s)) if s == "zsh")
        );
        assert!(parse_args(&args(&["--init", "tcsh"])).is_err());
        assert!(
            matches!(parse_args(&args(&["--widget", "--", "grep", "-r"])), Ok(Command::Widget(l)) if l == "grep -r")
        );
        assert!(
            matches!(parse_args(&args(&["--widget", "--"])), Ok(Command::Widget(l)) if l.is_empty())
        );
    }
}
