//! Cron schedules: `*/5 * * * *` → "A cada 5 minutos, todos os dias".
//!
//! Explains the five fields and builds a sentence in Portuguese. There is no
//! "next run" list: that would depend on the clock and break determinism.

use super::permissions::join_pt;

/// One of the five time fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub raw: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// `@daily` and friends, when used.
    pub shortcut: Option<String>,
    pub fields: Vec<Field>,
    pub summary: String,
    pub command: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Spec {
    name: &'static str,
    min: u8,
    max: u8,
    names: &'static [&'static str],
}

const MINUTE: Spec = Spec {
    name: "minuto",
    min: 0,
    max: 59,
    names: &[],
};
const HOUR: Spec = Spec {
    name: "hora",
    min: 0,
    max: 23,
    names: &[],
};
const DAY: Spec = Spec {
    name: "dia do mês",
    min: 1,
    max: 31,
    names: &[],
};
const MONTH: Spec = Spec {
    name: "mês",
    min: 1,
    max: 12,
    names: &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ],
};
const WEEKDAY: Spec = Spec {
    name: "dia da semana",
    min: 0,
    max: 7,
    names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
};

const MONTHS: [&str; 12] = [
    "janeiro",
    "fevereiro",
    "março",
    "abril",
    "maio",
    "junho",
    "julho",
    "agosto",
    "setembro",
    "outubro",
    "novembro",
    "dezembro",
];
const WEEKDAYS: [&str; 7] = [
    "domingo",
    "segunda-feira",
    "terça-feira",
    "quarta-feira",
    "quinta-feira",
    "sexta-feira",
    "sábado",
];

const SHORTCUTS: &[(&str, Option<&str>, &str)] = &[
    (
        "@reboot",
        None,
        "Uma vez, quando o sistema (o serviço cron) inicia",
    ),
    (
        "@yearly",
        Some("0 0 1 1 *"),
        "Uma vez por ano, à meia-noite de 1º de janeiro",
    ),
    (
        "@annually",
        Some("0 0 1 1 *"),
        "Uma vez por ano, à meia-noite de 1º de janeiro",
    ),
    (
        "@monthly",
        Some("0 0 1 * *"),
        "Uma vez por mês, à meia-noite do dia 1",
    ),
    (
        "@weekly",
        Some("0 0 * * 0"),
        "Uma vez por semana, à meia-noite de domingo",
    ),
    ("@daily", Some("0 0 * * *"), "Uma vez por dia, à meia-noite"),
    (
        "@midnight",
        Some("0 0 * * *"),
        "Uma vez por dia, à meia-noite",
    ),
    (
        "@hourly",
        Some("0 * * * *"),
        "Uma vez por hora, no minuto 0",
    ),
];

/// A parsed field: every value it matches, plus its shape.
#[derive(Debug, Clone)]
struct Parsed {
    values: Vec<u8>,
    any: bool,
    /// `*/n` — every n units over the whole range.
    step: Option<u8>,
    /// A single `a-b` range.
    range: Option<(u8, u8)>,
}

fn parse_value(s: &str, spec: Spec) -> Option<u8> {
    if let Ok(v) = s.parse::<u8>() {
        return (spec.min..=spec.max).contains(&v).then_some(v);
    }
    let lower = s.to_ascii_lowercase();
    let i = spec.names.iter().position(|n| *n == lower)?;
    Some(i as u8 + if spec.min == 1 { 1 } else { 0 })
}

fn parse_field(raw: &str, spec: Spec) -> Result<Parsed, String> {
    let bad = || format!("valor inválido no campo {}: {raw}", spec.name);
    let mut values = Vec::new();
    let parts: Vec<&str> = raw.split(',').collect();
    let mut step_all = None;
    let mut range = None;
    for part in &parts {
        let (base, step) = match part.split_once('/') {
            Some((b, s)) => {
                let s: u8 = s.parse().map_err(|_| bad())?;
                if s == 0 {
                    return Err(bad());
                }
                (b, Some(s))
            }
            None => (*part, None),
        };
        let (lo, hi) = if base == "*" {
            if parts.len() == 1 {
                step_all = step;
            }
            (spec.min, spec.max)
        } else if let Some((a, b)) = base.split_once('-') {
            let (a, b) = (
                parse_value(a, spec).ok_or_else(bad)?,
                parse_value(b, spec).ok_or_else(bad)?,
            );
            if a > b {
                return Err(bad());
            }
            if parts.len() == 1 && step.is_none() {
                range = Some((a, b));
            }
            (a, b)
        } else {
            let v = parse_value(base, spec).ok_or_else(bad)?;
            // `5/10` means "from 5 to the end, every 10".
            (v, if step.is_some() { spec.max } else { v })
        };
        let mut v = lo;
        loop {
            values.push(v);
            match v.checked_add(step.unwrap_or(1)) {
                Some(next) if next <= hi => v = next,
                _ => break,
            }
        }
    }
    if spec.name == WEEKDAY.name {
        for v in &mut values {
            if *v == 7 {
                *v = 0;
            }
        }
    }
    values.sort_unstable();
    values.dedup();
    let full = usize::from(spec.max - spec.min) + 1 - usize::from(spec.name == WEEKDAY.name);
    Ok(Parsed {
        any: raw == "*" || (step_all.is_none() && values.len() == full && raw.contains('*')),
        values,
        step: step_all,
        range,
    })
}

fn is_field_shaped(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'*' | b'/' | b',' | b'-'))
}

/// Cheap check used on every keystroke: the line starts with a cron shortcut
/// or with five valid cron fields, at least one of them using `*`.
pub fn looks_like_cron(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    if first.starts_with('@') {
        return SHORTCUTS.iter().any(|(s, ..)| *s == first);
    }
    let fields: Vec<&str> = std::iter::once(first).chain(words.take(4)).collect();
    fields.len() == 5
        && fields.iter().all(|f| is_field_shaped(f))
        && fields.iter().any(|f| f.contains('*'))
        && parse(line).is_ok()
}

/// Parses a crontab line: five fields (or a shortcut) plus an optional command.
pub fn parse(line: &str) -> Result<Schedule, String> {
    let line = line.trim();
    let first = line.split_whitespace().next().unwrap_or_default();
    if first.starts_with('@') {
        let (_, expansion, text) = SHORTCUTS
            .iter()
            .find(|(s, ..)| *s == first)
            .ok_or_else(|| format!("atalho desconhecido: {first}"))?;
        let command = rest_after(line, 1);
        let mut schedule = match expansion {
            Some(fields) => parse(fields)?,
            None => Schedule {
                shortcut: None,
                fields: Vec::new(),
                summary: String::new(),
                command: None,
                notes: Vec::new(),
            },
        };
        schedule.shortcut = Some(first.to_string());
        schedule.summary = format!("{text}.");
        schedule.command = command;
        schedule.notes = notes(schedule.command.as_deref(), false, None);
        return Ok(schedule);
    }

    let raw: Vec<&str> = line.split_whitespace().take(5).collect();
    if raw.len() < 5 {
        return Err(
            "uma linha de cron tem 5 campos: minuto hora dia-do-mês mês dia-da-semana".into(),
        );
    }
    let specs = [MINUTE, HOUR, DAY, MONTH, WEEKDAY];
    let mut parsed = Vec::with_capacity(5);
    for (r, spec) in raw.iter().zip(specs) {
        parsed.push(parse_field(r, spec)?);
    }
    let [min, hour, day, month, weekday] = [0, 1, 2, 3, 4].map(|i| &parsed[i]);

    let fields = raw
        .iter()
        .zip(specs)
        .zip(&parsed)
        .map(|((r, spec), p)| Field {
            name: spec.name,
            raw: (*r).to_string(),
            description: describe_field(p, spec),
        })
        .collect();

    let mut summary = time_phrase(min, hour);
    let days = day_phrase(day, weekday, month);
    summary.push_str(", ");
    summary.push_str(&days);
    summary.push('.');
    let command = rest_after(line, 5);
    let irregular = min
        .step
        .filter(|s| 60 % u16::from(*s) != 0)
        .map(|s| format!("*/{s} nos minutos recomeça a cada hora: roda em 0, {s}, {}... e de novo em 0, então o intervalo na virada da hora é irregular.", 2 * u16::from(s)));
    Ok(Schedule {
        shortcut: None,
        fields,
        summary: capitalize(&summary),
        notes: notes(command.as_deref(), !day.any && !weekday.any, irregular),
        command,
    })
}

fn rest_after(line: &str, fields: usize) -> Option<String> {
    let mut rest = line;
    for _ in 0..fields {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = &rest[end..];
    }
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn notes(command: Option<&str>, day_or: bool, irregular: Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    if day_or {
        out.push("Dia do mês e dia da semana restritos ao mesmo tempo: o cron roda quando QUALQUER um dos dois bate (é OU, não E).".into());
    }
    out.extend(irregular);
    if let Some(cmd) = command
        && has_unescaped_percent(cmd)
    {
        out.push(
            "No crontab, % no comando vira quebra de linha: escreva \\% (ex.: date +\\%F).".into(),
        );
    }
    out.push("Os horários seguem o fuso do sistema (timedatectl mostra qual é).".into());
    out.push("O cron roda com um ambiente mínimo (PATH curto, sem o seu .bashrc): use caminhos absolutos.".into());
    out.push("Guarde a saída para depurar: comando >> /tmp/tarefa.log 2>&1".into());
    out
}

fn has_unescaped_percent(cmd: &str) -> bool {
    let b = cmd.as_bytes();
    b.iter()
        .enumerate()
        .any(|(i, &c)| c == b'%' && (i == 0 || b[i - 1] != b'\\'))
}

fn describe_field(p: &Parsed, spec: Spec) -> String {
    let unit = spec.name;
    if p.any {
        return match spec.name {
            "minuto" => "todo minuto".into(),
            "hora" => "toda hora".into(),
            "dia do mês" => "todo dia do mês".into(),
            "mês" => "todo mês".into(),
            _ => "todo dia da semana".into(),
        };
    }
    if let Some(s) = p.step {
        let plural = match spec.name {
            "minuto" => "minutos",
            "hora" => "horas",
            "mês" => "meses",
            _ => "dias",
        };
        return format!("a cada {s} {plural}");
    }
    let name = |v: u8| value_name(v, spec);
    if let Some((a, b)) = p.range {
        return format!("{unit}: de {} a {}", name(a), name(b));
    }
    let names: Vec<String> = p.values.iter().map(|v| name(*v)).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    format!("{unit}: {}", join_pt(&refs))
}

fn value_name(v: u8, spec: Spec) -> String {
    match spec.name {
        "mês" => MONTHS[usize::from(v - 1)].to_string(),
        "dia da semana" => WEEKDAYS[usize::from(v % 7)].to_string(),
        "hora" => format!("{v}h"),
        _ => v.to_string(),
    }
}

fn hhmm(h: u8, m: u8) -> String {
    format!("{h:02}:{m:02}")
}

fn time_phrase(min: &Parsed, hour: &Parsed) -> String {
    let single_min = (min.values.len() == 1).then(|| min.values[0]);
    match (single_min, hour) {
        (Some(m), h) if !h.any && h.step.is_none() && h.range.is_none() && h.values.len() <= 8 => {
            let times: Vec<String> = h.values.iter().map(|&h| hhmm(h, m)).collect();
            let refs: Vec<&str> = times.iter().map(String::as_str).collect();
            format!("às {}", join_pt(&refs))
        }
        (Some(0), h) if h.any => "de hora em hora, no minuto 0".into(),
        (Some(m), h) if h.any => format!("no minuto {m} de cada hora"),
        (Some(m), h) if h.step.is_some() => {
            format!("a cada {} horas, no minuto {m}", h.step.unwrap_or(1))
        }
        (Some(m), h) if h.range.is_some() => {
            let (a, b) = h.range.unwrap_or_default();
            format!("no minuto {m} de cada hora, das {a}h às {b}h")
        }
        _ => {
            let minutes = if min.any {
                "a cada minuto".to_string()
            } else if let Some(s) = min.step {
                format!("a cada {s} minutos")
            } else if let Some((a, b)) = min.range {
                format!("a cada minuto do {a} ao {b}")
            } else {
                let v: Vec<String> = min.values.iter().map(u8::to_string).collect();
                let refs: Vec<&str> = v.iter().map(String::as_str).collect();
                format!("nos minutos {}", join_pt(&refs))
            };
            let hours = if hour.any {
                String::new()
            } else if let Some(s) = hour.step {
                format!(", a cada {s} horas")
            } else if let Some((a, b)) = hour.range {
                format!(", entre {a}h00 e {b}h59")
            } else if hour.values.len() == 1 {
                let h = hour.values[0];
                format!(", entre {h}h00 e {h}h59")
            } else {
                let v: Vec<String> = hour.values.iter().map(|h| format!("{h}h")).collect();
                let refs: Vec<&str> = v.iter().map(String::as_str).collect();
                format!(", nas horas {}", join_pt(&refs))
            };
            format!("{minutes}{hours}")
        }
    }
}

fn weekday_phrase(p: &Parsed) -> String {
    if let Some((a, b)) = p.range {
        let (a, b) = (a % 7, b % 7);
        if (a, b) == (1, 5) {
            return "de segunda a sexta-feira".into();
        }
        return format!(
            "de {} a {}",
            WEEKDAYS[usize::from(a)],
            WEEKDAYS[usize::from(b)]
        );
    }
    if p.values == [0, 6] {
        return "aos sábados e domingos".into();
    }
    let names: Vec<String> = p
        .values
        .iter()
        .map(|&d| {
            let name = WEEKDAYS[usize::from(d)];
            let plural = if d == 0 || d == 6 {
                format!("{name}s")
            } else {
                name.replace("-feira", "s-feiras")
            };
            let article = if d == 0 || d == 6 { "aos" } else { "às" };
            format!("{article} {plural}")
        })
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    join_pt(&refs)
}

fn month_phrase(p: &Parsed) -> String {
    if let Some((a, b)) = p.range {
        return format!(
            "de {} a {}",
            MONTHS[usize::from(a - 1)],
            MONTHS[usize::from(b - 1)]
        );
    }
    if let Some(s) = p.step {
        return format!("a cada {s} meses");
    }
    let names: Vec<&str> = p
        .values
        .iter()
        .map(|&m| MONTHS[usize::from(m - 1)])
        .collect();
    join_pt(&names)
}

fn day_phrase(day: &Parsed, weekday: &Parsed, month: &Parsed) -> String {
    let month_text = if month.any {
        None
    } else {
        Some(month_phrase(month))
    };
    let days = || {
        if let Some(s) = day.step {
            return format!("a cada {s} dias");
        }
        if let Some((a, b)) = day.range {
            return format!("do dia {a} ao dia {b}");
        }
        let v: Vec<String> = day.values.iter().map(u8::to_string).collect();
        let refs: Vec<&str> = v.iter().map(String::as_str).collect();
        if day.values.len() == 1 {
            format!("no dia {}", refs[0])
        } else {
            format!("nos dias {}", join_pt(&refs))
        }
    };
    match (day.any, weekday.any) {
        (true, true) => match month_text {
            Some(m) => format!("todos os dias, {}", in_month(&m)),
            None => "todos os dias".into(),
        },
        (true, false) => {
            let w = weekday_phrase(weekday);
            match month_text {
                Some(m) => format!("{w}, {}", in_month(&m)),
                None => w,
            }
        }
        (false, true) => match month_text {
            Some(m) if month.values.len() == 1 => format!("{} de {m}", days()),
            Some(m) => format!("{}, {}", days(), in_month(&m)),
            None => format!("{} de cada mês", days()),
        },
        (false, false) => {
            let base = format!("{} ou {}", days(), weekday_phrase(weekday));
            match month_text {
                Some(m) => format!("{base}, {}", in_month(&m)),
                None => base,
            }
        }
    }
}

fn in_month(m: &str) -> String {
    if m.starts_with("de ") || m.starts_with("a cada") {
        m.to_string()
    } else {
        format!("em {m}")
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(line: &str) -> String {
        parse(line).unwrap().summary
    }

    #[test]
    fn sentences() {
        assert_eq!(summary("*/5 * * * *"), "A cada 5 minutos, todos os dias.");
        assert_eq!(summary("0 3 * * 1"), "Às 03:00, às segundas-feiras.");
        assert_eq!(summary("30 8 1 * *"), "Às 08:30, no dia 1 de cada mês.");
        assert_eq!(summary("0 0 1 1 *"), "Às 00:00, no dia 1 de janeiro.");
        assert_eq!(
            summary("0 9-17 * * 1-5"),
            "No minuto 0 de cada hora, das 9h às 17h, de segunda a sexta-feira."
        );
        assert_eq!(summary("* * * * *"), "A cada minuto, todos os dias.");
        assert_eq!(
            summary("0 8,18 * * sat,sun"),
            "Às 08:00 e 18:00, aos sábados e domingos."
        );
        assert_eq!(
            summary("*/15 9 * * *"),
            "A cada 15 minutos, entre 9h00 e 9h59, todos os dias."
        );
        assert_eq!(
            summary("0 */6 * * *"),
            "A cada 6 horas, no minuto 0, todos os dias."
        );
        assert_eq!(
            summary("0 12 * jan-mar 1,3,5"),
            "Às 12:00, às segundas-feiras, às quartas-feiras e às sextas-feiras, de janeiro a março."
        );
    }

    #[test]
    fn fields_and_command() {
        let s = parse("0 3 * * * /usr/local/bin/backup.sh --full").unwrap();
        assert_eq!(s.fields.len(), 5);
        assert_eq!(s.fields[1].description, "hora: 3h");
        assert_eq!(
            s.command.as_deref(),
            Some("/usr/local/bin/backup.sh --full")
        );
    }

    #[test]
    fn shortcuts() {
        let s = parse("@daily /opt/job.sh").unwrap();
        assert!(s.summary.starts_with("Uma vez por dia"));
        assert_eq!(s.fields.len(), 5);
        assert_eq!(s.command.as_deref(), Some("/opt/job.sh"));
        assert!(parse("@reboot").unwrap().fields.is_empty());
    }

    #[test]
    fn notes_and_errors() {
        let s = parse("0 0 1 * 1 date +%F").unwrap();
        assert!(s.notes.iter().any(|n| n.contains("OU")));
        assert!(s.notes.iter().any(|n| n.contains("\\%")));
        assert!(parse("*/7 * * * *").unwrap().notes[0].contains("irregular"));
        assert!(parse("61 * * * *").is_err());
        assert!(parse("* * *").is_err());
    }

    #[test]
    fn detection() {
        assert!(looks_like_cron("*/5 * * * * echo oi"));
        assert!(looks_like_cron("@hourly"));
        assert!(!looks_like_cron("ls * * * *"));
        assert!(
            !looks_like_cron("0 3 1 2 5"),
            "sem * é só uma lista de números"
        );
        assert!(!looks_like_cron("grep -r x"));
        assert!(!looks_like_cron("@usuario"));
    }
}
