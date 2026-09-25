//! Command-line explainer: `ss -ltnp | grep ':8080'` → what every part does,
//! how data flows through the pipeline, and what could go wrong.
//! Explanation only — nothing is executed.

use crate::analysis::{awk, permissions, sed};
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

    let mut commands = commands_in(&a);
    for e in a.constructs() {
        if !commands.iter().any(|x| x.id == e.id) {
            commands.push(e);
        }
    }
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
        Role::UnknownCommand => {
            let desc = format!(
                "Comando fora da base de conhecimento: consulte man {0} ou {0} --help",
                t.value
            );
            vec![Row::new([text, "comando".into(), desc]).tone(Tone::Warning)]
        }
        Role::Keyword { construct } => {
            let id = construct.map_or("", |e| e.id.as_str());
            let (label, desc) = keyword_text(&t.text, id, a, index);
            let mut row = Row::new([text, label.into(), desc]).tone(Tone::Info);
            if let Some(e) = construct {
                row = row.link(Link::Entry(e.id.clone()));
            }
            vec![row]
        }
        Role::LoopVariable => vec![
            Row::new([
                text.clone(),
                "variável".into(),
                format!("Variável do laço: recebe um item da lista por vez (use ${text} no corpo)"),
            ])
            .indent(1),
        ],
        Role::CasePattern => vec![
            Row::new([
                text,
                "padrão".into(),
                "Padrão do case (glob): o bloco a seguir roda se o valor casar; * casa com qualquer coisa".into(),
            ])
            .indent(1),
        ],
        Role::FunctionName => vec![Row::new([
            text.trim_end_matches("()").to_string(),
            "função".into(),
            "Nome da função definida aqui; depois ela é chamada como um comando".into(),
        ])],
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
            if let Some(extra) = option
                .kind
                .and_then(|k| describe_kind(k, &t.value))
                .or_else(|| annotate_value(repo, t))
            {
                desc.push_str(" · ");
                desc.push_str(&extra);
            }
            let mut rows = vec![Row::new([text, "valor".into(), desc]).indent(2)];
            if let Some(k) = option.kind {
                rows.extend(program_rows(k, &t.value, regex, 3));
            }
            rows
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
            let after_match_operator = index > 0 && a.tokens[index - 1].text == "=~";
            if (kind == Some(ArgKind::Regex) || after_match_operator) && !t.value.is_empty() {
                let analysis = regex.analyze(&t.value);
                parts.push(format!("regex: {}", analysis.interpretation));
            }
            if let Some(extra) = kind
                .and_then(|k| describe_kind(k, &t.value))
                .or_else(|| annotate_value(repo, t))
            {
                parts.push(extra);
            }
            let mut rows = vec![Row::new([text, label, parts.join(" · ")]).indent(1)];
            if let Some(k) = kind {
                rows.extend(program_rows(k, &t.value, regex, 2));
            }
            rows
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
            let previous = index.checked_sub(1).map(|i| a.tokens[i].text.as_str());
            let (label, desc) = match previous {
                Some("<<" | "<<-") => (
                    "delimitador",
                    format!("O texto vem nas linhas seguintes, até uma linha só com {}", t.value),
                ),
                Some("<<<") => (
                    "texto",
                    "Here-string: este texto (com quebra de linha no fim) vira a entrada do comando".to_string(),
                ),
                _ if t.value == "/dev/null" => (
                    "arquivo",
                    "Descarta o que for escrito (\"buraco negro\")".to_string(),
                ),
                _ => (
                    "arquivo",
                    "Arquivo de destino/origem do redirecionamento".to_string(),
                ),
            };
            vec![Row::new([text, label.into(), desc]).indent(1)]
        }
    }
}

/// Label and meaning of a grammar word, given its construct.
fn keyword_text(
    word: &str,
    construct: &str,
    a: &LineAnalysis<'_>,
    index: usize,
) -> (&'static str, String) {
    let previous = index.checked_sub(1).map(|i| a.tokens[i].text.as_str());
    let text = match (word, construct) {
        ("if", _) => "Início do if: roda o comando a seguir e testa o exit code (0 = verdadeiro)",
        ("then", _) => "Bloco executado se a condição for verdadeira (exit code 0)",
        ("elif", _) => "Senão, se: testa outra condição",
        ("else", _) => "Bloco executado se nenhuma condição for verdadeira",
        ("fi", _) => "Fim do if",
        ("for", _) => "Laço: repete os comandos entre do e done para cada item da lista",
        ("select", _) => "Menu: mostra a lista numerada e lê a escolha do usuário",
        ("while", _) => "Laço: repete enquanto o comando de teste terminar com sucesso",
        ("until", _) => "Laço: repete até o comando de teste terminar com sucesso",
        ("in", "case") => "Separa o valor testado dos padrões",
        ("in", _) => {
            "Separa a variável da lista de itens (sem in, o for percorre os argumentos \"$@\")"
        }
        ("do", _) => "Início do corpo do laço",
        ("done", _) => "Fim do laço",
        ("case", _) => "Compara um valor com padrões (glob) e roda o bloco do primeiro que casar",
        ("esac", _) => "Fim do case",
        (";;", _) => "Fim do bloco deste padrão (não continua no próximo)",
        ("|", "case") => "Alternativa: o bloco roda se casar com qualquer um dos padrões",
        ("function", _) => "Define uma função",
        ("{", _) => "Início de um grupo de comandos (roda no shell atual, sem subshell)",
        ("}", _) => "Fim do grupo de comandos",
        ("!", "exit-code") => "Inverte o exit code do comando: sucesso vira falha e vice-versa",
        ("!", _) => "Nega a condição seguinte",
        ("]]", _) => "Fecha o teste [[ ]]",
        ("]", _) => "Fecha o teste (o [ exige ] como último argumento)",
        ("&&", _) => "E lógico: as duas condições precisam ser verdadeiras",
        ("||", _) => "OU lógico: basta uma condição ser verdadeira",
        ("==" | "=", "double-bracket") => {
            "Igual: sem aspas, o lado direito é um padrão glob (ex.: == *.log)"
        }
        ("==" | "=", _) => "Textos iguais",
        ("!=", _) => "Textos diferentes",
        ("=~", _) => "Casa com a regex (ERE) à direita; os grupos ficam em ${BASH_REMATCH[@]}",
        ("(" | ")", _) => "Agrupa condições",
        (w, _) if w.starts_with("((") && previous == Some("for") => {
            "Cabeçalho do for estilo C: início; condição; incremento"
        }
        (w, _) if w.starts_with("((") => {
            "Comando aritmético: calcula a expressão (inteiros); exit code 0 se o resultado for diferente de zero"
        }
        _ => "Palavra reservada do shell",
    };
    let label = match word {
        "&&" | "||" | "==" | "=" | "!=" | "=~" | "!" | "|" => "operador",
        w if w.starts_with("((") => "aritmética",
        _ => "palavra-chave",
    };
    (label, text.to_string())
}

/// One-line meaning of a typed value: permissions, umask.
fn describe_kind(kind: ArgKind, value: &str) -> Option<String> {
    match kind {
        ArgKind::Mode => permissions::describe_mode_argument(value),
        ArgKind::Umask => {
            let mask = permissions::Mode::parse_octal(value)
                .or_else(|| permissions::Mode::parse_octal(&format!("0{value}")))?;
            let (file, dir) = permissions::umask_result(mask);
            Some(format!(
                "arquivos novos nascem com {} ({}), diretórios com {} ({})",
                file.octal(),
                file.symbolic(),
                dir.octal(),
                dir.symbolic()
            ))
        }
        _ => None,
    }
}

/// Breakdown of an embedded program (sed script, awk program).
fn program_rows(kind: ArgKind, value: &str, regex: &dyn RegexAnalyzer, indent: u16) -> Vec<Row> {
    match kind {
        ArgKind::Sed => sed::explain(value)
            .into_iter()
            .map(|p| {
                let mut desc = p.description;
                if let Some(re) = p.regex {
                    desc.push_str(&format!(" · regex: {}", regex.analyze(&re).interpretation));
                }
                Row::new([p.text, p.label.to_string(), desc]).indent(indent)
            })
            .collect(),
        ArgKind::Awk => awk::explain(value)
            .into_iter()
            .map(|p| Row::new([p.text, p.label.to_string(), p.description]).indent(indent))
            .collect(),
        _ => Vec::new(),
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
        "<<-" => "Here-document que ignora tabs no início das linhas (permite indentar)",
        "<<<" => "Here-string: o texto a seguir vira a entrada (stdin) do comando",
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
        TokenKind::String if is_single_expansion(&t.value) => format!(
            "{} (entre aspas: o valor não é dividido em palavras)",
            describe_variable(&t.value)
        ),
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
        TokenKind::Variable => describe_variable(&t.value),
        TokenKind::Substitution if t.text.starts_with("$((") => {
            "Expansão aritmética: o shell calcula a conta (inteiros) e usa o resultado".into()
        }
        TokenKind::Substitution if t.text.starts_with("<(") => {
            "Process substitution: a saída do comando vira um arquivo temporário (/dev/fd/N)".into()
        }
        TokenKind::Substitution => {
            "Substituição: o shell executa o comando interno e usa a saída aqui".into()
        }
        TokenKind::Glob => {
            "Padrão de nomes expandido pelo shell (globbing) antes da execução".into()
        }
        _ => "Argumento".into(),
    }
}

/// `"$VAR"`, `"$1"` or `"${VAR%.*}"`: quotes around one expansion only.
fn is_single_expansion(value: &str) -> bool {
    if let Some(inner) = value.strip_prefix("${") {
        return inner.ends_with('}') && inner.matches('}').count() == 1;
    }
    let Some(name) = value.strip_prefix('$') else {
        return false;
    };
    (name.len() == 1 && "@*#?$!0123456789-_".contains(name))
        || (!name.is_empty()
            && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// `$1`, `$@`, `${VAR:-padrão}`, `${#VAR}`...
fn describe_variable(value: &str) -> String {
    let special = match value {
        "$@" => Some("Todos os argumentos, cada um como uma palavra (use \"$@\", entre aspas)"),
        "$*" => Some("Todos os argumentos juntos numa palavra só (entre aspas)"),
        "$#" => Some("Quantidade de argumentos"),
        "$?" => Some("Exit code do último comando (0 = sucesso)"),
        "$$" => Some("PID do shell atual"),
        "$!" => Some("PID do último processo iniciado em segundo plano (&)"),
        "$0" => Some("Nome do script (ou do shell)"),
        "$-" => Some("Opções ativas do shell"),
        "$_" => Some("Último argumento do comando anterior"),
        _ => None,
    };
    if let Some(s) = special {
        return s.into();
    }
    if let Some(n) = value
        .strip_prefix('$')
        .filter(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
    {
        return format!("Argumento posicional {n} do script ou da função");
    }
    let Some(inner) = value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) else {
        return "Variável expandida pelo shell antes da execução".into();
    };
    if let Some(name) = inner.strip_prefix('#') {
        return format!("Tamanho (em caracteres) do valor de {name}");
    }
    let name_end = inner
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(inner.len());
    let (name, op) = inner.split_at(name_end);
    let arg = |prefix: &str| op[prefix.len()..].to_string();
    match op {
        "" => format!("Variável {name} (as chaves separam o nome do texto ao lado)"),
        o if o.starts_with(":-") => format!(
            "Valor de {name}, ou \"{}\" se estiver vazia ou indefinida",
            arg(":-")
        ),
        o if o.starts_with(":=") => {
            format!("Valor de {name}; se vazia, atribui \"{}\" e usa", arg(":="))
        }
        o if o.starts_with(":?") => format!(
            "Valor de {name}; se vazia, aborta com a mensagem \"{}\"",
            arg(":?")
        ),
        o if o.starts_with(":+") => {
            format!("\"{}\" se {name} tiver valor; senão, vazio", arg(":+"))
        }
        o if o.starts_with("##") => {
            format!("{name} sem o MAIOR prefixo que casa com {}", arg("##"))
        }
        o if o.starts_with('#') => format!("{name} sem o menor prefixo que casa com {}", arg("#")),
        o if o.starts_with("%%") => format!("{name} sem o MAIOR sufixo que casa com {}", arg("%%")),
        o if o.starts_with('%') => format!(
            "{name} sem o menor sufixo que casa com {} (ex.: ${{f%.*}} tira a extensão)",
            arg("%")
        ),
        o if o.starts_with("//") => format!("{name} trocando TODAS as ocorrências ({})", arg("//")),
        o if o.starts_with('/') => format!("{name} trocando a primeira ocorrência ({})", arg("/")),
        "^^" => format!("{name} em MAIÚSCULAS"),
        ",," => format!("{name} em minúsculas"),
        "[@]" | "[*]" => format!("Todos os elementos do array {name}"),
        o if o.starts_with(':') => format!("Trecho de {name} (início:tamanho = {})", arg(":")),
        o if o.starts_with('[') => {
            format!("Elemento {} do array {name}", o.trim_matches(['[', ']']))
        }
        _ => format!("Expansão da variável {name}"),
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
    let not_a_pipeline = a
        .roles
        .iter()
        .any(|r| matches!(r, Role::Operator | Role::Keyword { .. }));
    if a.segments.len() < 2 || not_a_pipeline {
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
    fn shell_grammar_is_explained() {
        let doc = explain(r#"for f in *.log; do gzip "$f"; done"#);
        let r = rows(&doc);
        let find = |text: &str| r.iter().find(|x| x[0] == text).unwrap().clone();
        assert_eq!(find("for")[1], "palavra-chave");
        assert!(find("f")[2].contains("Variável do laço"));
        assert!(find("done")[2].contains("Fim do laço"));
        assert!(find("\"$f\"")[2].contains("não é dividido"));
        assert!(doc.blocks.iter().all(|b| !matches!(b, Block::Flow(_))));

        let doc = explain("[[ $n -gt 10 && -f x ]]");
        let r = rows(&doc);
        let f = r.iter().find(|x| x[0] == "-f").unwrap();
        assert_eq!(f[1], "opção", "{f:?}");
        assert!(f[2].contains("arquivo comum"));
    }

    #[test]
    fn embedded_programs_are_broken_down() {
        let doc = explain("sed -E 's/([0-9]+)-([0-9]+)/\\2-\\1/g' datas.txt");
        let r = rows(&doc);
        assert!(
            r.iter()
                .any(|x| x[1] == "padrão" && x[2].contains("grupo 1")),
            "{r:?}"
        );
        assert!(r.iter().any(|x| x[1] == "flag" && x[2].contains("Todas")));

        let doc = explain("awk -F, 'NR > 1 {s += $3} END {print s}' dados.csv");
        let r = rows(&doc);
        assert!(r.iter().any(|x| x[0] == "END" && x[1] == "padrão"));
        assert!(r.iter().any(|x| x[0] == "$3" && x[2].contains("3º campo")));

        let doc = explain("chmod 750 bin/");
        let r = rows(&doc);
        assert!(r[1][2].contains("rwxr-x---"), "{:?}", r[1]);
    }

    #[test]
    fn variables_and_heredocs() {
        let doc = explain(r#"echo "${arquivo%.*}" $# <<< "$texto""#);
        let r = rows(&doc);
        assert!(r[1][2].contains("menor sufixo"), "{:?}", r[1]);
        assert!(r[2][2].contains("Quantidade de argumentos"));
        assert!(
            r.iter()
                .any(|x| x[0] == "<<<" && x[2].contains("Here-string"))
        );
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
