//! Command-line explainer: `ss -ltnp | grep ':8080'` → what every part does,
//! how data flows through the pipeline, and what could go wrong.
//! Explanation only — nothing is executed.

use crate::document::{Block, Document, FlowNode, Item, Link, Row, Tone};
use crate::knowledge::{ArgKind, CommandOption, Entry, Repository};
use crate::networking::knowledge::port_info;
use crate::regex::RegexAnalyzer;
use crate::search::context::{self, LineAnalysis, Role};
use crate::search::tokenizer::{Token, TokenKind};

/// Builds the explanation document for `line`.
pub fn explain_line(
    repo: &Repository,
    regex: &dyn RegexAnalyzer,
    line: &str,
    note: Option<&str>,
) -> Document {
    let line = line.trim();
    let a = context::analyze(repo, line);
    let mut doc = Document::new(line).subtitle("explicação do comando");
    if let Some(note) = note.filter(|n| !n.is_empty()) {
        doc.paragraph(note);
    }

    doc.heading("PARTES");
    let mut rows = Vec::new();
    for (i, (token, role)) in a.tokens.iter().zip(&a.roles).enumerate() {
        rows.extend(explain_token(repo, regex, &a, i, token, role));
    }
    doc.table(rows);

    // Raw tokenizer output: the syntax the shell sees before any meaning.
    doc.heading("ANÁLISE LÉXICA");
    doc.paragraph(
        a.tokens
            .iter()
            .map(|t| format!("{} {}", t.kind.label(), t.text))
            .collect::<Vec<_>>()
            .join("  ·  "),
    );

    if let Some(flow) = flow(&a) {
        doc.heading("FLUXO DOS DADOS");
        doc.push(Block::Flow(flow));
    }

    let warnings = warnings(&a, line);
    if !warnings.is_empty() {
        doc.heading("ATENÇÃO");
        for (tone, text) in warnings {
            doc.note(tone, text);
        }
    }

    let commands = commands_in(&a);
    if !commands.is_empty() {
        doc.heading("COMANDOS USADOS");
        doc.list(
            commands
                .iter()
                .map(|e| {
                    Item::new(&e.name)
                        .detail(&e.summary)
                        .link(Link::Entry(e.id.clone()))
                })
                .collect(),
        );
    }
    doc.note(
        Tone::Muted,
        "Apenas explicação: o TermSense nunca executa comandos.",
    );
    doc
}

fn explain_token(
    repo: &Repository,
    regex: &dyn RegexAnalyzer,
    a: &LineAnalysis<'_>,
    index: usize,
    t: &Token,
    role: &Role<'_>,
) -> Vec<Row> {
    let text = t.text.clone();
    match role {
        Role::Command(e) => {
            let mut desc = e.summary.clone();
            if e.wrapper {
                desc.push_str(" (executa o comando que vem a seguir)");
            }
            vec![
                Row::new([text, "comando".into(), desc])
                    .tone(Tone::Accent)
                    .link(Link::Entry(e.id.clone())),
            ]
        }
        Role::UnknownCommand => vec![
            Row::new([
                text,
                "comando".into(),
                "Comando fora da base de conhecimento".into(),
            ])
            .tone(Tone::Warning),
        ],
        Role::Subcommand(e) => vec![
            Row::new([text, "subcomando".into(), e.summary.clone()])
                .indent(1)
                .tone(Tone::Accent)
                .link(Link::Entry(e.id.clone())),
        ],
        Role::Option { option, inline } => {
            let mut desc = option_text(&t.value, option);
            if let Some(v) = inline {
                desc.push_str(&format!(" · valor: {v}"));
            }
            vec![Row::new([text, "opção".into(), desc]).indent(1)]
        }
        Role::OptionCluster(flags) => {
            let expanded: Vec<String> = flags.iter().map(|(c, _)| format!("-{c}")).collect();
            let mut rows = vec![
                Row::new([
                    text,
                    "opções".into(),
                    format!("Flags curtas combinadas: {}", expanded.join(" ")),
                ])
                .indent(1),
            ];
            for (c, option) in flags {
                let flag = format!("-{c}");
                rows.push(match option {
                    Some(o) => {
                        Row::new([flag.clone(), "opção".into(), option_text(&flag, o)]).indent(2)
                    }
                    None => Row::new([flag, "opção".into(), "Não documentada na base".into()])
                        .indent(2)
                        .tone(Tone::Warning),
                });
            }
            rows
        }
        Role::UnknownOption => {
            let owner = owner(a, index).map_or("o comando".to_string(), |e| e.name.clone());
            vec![
                Row::new([
                    text,
                    "opção".into(),
                    format!("Opção não documentada para {owner}"),
                ])
                .indent(1)
                .tone(Tone::Warning),
            ]
        }
        Role::OptionValue(option) => {
            let arg = option.arg.as_deref().unwrap_or("VALOR");
            let mut desc = format!("Valor de {} ({arg})", option.key());
            if let Some(extra) = annotate_value(repo, t) {
                desc.push_str(" · ");
                desc.push_str(&extra);
            }
            vec![Row::new([text, "valor".into(), desc]).indent(2)]
        }
        Role::Argument(spec) => {
            let kind = spec.map(|s| s.kind);
            let label = match kind {
                Some(k) if k != ArgKind::Text => k.label().to_string(),
                _ => shape_label(t.kind).to_string(),
            };
            let mut parts: Vec<String> = Vec::new();
            match spec.filter(|s| !s.description.is_empty()) {
                Some(s) => {
                    parts.push(s.description.clone());
                    if matches!(t.kind, TokenKind::Url | TokenKind::Host) {
                        parts.push(shape_description(t));
                    }
                }
                None => parts.push(shape_description(t)),
            }
            if kind == Some(ArgKind::Regex) && !t.value.is_empty() {
                let analysis = regex.analyze(&t.value);
                parts.push(format!("regex: {}", analysis.interpretation));
            }
            if let Some(extra) = annotate_value(repo, t) {
                parts.push(extra);
            }
            vec![Row::new([text, label, parts.join(" · ")]).indent(1)]
        }
        Role::EndOfOptions => vec![
            Row::new([
                text,
                "fim das opções".into(),
                "Tudo depois é argumento, mesmo começando com -".into(),
            ])
            .indent(1),
        ],
        Role::Assignment => vec![Row::new([
            text,
            "variável".into(),
            "Define uma variável de ambiente só para este comando".into(),
        ])],
        Role::Pipe => {
            let (from, to) = neighbours(a, index);
            vec![
                Row::new([
                    text,
                    "pipe".into(),
                    format!("Envia a saída (stdout) de {from} para a entrada (stdin) de {to}"),
                ])
                .tone(Tone::Info),
            ]
        }
        Role::Operator => vec![
            Row::new([text.clone(), "operador".into(), operator_text(&text).into()])
                .tone(Tone::Info),
        ],
        Role::Redirect => vec![
            Row::new([
                text.clone(),
                "redirecionamento".into(),
                redirect_text(&text).into(),
            ])
            .tone(Tone::Info),
        ],
        Role::RedirectTarget => {
            let desc = if t.value == "/dev/null" {
                "Descarta o que for escrito (\"buraco negro\")"
            } else {
                "Arquivo de destino/origem do redirecionamento"
            };
            vec![Row::new([text, "arquivo".into(), desc.into()]).indent(1)]
        }
    }
}

/// Command owning the token at `index`.
fn owner<'r>(a: &LineAnalysis<'r>, index: usize) -> Option<&'r Entry> {
    a.segments
        .iter()
        .find(|s| s.tokens.contains(&index))
        .and_then(|s| s.entry())
}

/// `-i` → `--ignore-case · Ignora maiúsculas/minúsculas`.
fn option_text(typed: &str, o: &CommandOption) -> String {
    let other = if typed.starts_with("--") {
        o.short.as_deref()
    } else {
        o.long.as_deref()
    };
    match other {
        Some(alt) if alt != typed => format!("{alt} · {}", o.description),
        _ => o.description.clone(),
    }
}

fn neighbours(a: &LineAnalysis<'_>, pipe_index: usize) -> (String, String) {
    let name = |idx: Option<usize>| {
        idx.and_then(|i| a.segments.get(i))
            .and_then(|s| a.tokens.get(s.tokens.start))
            .map_or_else(|| "o próximo comando".to_string(), |t| t.text.clone())
    };
    let seg = a.segments.iter().position(|s| s.tokens.end == pipe_index);
    (name(seg), name(seg.map(|i| i + 1)))
}

fn operator_text(op: &str) -> &'static str {
    match op {
        "&&" => "Executa o próximo comando só se o anterior terminar com sucesso (exit code 0)",
        "||" => "Executa o próximo comando só se o anterior falhar (exit code ≠ 0)",
        ";" => "Executa os comandos em sequência, independentemente do resultado",
        "&" => "Executa o comando anterior em segundo plano (job)",
        _ => "Operador do shell",
    }
}

fn redirect_text(op: &str) -> &'static str {
    match op {
        ">" | "1>" => "Redireciona stdout para um arquivo (sobrescreve)",
        ">>" | "1>>" => "Redireciona stdout para um arquivo (acrescenta ao final)",
        "<" => "Lê stdin de um arquivo",
        "2>" => "Redireciona stderr (erros) para um arquivo",
        "2>>" => "Acrescenta stderr (erros) ao final de um arquivo",
        "2>&1" => "Envia stderr para o mesmo destino de stdout",
        "1>&2" | ">&2" => "Envia stdout para stderr",
        "&>" | "&>>" => "Redireciona stdout e stderr juntos",
        "<<" => "Here-document: stdin vem das linhas seguintes até o delimitador",
        _ => "Redirecionamento de entrada/saída",
    }
}

fn shape_label(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::String => "texto",
        TokenKind::Path => "caminho",
        TokenKind::Url => "URL",
        TokenKind::Host => "host",
        TokenKind::Number => "número",
        TokenKind::Variable => "variável",
        TokenKind::Substitution => "substituição",
        TokenKind::Glob => "glob",
        _ => "argumento",
    }
}

fn shape_description(t: &Token) -> String {
    match t.kind {
        TokenKind::String if t.text.starts_with('\'') => {
            "Texto entre aspas simples: nada dentro é expandido pelo shell".into()
        }
        TokenKind::String => {
            "Texto entre aspas duplas: $variáveis são expandidas, espaços não separam".into()
        }
        TokenKind::Path if t.value.starts_with('~') => {
            "Caminho relativo ao seu diretório home (~)".into()
        }
        TokenKind::Path => "Caminho de arquivo ou diretório".into(),
        TokenKind::Url => describe_url(&t.value),
        TokenKind::Host => describe_host(&t.value),
        TokenKind::Number => "Número".into(),
        TokenKind::Variable => "Variável expandida pelo shell antes da execução".into(),
        TokenKind::Substitution => {
            "Substituição: o shell executa o comando interno e usa a saída aqui".into()
        }
        TokenKind::Glob => {
            "Padrão de nomes expandido pelo shell (globbing) antes da execução".into()
        }
        _ => "Argumento".into(),
    }
}

fn describe_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "URL".into();
    };
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() => (h, Some(p)),
        _ => (authority, None),
    };
    let mut parts = vec![format!("esquema {scheme}"), format!("host {host}")];
    match port.and_then(|p| p.parse::<u16>().ok()) {
        Some(p) => parts.push(format!("porta {p}{}", port_suffix(p))),
        None => match scheme {
            "http" | "ws" => parts.push("porta 80 (padrão)".into()),
            "https" | "wss" => parts.push("porta 443 (padrão)".into()),
            _ => {}
        },
    }
    if !path.is_empty() {
        parts.push(format!("caminho {path}"));
    }
    format!("URL: {}", parts.join(", "))
}

fn describe_host(value: &str) -> String {
    if let Some((user, host)) = value.split_once('@') {
        return format!("Usuário {user} no host {host}");
    }
    if let Some((host, port)) = value.rsplit_once(':')
        && let Ok(p) = port.parse::<u16>()
    {
        return format!("Host {host}, porta {p}{}", port_suffix(p));
    }
    if value.contains('/') {
        return "Bloco de endereços em notação CIDR".into();
    }
    "Host (nome ou endereço IP)".into()
}

fn port_suffix(port: u16) -> String {
    port_info(port).map_or(String::new(), |p| format!(" ({})", p.service))
}

/// Extra knowledge about a value: HTTP methods, headers, well-known ports.
fn annotate_value(repo: &Repository, t: &Token) -> Option<String> {
    let value = t.value.trim();
    if let Ok(port) = value.trim_start_matches(':').parse::<u16>()
        && let Some(p) = port_info(port)
    {
        return Some(format!("porta {port} = {}", p.service));
    }
    let key = value.split(':').next().unwrap_or(value).trim();
    let concept = repo
        .concept(value)
        .or_else(|| (key.len() > 1).then(|| repo.concept(key)).flatten())?;
    Some(format!("{}: {}", concept.name, concept.summary))
}

/// Data flow of a pure pipeline (`a | b | c`).
fn flow(a: &LineAnalysis<'_>) -> Option<Vec<FlowNode>> {
    if a.segments.len() < 2 || a.roles.iter().any(|r| matches!(r, Role::Operator)) {
        return None;
    }
    let text = |range: std::ops::Range<usize>| {
        a.tokens[range]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut nodes: Vec<FlowNode> = a
        .segments
        .iter()
        .map(|s| FlowNode {
            label: text(s.tokens.clone()),
            edge: Some("stdout → pipe → stdin".into()),
        })
        .collect();
    let last = nodes.last_mut().expect("at least two segments");
    last.edge = Some("stdout".into());
    nodes.push(FlowNode {
        label: "terminal".into(),
        edge: None,
    });
    Some(nodes)
}

fn commands_in<'r>(a: &LineAnalysis<'r>) -> Vec<&'r Entry> {
    let mut out: Vec<&Entry> = Vec::new();
    for seg in &a.segments {
        for e in seg.wrappers.iter().chain(&seg.chain) {
            if !out.iter().any(|x| x.id == e.id) {
                out.push(e);
            }
        }
    }
    out
}

/// Knowledge warnings of the commands used plus risky patterns.
fn warnings(a: &LineAnalysis<'_>, line: &str) -> Vec<(Tone, String)> {
    let mut out: Vec<(Tone, String)> = Vec::new();
    for e in commands_in(a) {
        for w in &e.warnings {
            out.push((Tone::Warning, format!("{}: {w}", e.name)));
        }
    }
    let squashed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let has_flag = |cmd: &str, letter: char| {
        a.segments.iter().any(|s| {
            s.entry().is_some_and(|e| e.id == cmd)
                && a.tokens[s.tokens.clone()].iter().any(|t| {
                    t.kind == TokenKind::Option
                        && !t.text.starts_with("--")
                        && t.text.contains(letter)
                })
        })
    };
    if has_flag("rm", 'r') && has_flag("rm", 'f') {
        let catastrophic = a
            .tokens
            .iter()
            .any(|t| matches!(t.value.as_str(), "/" | "/*" | "~" | "~/" | "*" | "."));
        let text = if catastrophic {
            "rm -rf em /, ~, . ou * pode apagar o sistema ou todos os seus arquivos. Revise antes de executar."
        } else {
            "rm -rf apaga recursivamente e sem confirmação; não existe lixeira."
        };
        out.push((
            if catastrophic {
                Tone::Danger
            } else {
                Tone::Warning
            },
            text.into(),
        ));
    }
    let checks: &[(&str, Tone, &str)] = &[
        (
            "chmod -R 777",
            Tone::Danger,
            "Permissão total para qualquer usuário, recursivamente: grave risco de segurança.",
        ),
        (
            "chmod 777",
            Tone::Warning,
            "777 dá leitura, escrita e execução para qualquer usuário.",
        ),
        (
            "mkfs",
            Tone::Danger,
            "mkfs formata o dispositivo: todos os dados dele serão perdidos.",
        ),
        (
            "of=/dev/",
            Tone::Danger,
            "dd escrevendo em /dev/ sobrescreve o disco inteiro.",
        ),
        (
            "> /dev/sd",
            Tone::Danger,
            "Escrever direto em /dev/sd* corrompe o disco.",
        ),
        (
            ":(){",
            Tone::Danger,
            "Isto é uma fork bomb: esgota os processos do sistema.",
        ),
        (
            "| sh",
            Tone::Warning,
            "Executar código baixado direto no shell (curl | sh) sem inspecioná-lo é arriscado.",
        ),
        (
            "| bash",
            Tone::Warning,
            "Executar código baixado direto no shell (curl | bash) sem inspecioná-lo é arriscado.",
        ),
        (
            "git reset --hard",
            Tone::Warning,
            "reset --hard descarta alterações não commitadas sem volta.",
        ),
        (
            "git push --force",
            Tone::Warning,
            "--force reescreve o histórico remoto; prefira --force-with-lease.",
        ),
        (
            "git push -f",
            Tone::Warning,
            "-f reescreve o histórico remoto; prefira --force-with-lease.",
        ),
        (
            "git clean -f",
            Tone::Warning,
            "git clean -f apaga arquivos não rastreados definitivamente.",
        ),
        (
            "docker system prune",
            Tone::Warning,
            "Remove containers parados, redes e imagens sem uso.",
        ),
    ];
    for (needle, tone, text) in checks {
        if squashed.contains(needle) && !out.iter().any(|(_, t)| t == text) {
            out.push((*tone, (*text).to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regex::StandardRegexAnalyzer;

    fn explain(line: &str) -> Document {
        let repo = Repository::embedded().unwrap();
        explain_line(&repo, &StandardRegexAnalyzer, line, None)
    }

    fn rows(doc: &Document) -> Vec<Vec<String>> {
        doc.blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(rows) => Some(rows.iter().map(|r| r.cells.clone()).collect()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn philosophy_example() {
        let doc = explain("ss -ltnp | grep ':8080'");
        let rows = rows(&doc);
        let first: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(
            first,
            [
                "ss", "-ltnp", "-l", "-t", "-n", "-p", "|", "grep", "':8080'"
            ]
        );
        assert!(
            rows[2][2].to_lowercase().contains("escuta"),
            "{:?}",
            rows[2]
        );
        assert!(rows[3][2].contains("TCP"), "{:?}", rows[3]);
        assert!(
            rows[6][2].contains("(stdout) de ss para a entrada (stdin) de grep"),
            "{:?}",
            rows[6]
        );
        assert!(rows[8][2].contains("HTTP alternativo"), "{:?}", rows[8]);
        assert!(doc.blocks.iter().any(|b| matches!(b, Block::Flow(_))));
    }

    #[test]
    fn curl_json_post_is_fully_explained() {
        let doc = explain(
            r#"curl -X POST -H "Content-Type: application/json" -d '{"name":"William"}' http://localhost:8080/api/users"#,
        );
        let rows = rows(&doc);
        let labels: Vec<&str> = rows.iter().map(|r| r[1].as_str()).collect();
        assert_eq!(
            labels,
            [
                "comando", "opção", "valor", "opção", "valor", "opção", "valor", "URL"
            ]
        );
        assert!(rows[2][2].contains("POST"), "{:?}", rows[2]);
        assert!(rows[7][2].contains("porta 8080"), "{:?}", rows[7]);
        assert!(rows[7][2].contains("caminho /api/users"));
    }

    #[test]
    fn dangerous_commands_warn() {
        let doc = explain("sudo rm -rf /");
        assert!(doc.blocks.iter().any(|b| matches!(
            b,
            Block::Note {
                tone: Tone::Danger,
                ..
            }
        )));
    }

    #[test]
    fn regex_arguments_are_interpreted() {
        let doc = explain(r#"grep -E "^[0-9]+$" arquivo.txt"#);
        let rows = rows(&doc);
        let pattern = rows.iter().find(|r| r[0] == r#""^[0-9]+$""#).unwrap();
        assert!(pattern[2].contains("somente por dígitos"), "{pattern:?}");
    }

    #[test]
    fn redirects_and_operators() {
        let doc = explain("make 2>&1 | tee build.log && echo ok");
        let rows = rows(&doc);
        assert!(
            rows.iter()
                .any(|r| r[0] == "2>&1" && r[2].contains("stderr"))
        );
        assert!(
            rows.iter()
                .any(|r| r[0] == "&&" && r[2].contains("sucesso"))
        );
    }
}
