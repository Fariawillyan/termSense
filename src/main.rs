//! TermSense (`ts`): terminal knowledge assistant.
//!
//! Event loop: key event → update input → tokenize → search/analyze → rank →
//! update state → render. Blocking reads: no CPU is used while idle.

mod app;
mod assistant;
mod config;
mod document;
mod input;
mod knowledge;
mod networking;
mod regex;
mod search;
mod system;
mod ui;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;

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
    ts -h, --help             mostra esta ajuda
    ts -V, --version          mostra a versão

EXEMPLOS:
    ts
    ts grep -r
    ts --print \"quem usa a porta 8080\"
    ts -p \"ss -ltnp | grep ':8080'\"
    ts -p regex '^[0-9]+$'

Conhecimento extra: arquivos *.json em ~/.config/termsense/knowledge/
O TermSense apenas consulta: nunca executa comandos.
";

enum Command {
    Interactive(String),
    Print(String),
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
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
            match run(App::new(assistant, &query)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e.to_string()),
            }
        }
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

fn run(mut app: App) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    // Paste arrives as a single event instead of one key per character.
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = event_loop(&mut terminal, &mut app);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
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
    }
}
