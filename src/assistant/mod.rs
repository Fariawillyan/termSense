//! The application core: turns the text being typed into suggestions and
//! documents. It orchestrates search, contextual completion, the regex,
//! networking, permission, cron and exit-code analyzers and the explainer —
//! and knows nothing about the terminal.

pub mod explain;
pub mod pages;
pub mod suggestion;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;

use crate::analysis::{cron, exit_code, permissions};
use crate::config::Config;
use crate::document::{Document, Link};
use crate::knowledge::template::render;
use crate::knowledge::{Entry, EntryKind, Repository, Vars};
use crate::networking::{self, Cidr, NetQuery};
use crate::regex::{RegexAnalyzer, RegexTokenKind, StandardRegexAnalyzer};
use crate::search::SearchEngine;
use crate::search::context::{self, Cursor, ExampleMatch, LineAnalysis, Role};
use crate::search::tokenizer::TokenKind;
use crate::system;

use pages::Availability;
pub use suggestion::{Suggestion, SuggestionKind, Target};

/// What kind of input is being answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Home,
    Search,
    Command,
    Regex,
    Network,
    Permissions,
    Cron,
    ExitCode,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Home => "início",
            Mode::Search => "busca",
            Mode::Command => "comando",
            Mode::Regex => "regex",
            Mode::Network => "rede",
            Mode::Permissions => "permissões",
            Mode::Cron => "cron",
            Mode::ExitCode => "exit code",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub mode: Mode,
    pub suggestions: Vec<Suggestion>,
}

/// Recipes expanded into their commands in a result list.
const EXPANDED_RECIPES: usize = 3;
const MAX_RELATED: usize = 6;

pub struct Assistant {
    repo: Repository,
    engine: SearchEngine,
    regex: Box<dyn RegexAnalyzer>,
    max_results: usize,
    /// Parameters of the current query (port, host...) used by templates.
    vars: Vars,
    availability: RefCell<HashMap<String, Availability>>,
}

impl Assistant {
    pub fn new(repo: Repository, config: &Config) -> Self {
        let engine = SearchEngine::new(&repo);
        Self {
            repo,
            engine,
            regex: Box::new(StandardRegexAnalyzer),
            max_results: config.max_results,
            vars: Vars::new(),
            availability: RefCell::new(HashMap::new()),
        }
    }

    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    /// Answers the current input. Called on every keystroke.
    pub fn respond(&mut self, input: &str) -> Response {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            self.vars = Vars::new();
            return self.home();
        }
        let net = networking::scan(input);
        self.vars = net.vars();
        if let Some(pattern) = regex_pattern(input) {
            return self.regex_mode(pattern);
        }
        if let Some(line) = cron_line(trimmed) {
            return self.cron_mode(line);
        }
        if let Some((code, program)) = exit_code::parse_query(trimmed) {
            return self.exit_code_mode(code, program.as_deref());
        }
        if let Some((file_type, mode)) = permission_query(trimmed) {
            return self.permissions_mode(trimmed, file_type, mode);
        }
        if let Some(address) = single_address(trimmed) {
            return self.network_mode(address);
        }
        let analysis = context::analyze(&self.repo, input);
        if is_command_line(&analysis) {
            return self.command_mode(input, &analysis);
        }
        self.search_mode(input, &net)
    }

    /// Document shown for a suggestion (preview pane and detail view).
    pub fn preview(&self, s: &Suggestion) -> Document {
        match &s.target {
            Target::Document(doc) => (**doc).clone(),
            Target::Link(link) => self.open(link),
            Target::Option { entry, flag } => {
                match self
                    .repo
                    .get(entry)
                    .and_then(|e| e.options.iter().find(|o| o.key() == flag).map(|o| (e, o)))
                {
                    Some((e, o)) => pages::option_page(&self.repo, e, o, &self.vars),
                    None => pages::not_found(flag),
                }
            }
        }
    }

    /// Resolves a link to its document.
    pub fn open(&self, link: &Link) -> Document {
        match link {
            Link::Entry(id) => match self.repo.get(id) {
                Some(e) => {
                    let availability = self.availability(e);
                    pages::entry_page(&self.repo, e, &self.vars, availability.as_ref())
                }
                None => pages::not_found(id),
            },
            Link::Command { line, .. } if cron::looks_like_cron(line) => {
                pages::cron_page(&self.repo, line, &cron::parse(line))
            }
            Link::Command { line, note } => {
                explain::explain_line(&self.repo, self.regex.as_ref(), line, note.as_deref())
            }
            Link::Category(id) => pages::category_page(&self.repo, id),
        }
    }

    /// Cached `PATH` lookup for command entries.
    fn availability(&self, e: &Entry) -> Option<Availability> {
        Some(self.which(e.binary()?))
    }

    /// Cached `PATH` lookup (a `stat` per directory; nothing is executed).
    fn which(&self, binary: &str) -> Availability {
        if let Some(a) = self.availability.borrow().get(binary) {
            return a.clone();
        }
        let a = match system::which(binary) {
            Some(path) => Availability::Found(path),
            None => Availability::Missing,
        };
        self.availability
            .borrow_mut()
            .insert(binary.to_string(), a.clone());
        a
    }

    // -----------------------------------------------------------------------
    // Modes
    // -----------------------------------------------------------------------

    fn home(&self) -> Response {
        let mut out = vec![Suggestion::analysis(
            "Como usar o TermSense",
            "Exemplos do que digitar e atalhos de teclado",
            pages::welcome_page(),
        )];
        for cat in self.repo.categories() {
            let count = self.repo.in_category(&cat.id).count();
            if count == 0 {
                continue;
            }
            out.push(Suggestion {
                kind: SuggestionKind::Category,
                title: cat.name.clone(),
                subtitle: format!("{} · {count} itens", cat.description),
                completion: None,
                target: Target::Link(Link::Category(cat.id.clone())),
                indent: 0,
            });
        }
        Response {
            mode: Mode::Home,
            suggestions: out,
        }
    }

    fn search_mode(&self, input: &str, net: &NetQuery) -> Response {
        // Parameters (ports, hosts) are not search terms: "porta 5432" → "porta".
        let query: String = input
            .split_whitespace()
            .filter(|w| {
                !net.parameters
                    .iter()
                    .any(|p| p == w.trim_matches(|c: char| ",;?!".contains(c)))
            })
            .collect::<Vec<_>>()
            .join(" ");
        let only_parameters = query.trim().is_empty();
        let query = if only_parameters { input } else { &query };
        let hits = self.engine.search(query, self.max_results);
        // "8080" alone asks about the port, not about texts that mention it.
        let bare_port = net
            .ports
            .first()
            .is_some_and(|p| input.trim() == p.to_string());
        let confident = !only_parameters && !bare_port && hits.iter().any(|h| h.strong);

        let mut insights = self.network_insights(net);
        let mut out = Vec::new();
        if !confident {
            // Computed facts beat approximate matches ("8080" → the port).
            out.append(&mut insights);
            if out.is_empty() {
                out.push(self.no_match(query, !hits.is_empty()));
            }
        }
        let mut expanded = 0;
        for (rank, hit) in hits.iter().enumerate() {
            let entry = self.repo.entry(hit.index);
            out.push(Suggestion::entry(entry));
            if entry.kind == EntryKind::Recipe && expanded < EXPANDED_RECIPES {
                expanded += 1;
                out.extend(self.recipe_commands(entry));
            }
            if rank == 0 {
                out.append(&mut insights);
            }
        }
        out.append(&mut insights);
        Response {
            mode: Mode::Search,
            suggestions: dedup(out),
        }
    }

    /// First suggestion when nothing matches well: what is known about an
    /// unknown command, or a note that the results are approximate.
    fn no_match(&self, query: &str, has_results: bool) -> Suggestion {
        let first = query.split_whitespace().next().unwrap_or_default();
        let single = !query.trim().contains(char::is_whitespace);
        if is_command_shaped(first) && self.repo.command(first).is_none() {
            let availability = self.which(first);
            let installed = matches!(availability, Availability::Found(_));
            if single || installed {
                let subtitle = match &availability {
                    Availability::Found(path) => {
                        format!(
                            "instalado em {}, mas fora da base · veja man {first}",
                            path.display()
                        )
                    }
                    Availability::Missing if has_results => {
                        "fora da base e do PATH · abaixo, resultados aproximados".to_string()
                    }
                    Availability::Missing => "fora da base e do PATH".to_string(),
                };
                return Suggestion::analysis(
                    format!("{first}: não está na base"),
                    subtitle,
                    pages::unknown_page(first, &availability),
                );
            }
        }
        let subtitle = if has_results {
            "os resultados abaixo são aproximados"
        } else {
            "tente outras palavras"
        };
        Suggestion::analysis(
            format!("Sem correspondência exata para \"{}\"", query.trim()),
            subtitle,
            pages::weak_results_page(query, has_results),
        )
    }

    fn permissions_mode(
        &self,
        input: &str,
        file_type: Option<char>,
        mode: permissions::Mode,
    ) -> Response {
        let doc = pages::permissions_page(&self.repo, mode, file_type);
        let subtitle = (0..3)
            .map(|c| {
                format!(
                    "{} {}",
                    permissions::CLASSES[c],
                    permissions::digit_meaning(mode.digit(c))
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let mut out = vec![Suggestion::analysis(doc.title.clone(), subtitle, doc)];
        out.extend(
            ["chmod", "permissions", "umask", "chown", "stat", "ls"]
                .iter()
                .filter_map(|id| self.repo.get(id))
                .map(Suggestion::entry),
        );
        out.extend(self.search_hits(input, 3));
        Response {
            mode: Mode::Permissions,
            suggestions: dedup(out),
        }
    }

    fn cron_mode(&self, line: &str) -> Response {
        let schedule = cron::parse(line);
        let subtitle = match &schedule {
            Ok(s) => s.summary.clone(),
            Err(e) => e.clone(),
        };
        let doc = pages::cron_page(&self.repo, line, &schedule);
        let mut out = vec![Suggestion::analysis(
            format!("Cron: {}", line.trim()),
            subtitle,
            doc,
        )];
        if let Ok(Some(cmd)) = schedule.as_ref().map(|s| s.command.clone()) {
            out.push(Suggestion::command_line(
                cmd.clone(),
                "comando agendado",
                cmd,
            ));
        }
        out.extend(
            ["cron-syntax", "crontab", "systemctl", "journalctl"]
                .iter()
                .filter_map(|id| self.repo.get(id))
                .map(Suggestion::entry),
        );
        Response {
            mode: Mode::Cron,
            suggestions: dedup(out),
        }
    }

    fn exit_code_mode(&self, code: u16, program: Option<&str>) -> Response {
        let mut out = Vec::new();
        match exit_code::explain(code) {
            Some(info) => {
                let specific = program
                    .and_then(|p| info.programs.iter().find(|(name, _)| *name == p))
                    .map(|(_, m)| (*m).to_string());
                let doc = pages::exit_code_page(&self.repo, &info, program);
                out.push(Suggestion::analysis(
                    doc.title.clone(),
                    specific.unwrap_or_else(|| info.meaning.clone()),
                    doc,
                ));
            }
            None => {
                let mut doc =
                    Document::new(format!("Exit code {code}")).subtitle("código de saída");
                doc.paragraph("Exit codes vão de 0 a 255: o valor é um byte. exit 256 vira 0 e exit 300 vira 44 (resto da divisão por 256).");
                out.push(Suggestion::analysis(
                    format!("Exit code {code}"),
                    "fora da faixa 0–255",
                    doc,
                ));
            }
        }
        let program_entry = program.and_then(|p| self.repo.command(p));
        out.extend(program_entry.map(Suggestion::entry));
        out.extend(
            ["exit-code", "signals", "kill"]
                .iter()
                .filter_map(|id| self.repo.get(id))
                .map(Suggestion::entry),
        );
        Response {
            mode: Mode::ExitCode,
            suggestions: dedup(out),
        }
    }

    /// Permission modes and umasks typed in a command line (`chmod 755`).
    fn mode_analyses(&self, a: &LineAnalysis<'_>) -> Vec<Suggestion> {
        use crate::knowledge::ArgKind;
        let mut out = Vec::new();
        for (t, role) in a.tokens.iter().zip(&a.roles) {
            let kind = match role {
                Role::Argument(Some(spec)) => Some(spec.kind),
                Role::OptionValue(o) => o.kind,
                _ => None,
            };
            let value = t.value.trim_start_matches(['-', '/']);
            match kind {
                Some(ArgKind::Mode) => {
                    if let Some(mode) = permissions::Mode::parse_octal(value) {
                        let doc = pages::permissions_page(&self.repo, mode, None);
                        out.push(Suggestion::analysis(
                            doc.title.clone(),
                            "calculadora de permissões",
                            doc,
                        ));
                    } else if let Some(clauses) = permissions::parse_symbolic_change(value) {
                        let subtitle = clauses
                            .iter()
                            .map(|c| c.description.clone())
                            .collect::<Vec<_>>()
                            .join("; ");
                        let doc = pages::symbolic_change_page(&self.repo, value, &clauses);
                        out.push(Suggestion::analysis(doc.title.clone(), subtitle, doc));
                    }
                }
                Some(ArgKind::Umask) => {
                    let mask = permissions::Mode::parse_octal(value)
                        .or_else(|| permissions::Mode::parse_octal(&format!("0{value}")));
                    if let Some(mask) = mask {
                        let doc = pages::umask_page(&self.repo, mask);
                        let (file, dir) = permissions::umask_result(mask);
                        out.push(Suggestion::analysis(
                            doc.title.clone(),
                            format!("arquivos {} · diretórios {}", file.octal(), dir.octal()),
                            doc,
                        ));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn regex_mode(&self, pattern: &str) -> Response {
        if pattern.is_empty() {
            let mut r = self.search_mode("regex", &NetQuery::default());
            r.mode = Mode::Regex;
            return r;
        }
        let analysis = self.regex.analyze(pattern);
        let mut out = vec![Suggestion::analysis(
            format!("Análise: {pattern}"),
            analysis.interpretation.clone(),
            pages::regex_page(&analysis),
        )];
        for s in self.regex.suggest(pattern) {
            let mut doc = Document::new(&s.snippet).subtitle("completar regex");
            doc.paragraph(format!("{}.", capitalize(&s.description)));
            if let Some(explained) = self.regex.explain(&s.snippet) {
                doc.paragraph(explained);
            }
            doc.heading("RESULTADO");
            doc.code(&s.result, None);
            let result_analysis = self.regex.analyze(&s.result);
            doc.paragraph(result_analysis.interpretation);
            out.push(Suggestion {
                kind: SuggestionKind::Snippet,
                title: s.snippet.clone(),
                subtitle: s.description.clone(),
                completion: Some(format!("regex {}", s.result)),
                target: Target::Document(Box::new(doc)),
                indent: 0,
            });
        }
        // A plain word ("regex email") is more likely a topic than a pattern.
        let topical = pattern.chars().all(|c| c.is_alphanumeric() || c == ' ');
        let hits = self
            .engine
            .search(&format!("regex {pattern}"), self.max_results);
        let pattern_library: Vec<Suggestion> = hits
            .iter()
            .map(|h| self.repo.entry(h.index))
            .filter(|e| e.category == "regex" && topical)
            .map(Suggestion::entry)
            .collect();
        let concepts: Vec<Suggestion> = regex_concepts(&self.regex.parse(pattern))
            .into_iter()
            .filter_map(|id| self.repo.get(id))
            .map(Suggestion::entry)
            .collect();
        if topical && !pattern_library.is_empty() {
            let mut first = pattern_library;
            first.extend(out);
            out = first;
        } else {
            out.extend(pattern_library);
        }
        out.extend(concepts);
        Response {
            mode: Mode::Regex,
            suggestions: dedup(out),
        }
    }

    fn network_mode(&self, address: Address) -> Response {
        let (title, subtitle, doc, related): (String, String, Document, &[&str]) = match address {
            Address::Cidr(c) => {
                let doc = pages::cidr_page(&self.repo, c);
                let subtitle = match c {
                    Cidr::V4 { prefix, .. } => {
                        let n = networking::ipv4_net(None, prefix);
                        format!(
                            "máscara {} · {} endereços, {} hosts utilizáveis",
                            n.mask, n.total, n.usable
                        )
                    }
                    Cidr::V6 { prefix, .. } => format!("2^{} endereços", 128 - prefix),
                };
                (
                    doc.title.clone(),
                    subtitle,
                    doc,
                    &[
                        "cidr", "subnet", "ipv4", "ipv6", "gateway", "ip-addr", "ip-route",
                    ],
                )
            }
            Address::Ip(ip) => {
                let doc = pages::ip_page(&self.repo, ip);
                let subtitle = match ip {
                    IpAddr::V4(v4) => {
                        let c = networking::knowledge::classify_ipv4(v4);
                        format!("{} · {}", c.name, c.description)
                    }
                    IpAddr::V6(_) => "endereço IPv6".to_string(),
                };
                (
                    doc.title.clone(),
                    subtitle,
                    doc,
                    &["ipv4", "ipv6", "nat", "loopback", "cidr", "subnet", "ping"],
                )
            }
        };
        let mut out = vec![Suggestion::analysis(title, subtitle, doc)];
        if let (Address::Ip(_), Some(recipe)) = (&address, self.repo.get("test-connection")) {
            out.push(Suggestion::entry(recipe));
            out.extend(self.recipe_commands(recipe));
        }
        out.extend(
            related
                .iter()
                .filter_map(|id| self.repo.get(id))
                .map(Suggestion::entry),
        );
        Response {
            mode: Mode::Network,
            suggestions: dedup(out),
        }
    }

    fn command_mode(&self, input: &str, a: &LineAnalysis<'_>) -> Response {
        let seg = a.current();
        let cursor = a.cursor();
        let mut out: Vec<Suggestion> = Vec::new();

        let bare_dash = matches!(&cursor, Cursor::Option(p) if p == "-" || p == "--");
        if a.tokens.len() >= 2 && !bare_dash {
            out.push(Suggestion {
                kind: SuggestionKind::Analysis,
                title: format!("Explicar: {}", input.trim()),
                subtitle: "O que cada parte do comando faz".into(),
                completion: None,
                target: Target::Link(Link::Command {
                    line: input.trim().to_string(),
                    note: None,
                }),
                indent: 0,
            });
        }

        out.extend(self.mode_analyses(a));

        let words = plain_words(a);
        if words >= 2 {
            let hits = self.search_hits(input, 5);
            // Prose without any known command is more likely a pasted error
            // message ("! [rejected] main -> main") than a command line.
            let no_known_command = a.segments.iter().all(|s| s.chain.is_empty());
            let prose = no_known_command && words >= 3;
            if prose && !hits.is_empty() {
                out.splice(0..0, hits);
            } else {
                out.extend(hits);
            }
        }

        let replace = |text: &str| format!("{}{}", &input[..a.completion_start(input)], text);
        let examples = || self.example_suggestions(input, a);
        let options = |prefix: &str| -> Vec<Suggestion> {
            let long = prefix.starts_with("--");
            context::option_candidates(seg, prefix)
                .into_iter()
                .filter(|o| !seg.flags.iter().any(|f| f == o.key()) || prefix.len() > 1)
                .map(|o| {
                    let flag = if long {
                        o.long.as_deref()
                    } else {
                        o.short.as_deref()
                    }
                    .or(o.long.as_deref())
                    .unwrap_or_default();
                    let owner = seg
                        .chain
                        .iter()
                        .rev()
                        .find(|e| e.options.iter().any(|x| std::ptr::eq(x, o)));
                    Suggestion {
                        kind: SuggestionKind::Option,
                        title: o.display(),
                        subtitle: o.description.clone(),
                        completion: Some(replace(&format!("{flag} "))),
                        target: Target::Option {
                            entry: owner.map_or_else(String::new, |e| e.id.clone()),
                            flag: o.key().to_string(),
                        },
                        indent: 0,
                    }
                })
                .collect()
        };

        match &cursor {
            Cursor::Subcommand { parent, prefix } => {
                out.extend(
                    context::subcommand_candidates(&self.repo, parent, prefix)
                        .into_iter()
                        .map(|c| Suggestion {
                            completion: Some(replace(&format!("{} ", c.leaf_name()))),
                            ..Suggestion::entry(c)
                        }),
                );
                out.extend(examples());
            }
            Cursor::Option(prefix) => {
                let resolved = matches!(
                    a.roles.last(),
                    Some(Role::Option { .. } | Role::OptionCluster(_))
                );
                let cluster = matches!(a.roles.last(), Some(Role::OptionCluster(_)));
                if resolved {
                    out.extend(examples());
                    if cluster {
                        out.extend(
                            context::cluster_extensions(seg, prefix)
                                .into_iter()
                                .map(|o| {
                                    let c = o.short_char().unwrap_or('?');
                                    let owner =
                                        seg.entry().map_or_else(String::new, |e| e.id.clone());
                                    Suggestion {
                                        kind: SuggestionKind::Option,
                                        title: format!("{prefix}{c}"),
                                        subtitle: format!("+ -{c}: {}", o.description),
                                        completion: Some(replace(&format!("{prefix}{c}"))),
                                        target: Target::Option {
                                            entry: owner,
                                            flag: o.key().to_string(),
                                        },
                                        indent: 0,
                                    }
                                }),
                        );
                    } else {
                        out.extend(options(prefix));
                    }
                } else {
                    out.extend(options(prefix));
                    out.extend(examples());
                }
            }
            Cursor::Empty => {
                if let Some(entry) = seg.entry()
                    && seg.positionals == 0
                    && self.repo.has_children(&entry.id)
                {
                    out.extend(self.repo.children(&entry.id).map(|c| Suggestion {
                        completion: Some(replace(&format!("{} ", c.leaf_name()))),
                        ..Suggestion::entry(c)
                    }));
                }
                out.extend(examples());
                out.extend(options("-"));
            }
            Cursor::Argument | Cursor::Command => {
                out.extend(examples());
                out.extend(options("-"));
            }
        }

        // Documentation of every command in the line, current one first.
        let mut commands: Vec<&Entry> = seg.chain.iter().rev().copied().collect();
        for s in a.segments.iter().rev() {
            commands.extend(s.chain.iter().rev().copied());
            commands.extend(s.wrappers.iter().copied());
        }
        out.extend(commands.into_iter().map(Suggestion::entry));
        out.extend(a.constructs().into_iter().map(Suggestion::entry));
        if let Some(entry) = seg.entry() {
            out.extend(
                self.repo
                    .related(entry)
                    .take(MAX_RELATED)
                    .map(Suggestion::entry),
            );
        }
        Response {
            mode: Mode::Command,
            suggestions: dedup(out),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn search_hits(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let mut out = Vec::new();
        for hit in self.engine.search(query, limit) {
            let entry = self.repo.entry(hit.index);
            out.push(Suggestion::entry(entry));
            if entry.kind == EntryKind::Recipe {
                out.extend(self.recipe_commands(entry));
            }
        }
        out
    }

    /// Steps and examples of a recipe as runnable-looking command lines.
    fn recipe_commands(&self, recipe: &Entry) -> Vec<Suggestion> {
        let steps = recipe.steps.iter().filter_map(|s| {
            s.command
                .as_ref()
                .map(|c| (render(c, &self.vars), format!("{}: {}", s.title, s.why)))
        });
        let examples = recipe
            .examples
            .iter()
            .map(|x| (render(&x.command, &self.vars), x.description.clone()));
        steps
            .chain(examples)
            .map(|(line, desc)| Suggestion::command_line(line.clone(), &desc, line).indented(1))
            .collect()
    }

    fn example_suggestions(&self, input: &str, a: &LineAnalysis<'_>) -> Vec<Suggestion> {
        let prefix = &input[..a.segment_start(input)];
        context::example_candidates(&self.repo, a)
            .into_iter()
            .map(|(x, m)| {
                let line = render(&x.command, &self.vars);
                let mut s = Suggestion::command_line(
                    line.clone(),
                    &x.description,
                    format!("{prefix}{line}"),
                );
                if m == ExampleMatch::StartsWith {
                    s.subtitle = format!("continua o que você digitou · {}", x.description);
                }
                s
            })
            .collect()
    }

    fn network_insights(&self, net: &NetQuery) -> Vec<Suggestion> {
        let mut out = Vec::new();
        if let Some(&port) = net.ports.first() {
            let doc = pages::port_page(&self.repo, port);
            let (title, subtitle) = match networking::knowledge::port_info(port) {
                Some(p) => (
                    format!("Porta {port}/{} · {}", p.transport, p.service),
                    p.description.to_string(),
                ),
                None => (
                    format!("Porta {port}"),
                    networking::knowledge::port_range(port).to_string(),
                ),
            };
            out.push(Suggestion::analysis(title, subtitle, doc));
        }
        for &cidr in &net.cidrs {
            let doc = pages::cidr_page(&self.repo, cidr);
            out.push(Suggestion::analysis(
                doc.title.clone(),
                "calculadora de sub-rede",
                doc,
            ));
        }
        out
    }
}

enum Address {
    Cidr(Cidr),
    Ip(IpAddr),
}

/// `regex <pattern>` → the raw pattern (spaces preserved).
fn regex_pattern(input: &str) -> Option<&str> {
    let rest = input.trim_start();
    let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let word = &rest[..word_end];
    if !(word.eq_ignore_ascii_case("regex") || word.eq_ignore_ascii_case("regexp")) {
        return None;
    }
    let pattern = &rest[word_end..];
    Some(pattern.strip_prefix(' ').unwrap_or(pattern))
}

/// The whole input is a CIDR block or an IP address.
fn single_address(trimmed: &str) -> Option<Address> {
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    if let Some(c) = networking::parse_cidr(trimmed) {
        return Some(Address::Cidr(c));
    }
    let is_ip_shaped = trimmed.contains(':') || trimmed.bytes().filter(|&b| b == b'.').count() == 3;
    trimmed
        .parse::<IpAddr>()
        .ok()
        .filter(|_| is_ip_shaped)
        .map(Address::Ip)
}

/// A command line (as opposed to a search): a known command followed by
/// something, shell grammar (`for x in`), or any shell operator.
fn is_command_line(a: &LineAnalysis<'_>) -> bool {
    let has_operator = a.tokens.iter().any(|t| {
        matches!(
            t.kind,
            TokenKind::Pipe | TokenKind::Operator | TokenKind::Redirect
        )
    });
    let known = a.first_command().is_some() || a.starts_with_keyword();
    has_operator || (known && (a.tokens.len() > 1 || a.trailing_space))
}

/// `cron */5 * * * *`, or a line that already looks like a crontab entry.
fn cron_line(trimmed: &str) -> Option<&str> {
    let (word, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    if word.eq_ignore_ascii_case("cron") && !rest.trim().is_empty() {
        let rest = rest.trim();
        // `cron job` is a search; `cron 0 3 * * *` is a schedule.
        let starts_like_field =
            rest.starts_with(|c: char| c.is_ascii_digit() || c == '*' || c == '@');
        return starts_like_field.then_some(rest);
    }
    cron::looks_like_cron(trimmed).then_some(trimmed)
}

/// `755`, `0644`, `rwxr-xr-x`, `-rw-r--r--`, or `permissão 755`. Bare
/// numbers that are well-known ports (`443`) stay port searches.
fn permission_query(trimmed: &str) -> Option<(Option<char>, permissions::Mode)> {
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let value = match words.as_slice() {
        [v] => *v,
        [k, v]
            if matches!(
                crate::search::tokenizer::normalize(k).as_str(),
                "permissao" | "permissoes" | "permission" | "permissions" | "modo" | "mode"
            ) =>
        {
            *v
        }
        _ => return None,
    };
    if let Some((kind, mode)) = permissions::Mode::parse_symbolic(value) {
        return Some((kind, mode));
    }
    let mode = permissions::Mode::parse_octal(value)?;
    let is_port = value
        .parse::<u16>()
        .ok()
        .and_then(networking::knowledge::port_info)
        .is_some();
    (words.len() == 2 || !is_port).then_some((None, mode))
}

/// A word that could be the name of an executable.
fn is_command_shaped(word: &str) -> bool {
    word.len() >= 2
        && word.len() <= 40
        && word.starts_with(|c: char| c.is_ascii_alphanumeric())
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Positional plain words (not paths, quotes or numbers): a hint that the
/// line is really a natural-language question ("find arquivos grandes").
fn plain_words(a: &LineAnalysis<'_>) -> usize {
    a.tokens
        .iter()
        .zip(&a.roles)
        .filter(|(t, r)| t.kind == TokenKind::Word && matches!(r, Role::Argument(_)))
        .count()
}

/// Knowledge pages for the regex constructs used in a pattern.
fn regex_concepts(tokens: &[crate::regex::RegexToken]) -> Vec<&'static str> {
    use RegexTokenKind as K;
    let mut ids = vec!["regex"];
    for t in tokens {
        let id = match t.kind {
            K::StartAnchor
            | K::EndAnchor
            | K::WordBoundary
            | K::NonWordBoundary
            | K::TextStart
            | K::TextEnd => "regex-anchors",
            K::CharacterClass | K::ShorthandClass | K::UnicodeClass | K::AnyChar => "regex-classes",
            K::Quantifier => "regex-quantifiers",
            K::GroupOpen | K::GroupClose | K::Backreference => "regex-groups",
            K::Alternation => "regex-alternation",
            _ => continue,
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.push("grep");
    ids
}

fn dedup(list: Vec<Suggestion>) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::with_capacity(list.len());
    for s in list {
        if !out.iter().any(|x| x.same_as(&s)) {
            out.push(s);
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant() -> Assistant {
        Assistant::new(Repository::embedded().unwrap(), &Config::default())
    }

    fn titles(r: &Response) -> Vec<&str> {
        r.suggestions.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn modes_are_detected() {
        let mut a = assistant();
        assert_eq!(a.respond("").mode, Mode::Home);
        assert_eq!(a.respond("gr").mode, Mode::Search);
        assert_eq!(a.respond("grep").mode, Mode::Search);
        assert_eq!(a.respond("grep -").mode, Mode::Command);
        assert_eq!(a.respond("regex ^[0-9]+$").mode, Mode::Regex);
        assert_eq!(a.respond("/24").mode, Mode::Network);
        assert_eq!(a.respond("192.168.1.10").mode, Mode::Network);
        assert_eq!(a.respond("porta 8080").mode, Mode::Search);
    }

    #[test]
    fn typing_a_command_shows_its_page_first() {
        let mut a = assistant();
        let r = a.respond("grep");
        assert_eq!(r.suggestions[0].title, "grep");
        let doc = a.preview(&r.suggestions[0]);
        assert_eq!(doc.title, "grep");
    }

    #[test]
    fn dash_lists_options_and_tab_completes() {
        let mut a = assistant();
        let r = a.respond("grep -");
        let first = &r.suggestions[0];
        assert_eq!(first.kind, SuggestionKind::Option);
        let i = r
            .suggestions
            .iter()
            .find(|s| s.title.starts_with("-i"))
            .unwrap();
        assert_eq!(i.completion.as_deref(), Some("grep -i "));
    }

    #[test]
    fn flag_shows_matching_examples() {
        let mut a = assistant();
        let r = a.respond("grep -r");
        assert_eq!(r.suggestions[0].kind, SuggestionKind::Analysis);
        let examples: Vec<&str> = r
            .suggestions
            .iter()
            .filter(|s| s.kind == SuggestionKind::CommandLine)
            .map(|s| s.title.as_str())
            .collect();
        assert!(!examples.is_empty());
        assert!(
            examples.iter().all(|e| e.starts_with("grep -r")),
            "{examples:?}"
        );
    }

    #[test]
    fn subcommands_complete() {
        let mut a = assistant();
        let r = a.respond("git s");
        let t = titles(&r);
        assert!(t.contains(&"git status"), "{t:?}");
        let status = r
            .suggestions
            .iter()
            .find(|s| s.title == "git status")
            .unwrap();
        assert_eq!(status.completion.as_deref(), Some("git status "));
    }

    #[test]
    fn port_query_renders_commands_with_the_port() {
        let mut a = assistant();
        let r = a.respond("quem usa a porta 5432");
        let t = titles(&r);
        assert!(
            t.iter()
                .any(|s| s.contains("ss -ltnp") && s.contains(":5432")),
            "{t:?}"
        );
        assert!(t.iter().any(|s| s.starts_with("Porta 5432")), "{t:?}");
    }

    #[test]
    fn troubleshooting_lists_steps_in_order() {
        let mut a = assistant();
        let r = a.respond("não consigo acessar servidor");
        assert_eq!(r.suggestions[0].kind, SuggestionKind::Recipe);
        let cmds: Vec<&str> = r
            .suggestions
            .iter()
            .skip(1)
            .take(7)
            .map(|s| s.title.as_str())
            .collect();
        assert!(cmds[0].starts_with("dig"), "{cmds:?}");
        assert!(
            cmds.iter().any(|c| c.starts_with("openssl s_client")),
            "{cmds:?}"
        );
    }

    #[test]
    fn regex_mode_analysis_first() {
        let mut a = assistant();
        let r = a.respond("regex ^[0-9]+$");
        assert_eq!(r.suggestions[0].kind, SuggestionKind::Analysis);
        let doc = a.preview(&r.suggestions[0]);
        assert_eq!(doc.title, "Análise de regex");
    }

    #[test]
    fn cidr_and_ip() {
        let mut a = assistant();
        let r = a.respond("/24");
        assert!(r.suggestions[0].subtitle.contains("255.255.255.0"));
        let r = a.respond("10.0.0.1");
        assert!(r.suggestions[0].subtitle.starts_with("privado"));
    }

    #[test]
    fn pipeline_explanation_is_offered() {
        let mut a = assistant();
        let r = a.respond("grep ERROR app.log | sort | uniq");
        assert_eq!(r.mode, Mode::Command);
        let doc = a.preview(&r.suggestions[0]);
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(b, crate::document::Block::Flow(_)))
        );
    }

    /// `entry:ID` for knowledge entries, `analysis:TITLE` for computed ones.
    fn first(a: &mut Assistant, q: &str) -> String {
        let r = a.respond(q);
        let s = r.suggestions.first().expect(q);
        match &s.target {
            Target::Link(Link::Entry(id)) => format!("entry:{id}"),
            _ => format!("{}:{}", s.kind.label(), s.title),
        }
    }

    /// Ranking guard: the first answer for real queries. Update it only when
    /// a change of first result is intended.
    #[test]
    fn golden_queries() {
        let mut a = assistant();
        let cases: &[(&str, &str)] = &[
            ("grep", "entry:grep"),
            ("gr", "entry:grep"),
            ("gerp", "entry:grep"),
            ("crontab", "entry:crontab"),
            ("agendar tarefa", "entry:schedule-task"),
            ("como sair do vim", "entry:exit-vim"),
            ("compactar pasta", "entry:compress-extract"),
            ("instalar programa", "entry:install-program"),
            ("quem usa a porta 8080", "entry:port-owner"),
            ("liberar porta no firewall", "entry:open-firewall-port"),
            ("abrir pasta no windows", "entry:wsl-open-folder"),
            ("recuperar commit", "entry:git-recover-commit"),
            ("desfazer alterações", "entry:git-undo"),
            ("jq", "entry:jq"),
            ("rsync", "entry:rsync"),
            ("useradd", "entry:useradd"),
            ("[[", "entry:double-bracket"),
            ("for", "entry:for"),
            ("variável vazia", "entry:double-bracket"),
            ("copiar saída do comando", "entry:wsl-copy-output"),
            ("wsl lento", "entry:wsl-windows-files"),
            ("tempo limite", "entry:timeout"),
            ("disco cheio", "entry:disk-usage"),
            ("ver processos", "entry:see-processes"),
            ("qual pacote instala", "entry:which-package"),
            ("Permission denied (publickey)", "entry:error-ssh-publickey"),
            (
                "bash: ./x.sh: Permission denied",
                "entry:error-permission-denied",
            ),
            (
                "-bash: ./x.sh: /bin/bash^M: bad interpreter: No such file or directory",
                "entry:error-bad-interpreter",
            ),
            (
                "E: Unable to locate package htop",
                "entry:error-unable-to-locate-package",
            ),
            (
                "sudo: unable to resolve host pc: Name or service not known",
                "entry:error-sudo-resolve-host",
            ),
            (
                "bash: kubectl: command not found",
                "entry:error-command-not-found",
            ),
            (
                "! [rejected]  main -> main (fetch first)",
                "entry:error-git-push-rejected",
            ),
            ("755", "análise:Permissões 755 · rwxr-xr-x"),
            ("*/5 * * * *", "análise:Cron: */5 * * * *"),
            ("exit 127", "análise:Exit code 127"),
            ("8080", "análise:Porta 8080/tcp · HTTP alternativo"),
            ("443", "análise:Porta 443/tcp · HTTPS"),
        ];
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|(q, expected)| {
                let got = first(&mut a, q);
                (got != *expected).then(|| format!("{q:?}: esperado {expected}, veio {got}"))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn unknown_words_are_admitted_not_guessed() {
        let mut a = assistant();
        let r = a.respond("xyzzy");
        assert_eq!(r.suggestions[0].kind, SuggestionKind::Analysis);
        assert!(
            r.suggestions[0]
                .title
                .starts_with("xyzzy: não está na base")
        );
        let doc = a.preview(&r.suggestions[0]);
        assert!(doc.blocks.iter().any(|b| matches!(b, crate::document::Block::Code { text, .. } if text.contains("\"name\": \"xyzzy\""))));

        // Weak, partial matches come after an honest note.
        let r = a.respond("zzzqqq agendar");
        assert!(
            r.suggestions[0]
                .title
                .starts_with("Sem correspondência exata"),
            "{:?}",
            titles(&r)
        );
        assert!(r.suggestions.len() > 1);
    }

    #[test]
    fn analysis_modes() {
        let mut a = assistant();
        let r = a.respond("rwxr-x---");
        assert_eq!(r.mode, Mode::Permissions);
        assert!(r.suggestions[0].title.contains("750"));
        let r = a.respond("0 3 * * 1 /opt/backup.sh");
        assert_eq!(r.mode, Mode::Cron);
        assert!(
            r.suggestions[0]
                .subtitle
                .starts_with("Às 03:00, às segundas-feiras")
        );
        let r = a.respond("cron @reboot /opt/app.sh");
        assert_eq!(r.mode, Mode::Cron);
        let r = a.respond("curl exit 28");
        assert_eq!(r.mode, Mode::ExitCode);
        assert!(r.suggestions[0].subtitle.contains("timeout"));
        let r = a.respond("chmod 640 segredo.txt");
        assert_eq!(r.mode, Mode::Command);
        assert!(
            r.suggestions
                .iter()
                .any(|s| s.title.starts_with("Permissões 640"))
        );
        let r = a.respond("umask 077");
        assert!(
            r.suggestions
                .iter()
                .any(|s| s.subtitle == "arquivos 600 · diretórios 700")
        );
        // A search word that happens to be a port stays a search.
        assert_eq!(a.respond("porta 3306").mode, Mode::Search);
    }

    #[test]
    fn shell_grammar_is_a_command_line() {
        let mut a = assistant();
        let r = a.respond("for f in *.log");
        assert_eq!(r.mode, Mode::Command);
        assert!(titles(&r).contains(&"for"), "{:?}", titles(&r));
        let r = a.respond("while read -r linha; do");
        assert!(titles(&r).contains(&"while"));
    }

    #[test]
    fn every_suggestion_previews() {
        let mut a = assistant();
        for q in [
            "",
            "gr",
            "grep -",
            "git ",
            "porta",
            "regex (a|b",
            "/24",
            "docker exec -it",
            "ssh -",
            "xyzzy",
            "755",
            "*/5 * * * * cmd",
            "exit 137",
            "chmod u+x f",
            "for i in 1 2; do echo $i; done",
            "Permission denied (publickey)",
        ] {
            let r = a.respond(q);
            for s in &r.suggestions {
                let doc = a.preview(s);
                assert!(!doc.title.is_empty(), "{q}: {}", s.title);
            }
        }
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    #[test]
    #[ignore = "medição manual: cargo test --release -- --ignored timing"]
    fn keystroke_latency() {
        let mut a = Assistant::new(Repository::embedded().unwrap(), &Config::default());
        let queries = [
            "g",
            "gr",
            "gre",
            "grep",
            "grep -",
            "grep -r",
            "quem usa a porta 8080",
            "não consigo acessar servidor",
            "regex ^[0-9]+$",
            "ss -ltnp | grep ':8080'",
            "/24",
        ];
        let start = std::time::Instant::now();
        let rounds = 200;
        for _ in 0..rounds {
            for q in queries {
                let r = a.respond(q);
                std::hint::black_box(a.preview(&r.suggestions[0]));
            }
        }
        let per = start.elapsed() / (rounds * queries.len() as u32);
        eprintln!("respond + preview: {per:?} por tecla");
    }
}
