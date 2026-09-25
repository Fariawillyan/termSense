//! Documents for knowledge entries, categories, options and analyses.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use crate::document::{Block, Document, Item, Link, Row, StepItem, Tone};
use crate::knowledge::template::render;
use crate::knowledge::{CommandOption, Entry, EntryKind, Repository, Vars};
use crate::networking::analyzer::ipv6_network;
use crate::networking::knowledge::{classify_ipv4, port_info, port_range};
use crate::networking::{Cidr, ipv4_net};
use crate::regex::RegexAnalysis;
use crate::search::context;

/// Whether a command's executable was found in `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Found(PathBuf),
    Missing,
}

fn command_link(line: &str, note: &str) -> Link {
    Link::Command {
        line: line.to_string(),
        note: (!note.is_empty()).then(|| note.to_string()),
    }
}

/// Detail page of a knowledge entry.
pub fn entry_page(
    repo: &Repository,
    e: &Entry,
    vars: &Vars,
    availability: Option<&Availability>,
) -> Document {
    let category = repo
        .category(&e.category)
        .map_or(e.category.as_str(), |c| c.name.as_str());
    let mut doc = Document::new(&e.name).subtitle(format!("{} · {}", e.kind.label(), category));
    doc.paragraph(format!("{}.", e.summary));
    if let Some(d) = &e.description {
        doc.paragraph(d);
    }
    if let Some(parent) = e.parent.as_deref().and_then(|p| repo.get(p)) {
        doc.list(vec![
            Item::new(format!("subcomando de {}", parent.name))
                .detail(&parent.summary)
                .link(Link::Entry(parent.id.clone())),
        ]);
    }
    if let Some(usage) = &e.usage {
        doc.heading("USO");
        doc.code(usage, None);
    }
    match availability {
        Some(Availability::Found(path)) => {
            doc.note(
                Tone::Success,
                format!("Disponível neste sistema: {}", path.display()),
            );
        }
        Some(Availability::Missing) => {
            doc.note(Tone::Warning, "Não encontrado no PATH deste sistema.");
            if let Some(install) = &e.install {
                doc.note(Tone::Info, install);
            }
        }
        None => {}
    }
    for w in &e.warnings {
        doc.note(Tone::Warning, w);
    }

    let children: Vec<Row> = repo
        .children(&e.id)
        .map(|c| Row::new([c.leaf_name(), &c.summary]).link(Link::Entry(c.id.clone())))
        .collect();
    if !children.is_empty() {
        doc.heading("SUBCOMANDOS");
        doc.table(children);
    }
    if !e.options.is_empty() {
        doc.heading("OPÇÕES");
        doc.table(
            e.options
                .iter()
                .map(|o| Row::new([o.display(), o.description.clone()]))
                .collect(),
        );
    }
    if !e.arguments.is_empty() {
        doc.heading("ARGUMENTOS");
        doc.table(
            e.arguments
                .iter()
                .map(|a| {
                    let name = if a.repeat {
                        format!("{}...", a.name)
                    } else {
                        a.name.clone()
                    };
                    Row::new([name, a.kind.label().to_string(), a.description.clone()])
                })
                .collect(),
        );
    }
    if !e.steps.is_empty() {
        doc.heading("PASSO A PASSO");
        doc.push(Block::Steps(
            e.steps
                .iter()
                .map(|s| {
                    let command = s.command.as_deref().map(|c| render(c, vars));
                    StepItem {
                        link: command.as_deref().map(|c| command_link(c, &s.why)),
                        title: s.title.clone(),
                        command,
                        why: s.why.clone(),
                    }
                })
                .collect(),
        ));
    }
    if !e.examples.is_empty() {
        doc.heading("EXEMPLOS");
        for x in &e.examples {
            let line = render(&x.command, vars);
            let link = command_link(&line, &x.description);
            doc.example(line, &x.description, Some(link));
        }
    }
    for s in &e.sections {
        doc.heading(s.title.to_uppercase());
        if let Some(t) = &s.text {
            doc.paragraph(t);
        }
        doc.list(s.items.iter().map(Item::new).collect());
        doc.table(s.rows.iter().map(|r| Row::new(r.iter().cloned())).collect());
    }
    let related: Vec<Item> = repo
        .related(e)
        .map(|r| {
            Item::new(&r.name)
                .detail(&r.summary)
                .link(Link::Entry(r.id.clone()))
        })
        .collect();
    if !related.is_empty() {
        doc.heading("RELACIONAMENTOS");
        doc.push(Block::Tree {
            root: e.name.clone(),
            children: related,
        });
    }
    doc
}

/// Entries of a category grouped by kind.
pub fn category_page(repo: &Repository, id: &str) -> Document {
    let Some(cat) = repo.category(id) else {
        return not_found(id);
    };
    let mut doc = Document::new(&cat.name).subtitle("tema");
    if !cat.description.is_empty() {
        doc.paragraph(&cat.description);
    }
    for (kind, heading) in [
        (EntryKind::Command, "COMANDOS"),
        (EntryKind::Recipe, "RECEITAS"),
        (EntryKind::Concept, "CONCEITOS"),
    ] {
        let mut entries: Vec<&Entry> = repo.in_category(id).filter(|e| e.kind == kind).collect();
        if entries.is_empty() {
            continue;
        }
        entries.sort_by_key(|e| e.name.to_lowercase());
        doc.heading(format!("{heading} ({})", entries.len()));
        doc.table(
            entries
                .iter()
                .map(|e| Row::new([&e.name, &e.summary]).link(Link::Entry(e.id.clone())))
                .collect(),
        );
    }
    doc
}

/// One option of a command, with the examples that use it.
pub fn option_page(
    repo: &Repository,
    entry: &Entry,
    option: &CommandOption,
    vars: &Vars,
) -> Document {
    let mut doc = Document::new(option.display()).subtitle(format!("opção de {}", entry.name));
    doc.paragraph(&option.description);
    if let Some(arg) = &option.arg {
        doc.paragraph(format!("Recebe um valor: {arg}."));
    }
    let examples: Vec<(String, &str)> = entry
        .examples
        .iter()
        .map(|x| (render(&x.command, vars), x.description.as_str()))
        .filter(|(line, _)| {
            context::analyze(repo, line)
                .segments
                .iter()
                .any(|s| s.flags.iter().any(|f| f == option.key()))
        })
        .collect();
    if !examples.is_empty() {
        doc.heading(format!("EXEMPLOS COM {}", option.key()));
        for (line, desc) in examples {
            let link = command_link(&line, desc);
            doc.example(line, desc, Some(link));
        }
    }
    doc.heading("COMANDO");
    doc.list(vec![
        Item::new(&entry.name)
            .detail(&entry.summary)
            .link(Link::Entry(entry.id.clone())),
    ]);
    doc
}

/// Regex analysis page.
pub fn regex_page(a: &RegexAnalysis) -> Document {
    let mut doc = Document::new("Análise de regex").subtitle("regex");
    doc.heading("PADRÃO");
    doc.code(&a.pattern, None);
    doc.heading("TOKENS");
    doc.table(
        a.tokens
            .iter()
            .map(|t| {
                let text = if t.text.is_empty() {
                    "∅".to_string()
                } else {
                    t.text.clone()
                };
                let row = Row::new([text, t.label.to_string(), t.description.clone()])
                    .indent(t.depth as u16);
                if t.invalid {
                    row.tone(Tone::Danger)
                } else {
                    row
                }
            })
            .collect(),
    );
    doc.heading("INTERPRETAÇÃO");
    doc.paragraph(&a.interpretation);
    if let Some(err) = &a.error {
        doc.note(
            Tone::Warning,
            format!("A crate regex não aceita este padrão: {err}"),
        );
    }
    let show = |s: &String| {
        if s.is_empty() {
            Item::new("(string vazia)")
        } else {
            Item::new(s)
        }
    };
    if !a.matches.is_empty() {
        doc.heading("CASA");
        doc.list(a.matches.iter().map(show).collect());
    }
    if !a.non_matches.is_empty() {
        doc.heading("NÃO CASA");
        doc.list(a.non_matches.iter().map(show).collect());
    }
    if !a.notes.is_empty() {
        doc.heading("PORTABILIDADE");
        for n in &a.notes {
            doc.note(Tone::Info, n);
        }
    }
    if !a.pattern.is_empty() {
        let quoted = shell_quote(&a.pattern);
        doc.heading("COMO USAR");
        for (line, caption) in [
            (
                format!("grep -E {quoted} arquivo.txt"),
                "Filtra linhas que casam (regex estendida)",
            ),
            (
                format!("grep -oE {quoted} arquivo.txt"),
                "Mostra só o trecho que casou",
            ),
            (
                format!(
                    "sed -E 's/{}/X/g' arquivo.txt",
                    a.pattern.replace('/', "\\/").replace('\'', "'\\''")
                ),
                "Substitui cada ocorrência por X",
            ),
        ] {
            let link = command_link(&line, caption);
            doc.example(line, caption, Some(link));
        }
    }
    doc
}

/// Wraps a value in single quotes for the shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// CIDR calculator page.
pub fn cidr_page(repo: &Repository, cidr: Cidr) -> Document {
    match cidr {
        Cidr::V4 { addr, prefix } => {
            let n = ipv4_net(addr, prefix);
            let title = match addr {
                Some(a) => format!("CIDR {a}/{prefix}"),
                None => format!("CIDR /{prefix}"),
            };
            let mut doc = Document::new(title).subtitle("sub-rede IPv4");
            let mut rows = vec![
                Row::new(["Máscara".to_string(), n.mask.to_string()]),
                Row::new(["Máscara (binário)".to_string(), binary(n.mask)]),
                Row::new([
                    "Wildcard".to_string(),
                    format!("{} (máscara invertida, usada em ACLs)", n.wildcard),
                ]),
                Row::new([
                    "Bits".to_string(),
                    format!("{prefix} de rede + {} de host", 32 - prefix),
                ]),
                Row::new([
                    "Endereços totais".to_string(),
                    format!("{} (2^{})", group(n.total), 32 - prefix),
                ])
                .tone(Tone::Accent),
                Row::new([
                    "Hosts utilizáveis".to_string(),
                    format!("{} {}", group(n.usable), usable_note(prefix)),
                ])
                .tone(Tone::Accent),
            ];
            if let (Some(a), Some(net), Some(first), Some(last)) =
                (addr, n.network, n.first_host, n.last_host)
            {
                rows.push(Row::new(["Endereço".to_string(), a.to_string()]));
                rows.push(Row::new(["Rede".to_string(), format!("{net}/{prefix}")]));
                if let Some(b) = n.broadcast {
                    rows.push(Row::new(["Broadcast".to_string(), b.to_string()]));
                }
                rows.push(Row::new([
                    "Faixa de hosts".to_string(),
                    format!("{first} – {last}"),
                ]));
                let class = classify_ipv4(net);
                rows.push(Row::new([
                    "Tipo".to_string(),
                    format!("{} — {}", class.name, class.description),
                ]));
            }
            doc.table(rows);
            doc.heading("TOTAIS × UTILIZÁVEIS");
            doc.paragraph(
                "Endereços totais contam todo o bloco. Em redes IPv4 comuns, o primeiro endereço identifica a rede e o último é o broadcast, por isso não são atribuídos a hosts (total − 2). Exceções: /31 (enlaces ponto a ponto, RFC 3021) usa os 2 endereços e /32 representa um único host.",
            );
            if addr.is_none() {
                doc.paragraph(format!(
                    "Dica: informe um endereço, por exemplo 192.168.1.10/{prefix}, para ver rede, broadcast e faixa de hosts."
                ));
            }
            commands_from(repo, &mut doc, "cidr", &Vars::new());
            doc
        }
        Cidr::V6 { addr, prefix } => {
            let title = match addr {
                Some(a) => format!("CIDR {a}/{prefix}"),
                None => format!("CIDR /{prefix} (IPv6)"),
            };
            let mut doc = Document::new(title).subtitle("sub-rede IPv6");
            let mut rows = vec![
                Row::new([
                    "Bits".to_string(),
                    format!("{prefix} de prefixo + {} de interface", 128 - prefix),
                ]),
                Row::new([
                    "Endereços totais".to_string(),
                    format!("2^{}", 128 - prefix),
                ])
                .tone(Tone::Accent),
            ];
            if let Some(a) = addr {
                rows.push(Row::new([
                    "Rede".to_string(),
                    format!("{}/{prefix}", ipv6_network(a, prefix)),
                ]));
                rows.push(Row::new(["Tipo".to_string(), ipv6_kind(a).to_string()]));
            }
            doc.table(rows);
            doc.paragraph("IPv6 não tem broadcast: todos os endereços do bloco podem ser usados. /64 é o tamanho padrão de uma sub-rede (necessário para SLAAC); /48 e /56 são blocos típicos entregues a organizações e residências.");
            doc
        }
    }
}

/// Classification page for a single IP address.
pub fn ip_page(repo: &Repository, ip: IpAddr) -> Document {
    let mut doc;
    match ip {
        IpAddr::V4(v4) => {
            let class = classify_ipv4(v4);
            doc = Document::new(format!("IPv4 {v4}")).subtitle("endereço");
            let mut rows = vec![
                Row::new(["Tipo".to_string(), class.name.to_string()]).tone(Tone::Accent),
                Row::new(["Descrição".to_string(), class.description.to_string()]),
            ];
            if class.prefix > 0 {
                rows.push(Row::new([
                    "Bloco".to_string(),
                    format!("{}/{}", Ipv4Addr::from(class.network), class.prefix),
                ]));
            }
            rows.push(Row::new(["Binário".to_string(), binary(v4)]));
            rows.push(Row::new([
                "Classe histórica".to_string(),
                format!("{} (obsoleta: hoje se usa CIDR)", historic_class(v4)),
            ]));
            doc.table(rows);
        }
        IpAddr::V6(v6) => {
            doc = Document::new(format!("IPv6 {v6}")).subtitle("endereço");
            doc.table(vec![
                Row::new(["Tipo".to_string(), ipv6_kind(v6).to_string()]).tone(Tone::Accent),
            ]);
        }
    }
    let mut vars = Vars::new();
    vars.set("host", ip.to_string());
    commands_from(repo, &mut doc, "test-connection", &vars);
    doc
}

/// Facts about a port plus the commands to investigate it.
pub fn port_page(repo: &Repository, port: u16) -> Document {
    let info = port_info(port);
    let mut doc =
        Document::new(format!("Porta {port}")).subtitle(info.map_or("porta", |i| i.service));
    let mut rows = Vec::new();
    if let Some(i) = info {
        rows.push(Row::new(["Serviço", i.service]).tone(Tone::Accent));
        rows.push(Row::new(["Transporte", i.transport]));
        rows.push(Row::new(["Uso típico", i.description]));
    }
    rows.push(Row::new(["Faixa", port_range(port)]));
    doc.table(rows);
    let mut vars = Vars::new();
    vars.set("port", port.to_string());
    commands_from(repo, &mut doc, "port-owner", &vars);
    doc
}

/// Appends the examples of entry `id` rendered with `vars`, if it exists.
fn commands_from(repo: &Repository, doc: &mut Document, id: &str, vars: &Vars) {
    let Some(e) = repo.get(id) else { return };
    let lines: Vec<(String, &str)> = e
        .examples
        .iter()
        .map(|x| (render(&x.command, vars), x.description.as_str()))
        .chain(e.steps.iter().filter_map(|s| {
            s.command
                .as_ref()
                .map(|c| (render(c, vars), s.title.as_str()))
        }))
        .collect();
    if lines.is_empty() {
        return;
    }
    doc.heading("COMANDOS");
    for (line, caption) in lines {
        let link = command_link(&line, caption);
        doc.example(line, caption, Some(link));
    }
    doc.list(vec![
        Item::new(&e.name)
            .detail(&e.summary)
            .link(Link::Entry(e.id.clone())),
    ]);
}

fn usable_note(prefix: u8) -> &'static str {
    match prefix {
        32 => "(um único host)",
        31 => "(ponto a ponto: sem rede/broadcast)",
        _ => "(total − rede − broadcast)",
    }
}

fn binary(ip: Ipv4Addr) -> String {
    ip.octets()
        .iter()
        .map(|o| format!("{o:08b}"))
        .collect::<Vec<_>>()
        .join(".")
}

/// Thousands separated with dots, pt-BR style.
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn historic_class(ip: Ipv4Addr) -> char {
    match ip.octets()[0] {
        0..=127 => 'A',
        128..=191 => 'B',
        192..=223 => 'C',
        224..=239 => 'D',
        _ => 'E',
    }
}

fn ipv6_kind(ip: Ipv6Addr) -> &'static str {
    let s = ip.segments();
    if ip.is_loopback() {
        "loopback (::1): o próprio host"
    } else if ip.is_unspecified() {
        "não especificado (::): todas as interfaces"
    } else if s[0] & 0xffc0 == 0xfe80 {
        "link-local (fe80::/10): válido só no enlace local"
    } else if s[0] & 0xfe00 == 0xfc00 {
        "ULA (fc00::/7): privado, equivalente às redes RFC 1918"
    } else if s[0] & 0xff00 == 0xff00 {
        "multicast (ff00::/8)"
    } else if s[0] == 0x2001 && s[1] == 0x0db8 {
        "documentação (2001:db8::/32)"
    } else {
        "global unicast: roteável na internet"
    }
}

/// Getting-started page shown on the home screen.
pub fn welcome_page() -> Document {
    let mut doc = Document::new("TermSense").subtitle("assistente de conhecimento do terminal");
    doc.paragraph(
        "Digite e os resultados se atualizam a cada tecla. Busque comandos, conceitos ou descreva o que quer fazer em linguagem natural. Tudo é local, offline e determinístico.",
    );
    doc.heading("EXPERIMENTE");
    doc.table(
        [
            ("gr", "descobrir comandos pelo prefixo"),
            ("grep -", "listar as opções de um comando"),
            ("grep -r", "exemplos com as flags digitadas"),
            ("git s", "subcomandos"),
            (
                "ss -ltnp | grep ':8080'",
                "explicar um comando parte por parte",
            ),
            ("quem usa a porta 8080", "perguntar em linguagem natural"),
            (
                "não consigo acessar servidor",
                "troubleshooting de rede guiado",
            ),
            ("regex ^[0-9]+$", "analisar uma expressão regular"),
            ("/24", "calcular uma sub-rede CIDR"),
            ("192.168.1.10", "classificar um endereço IP"),
        ]
        .into_iter()
        .map(|(q, d)| Row::new([q, d]))
        .collect(),
    );
    doc.heading("TECLAS");
    doc.table(
        [
            ("↑ ↓", "navegar nos resultados"),
            ("Enter", "abrir detalhes"),
            ("Tab", "completar a entrada com a sugestão"),
            ("← →", "mover o cursor"),
            ("PgUp PgDn", "rolar a pré-visualização"),
            ("Ctrl+U / Ctrl+W", "apagar a linha / a palavra"),
            ("Esc / Ctrl+C", "sair (nos detalhes, Esc volta)"),
        ]
        .into_iter()
        .map(|(k, d)| Row::new([k, d]))
        .collect(),
    );
    doc.note(
        Tone::Info,
        "O TermSense apenas consulta: nunca executa comandos, altera arquivos ou acessa a rede.",
    );
    doc
}

pub fn not_found(what: &str) -> Document {
    let mut doc = Document::new("Não encontrado");
    doc.paragraph(format!("\"{what}\" não existe na base de conhecimento."));
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repository {
        Repository::embedded().unwrap()
    }

    fn table_text(doc: &Document) -> String {
        doc.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Table(rows) => Some(
                    rows.iter()
                        .map(|r| r.cells.join(" | "))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn cidr_24_distinguishes_total_and_usable() {
        let doc = cidr_page(
            &repo(),
            Cidr::V4 {
                addr: None,
                prefix: 24,
            },
        );
        assert_eq!(doc.title, "CIDR /24");
        let t = table_text(&doc);
        assert!(t.contains("Máscara | 255.255.255.0"), "{t}");
        assert!(t.contains("Endereços totais | 256 (2^8)"), "{t}");
        assert!(t.contains("Hosts utilizáveis | 254"), "{t}");
    }

    #[test]
    fn cidr_with_address() {
        let doc = cidr_page(
            &repo(),
            Cidr::V4 {
                addr: Some(Ipv4Addr::new(10, 1, 2, 3)),
                prefix: 8,
            },
        );
        let t = table_text(&doc);
        assert!(t.contains("Rede | 10.0.0.0/8"));
        assert!(t.contains("Broadcast | 10.255.255.255"));
        assert!(t.contains("Endereços totais | 16.777.216"));
        assert!(t.contains("privado"));
    }

    #[test]
    fn port_page_renders_commands_with_the_port() {
        let doc = port_page(&repo(), 5432);
        assert_eq!(doc.subtitle.as_deref(), Some("PostgreSQL"));
        let codes: Vec<&String> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(codes.iter().any(|c| c.contains(":5432")), "{codes:?}");
    }

    #[test]
    fn entry_page_has_spec_sections() {
        let r = repo();
        let grep = r.get("grep").unwrap();
        let doc = entry_page(&r, grep, &Vars::new(), None);
        let headings: Vec<&str> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading(h) => Some(h.as_str()),
                _ => None,
            })
            .collect();
        for h in ["USO", "OPÇÕES", "EXEMPLOS", "RELACIONAMENTOS"] {
            assert!(headings.contains(&h), "{headings:?}");
        }
        assert!(!doc.links().is_empty());
    }

    #[test]
    fn number_grouping() {
        assert_eq!(group(256), "256");
        assert_eq!(group(16_777_216), "16.777.216");
        assert_eq!(group(4_294_967_296), "4.294.967.296");
    }
}
