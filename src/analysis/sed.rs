//! sed scripts: `s/foo/bar/g`, `/^#/d`, `1,5p` → what each piece does.
//!
//! Regex parts are returned raw in [`Part::regex`]; the caller interprets
//! them with the regex analyzer, keeping this module independent of it.

/// One explained piece of a sed script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub label: &'static str,
    pub description: String,
    /// A regular expression to interpret (addresses and `s` patterns).
    pub regex: Option<String>,
}

fn part(text: impl Into<String>, label: &'static str, description: impl Into<String>) -> Part {
    Part {
        text: text.into(),
        label,
        description: description.into(),
        regex: None,
    }
}

/// Explains every command of `script`. Never fails: unknown pieces are
/// reported as such.
pub fn explain(script: &str) -> Vec<Part> {
    let chars: Vec<char> = script.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == ';' {
            i += 1;
            continue;
        }
        if c == '#' {
            let end = chars[i..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(chars.len(), |p| i + p);
            out.push(part(
                collect(&chars[i..end]),
                "comentário",
                "Ignorado pelo sed",
            ));
            i = end;
            continue;
        }
        if c == '}' {
            out.push(part("}", "bloco", "Fim do bloco de comandos"));
            i += 1;
            continue;
        }
        let start = i;
        let addr = address(&chars, &mut i);
        if let Some(a) = &addr {
            let range = if chars.get(i) == Some(&',') {
                i += 1;
                address(&chars, &mut i)
            } else {
                None
            };
            out.extend(describe_address(a, range.as_ref(), &chars[start..i]));
        }
        while chars.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        if chars.get(i) == Some(&'!') {
            out.push(part(
                "!",
                "negação",
                "Aplica o comando às linhas que NÃO casam com o endereço",
            ));
            i += 1;
            while chars.get(i).is_some_and(|c| *c == ' ') {
                i += 1;
            }
        }
        let Some(&cmd) = chars.get(i) else {
            if addr.is_some() {
                out.push(part(
                    "",
                    "?",
                    "Falta o comando depois do endereço (ex.: p, d, s///)",
                ));
            }
            break;
        };
        i += 1;
        command(cmd, &chars, &mut i, &mut out);
    }
    out
}

#[derive(Debug, Clone)]
enum Address {
    Line(String),
    Last,
    Regex(String),
    Step(String, String),
    Relative(String),
}

fn address(chars: &[char], i: &mut usize) -> Option<Address> {
    let c = *chars.get(*i)?;
    if c.is_ascii_digit() {
        let n = take_while(chars, i, |c| c.is_ascii_digit());
        if chars.get(*i) == Some(&'~') {
            *i += 1;
            let step = take_while(chars, i, |c| c.is_ascii_digit());
            return Some(Address::Step(n, step));
        }
        return Some(Address::Line(n));
    }
    if c == '+' {
        *i += 1;
        return Some(Address::Relative(take_while(chars, i, |c| {
            c.is_ascii_digit()
        })));
    }
    if c == '$' {
        *i += 1;
        return Some(Address::Last);
    }
    if c == '/' || c == '\\' {
        let delim = if c == '\\' {
            *i += 1;
            *chars.get(*i)?
        } else {
            '/'
        };
        *i += 1;
        let re = until(chars, i, delim);
        // `I` flag: case-insensitive address (GNU).
        if chars.get(*i) == Some(&'I') {
            *i += 1;
        }
        return Some(Address::Regex(re));
    }
    None
}

fn describe_address(a: &Address, b: Option<&Address>, text: &[char]) -> Vec<Part> {
    let one = |a: &Address| match a {
        Address::Line(n) => format!("a linha {n}"),
        Address::Last => "a última linha".into(),
        Address::Regex(re) => format!("a próxima linha que casa com /{re}/"),
        Address::Step(n, s) => format!("a cada {s} linhas a partir da {n}"),
        Address::Relative(n) => format!("mais {n} linhas"),
    };
    let description = match (a, b) {
        (Address::Line(n), None) => format!("Só a linha {n}"),
        (Address::Last, None) => "Só a última linha".into(),
        (Address::Regex(re), None) => format!("Linhas que casam com a regex {re}"),
        (Address::Step(n, s), None) => format!("A cada {s} linhas, começando na linha {n}"),
        (Address::Relative(_), None) => "Endereço relativo sem início".into(),
        (a, Some(b)) => {
            let from = match a {
                Address::Line(n) => format!("Da linha {n}"),
                Address::Regex(re) => format!("Da primeira linha que casa com /{re}/"),
                _ => format!("De {}", one(a)),
            };
            format!("{from} até {} (intervalo)", one(b))
        }
    };
    let mut p = part(collect(text), "endereço", description);
    p.regex = match (a, b) {
        (Address::Regex(re), None) if !re.is_empty() => Some(re.clone()),
        _ => None,
    };
    vec![p]
}

fn command(cmd: char, chars: &[char], i: &mut usize, out: &mut Vec<Part>) {
    let simple = |d: &str| part(cmd.to_string(), "comando", d);
    match cmd {
        's' => substitute(chars, i, out),
        'y' => {
            let Some(&delim) = chars.get(*i) else {
                out.push(simple("Transliteração incompleta"));
                return;
            };
            *i += 1;
            let from = until(chars, i, delim);
            let to = until(chars, i, delim);
            out.push(part(
                format!("y{delim}{from}{delim}{to}{delim}"),
                "comando",
                format!("Troca cada caractere de \"{from}\" pelo da mesma posição em \"{to}\" (como o tr)"),
            ));
        }
        'd' => out.push(simple("Apaga a linha (não imprime) e passa para a próxima")),
        'D' => out.push(simple("Apaga até a primeira quebra de linha do espaço de padrão")),
        'p' => out.push(simple("Imprime a linha (com -n, só as escolhidas aparecem; sem -n, ela sai duplicada)")),
        'P' => out.push(simple("Imprime até a primeira quebra de linha do espaço de padrão")),
        'n' => out.push(simple("Imprime a linha atual e lê a próxima")),
        'N' => out.push(simple("Junta a próxima linha à atual (separadas por \\n)")),
        '=' => out.push(simple("Imprime o número da linha")),
        'l' => out.push(simple("Mostra a linha com caracteres invisíveis escapados")),
        'h' => out.push(simple("Copia a linha para o espaço reserva (hold space)")),
        'H' => out.push(simple("Acrescenta a linha ao espaço reserva")),
        'g' => out.push(simple("Substitui a linha pelo conteúdo do espaço reserva")),
        'G' => out.push(simple("Acrescenta o espaço reserva à linha (ex.: G sozinho põe linha em branco entre as linhas)")),
        'x' => out.push(simple("Troca a linha com o espaço reserva")),
        'z' => out.push(simple("Esvazia a linha")),
        '{' => out.push(part("{", "bloco", "Início de um bloco: os comandos até } valem para o endereço")),
        'q' | 'Q' => {
            let code = take_while(chars, i, |c| c.is_ascii_digit());
            let what = if cmd == 'q' {
                "Imprime a linha atual e encerra"
            } else {
                "Encerra sem imprimir"
            };
            let desc = if code.is_empty() {
                what.to_string()
            } else {
                format!("{what} com exit code {code}")
            };
            out.push(part(format!("{cmd}{code}"), "comando", desc));
        }
        'a' | 'i' | 'c' => {
            let text = text_argument(chars, i);
            let desc = match cmd {
                'a' => "Acrescenta o texto depois da linha",
                'i' => "Insere o texto antes da linha",
                _ => "Troca a linha pelo texto",
            };
            out.push(part(format!("{cmd} {text}").trim_end().to_string(), "comando", desc));
        }
        'r' | 'R' | 'w' | 'W' => {
            let file = text_argument(chars, i);
            let desc = match cmd {
                'r' => format!("Insere o conteúdo do arquivo {file} depois da linha"),
                'R' => format!("Insere uma linha do arquivo {file}"),
                'w' => format!("Grava a linha no arquivo {file}"),
                _ => format!("Grava a primeira linha do espaço de padrão em {file}"),
            };
            out.push(part(format!("{cmd} {file}"), "comando", desc));
        }
        ':' => {
            let label = label_argument(chars, i);
            out.push(part(format!(":{label}"), "rótulo", "Marca um ponto para b e t pularem"));
        }
        'b' | 't' | 'T' => {
            let label = label_argument(chars, i);
            let target = if label.is_empty() {
                "o fim do script".to_string()
            } else {
                format!("o rótulo {label}")
            };
            let desc = match cmd {
                'b' => format!("Pula para {target}"),
                't' => format!("Pula para {target} se houve substituição desde a última linha lida"),
                _ => format!("Pula para {target} se NÃO houve substituição"),
            };
            out.push(part(format!("{cmd}{label}"), "comando", desc));
        }
        'e' => out.push(simple("Executa o comando e insere a saída: cuidado")),
        other => out.push(part(other.to_string(), "?", "Comando sed não reconhecido")),
    }
}

fn substitute(chars: &[char], i: &mut usize, out: &mut Vec<Part>) {
    let Some(&delim) = chars.get(*i) else {
        out.push(part(
            "s",
            "comando",
            "Substituição incompleta: s/PADRÃO/TROCA/FLAGS",
        ));
        return;
    };
    *i += 1;
    let pattern = until(chars, i, delim);
    let replacement = until(chars, i, delim);
    let flags = take_while(chars, i, |c| c.is_ascii_alphanumeric());
    let global = flags.contains('g');
    let mut desc = "Substitui o trecho que casa com o padrão".to_string();
    if !global && !flags.chars().any(|c| c.is_ascii_digit()) {
        desc.push_str(" (sem g, só a primeira ocorrência de cada linha)");
    }
    if delim != '/' {
        desc.push_str(&format!(
            "; usa {delim} como delimitador, útil quando o texto tem /"
        ));
    }
    out.push(part("s", "comando", desc));
    let mut p = part(pattern.clone(), "padrão", "Regex procurada");
    if pattern.is_empty() {
        p.description = "Vazio: reutiliza a última regex usada".into();
    } else {
        p.regex = Some(pattern);
    }
    out.push(p);
    out.push(part(
        if replacement.is_empty() {
            "∅".to_string()
        } else {
            replacement.clone()
        },
        "troca",
        replacement_text(&replacement),
    ));
    for f in flags.chars() {
        let d = match f {
            'g' => "Todas as ocorrências da linha".to_string(),
            'i' | 'I' => "Ignora maiúsculas/minúsculas".to_string(),
            'p' => "Imprime a linha se houve troca (use com -n)".to_string(),
            'm' | 'M' => "Modo multilinha: ^ e $ casam em cada linha".to_string(),
            'e' => "Executa o resultado como comando: cuidado".to_string(),
            'w' => "Grava as linhas alteradas no arquivo a seguir".to_string(),
            d if d.is_ascii_digit() => format!("Só a {d}ª ocorrência de cada linha"),
            _ => "Flag não reconhecida".to_string(),
        };
        out.push(part(f.to_string(), "flag", d));
    }
}

fn replacement_text(r: &str) -> String {
    if r.is_empty() {
        return "Vazio: o trecho encontrado é apagado".into();
    }
    let mut notes = Vec::new();
    let mut prev_backslash = false;
    for c in r.chars() {
        if prev_backslash {
            prev_backslash = false;
            match c {
                '1'..='9' => notes.push(format!("\\{c} = o que o grupo {c} capturou")),
                'n' => notes.push("\\n = quebra de linha".into()),
                't' => notes.push("\\t = tabulação".into()),
                'U' | 'L' => notes.push(format!(
                    "\\{c} = converte o resto para {}",
                    if c == 'U' {
                        "MAIÚSCULAS"
                    } else {
                        "minúsculas"
                    }
                )),
                _ => {}
            }
            continue;
        }
        match c {
            '\\' => prev_backslash = true,
            '&' => notes.push("& = o trecho inteiro que casou".into()),
            _ => {}
        }
    }
    notes.dedup();
    if notes.is_empty() {
        "Texto que entra no lugar".into()
    } else {
        format!("Texto que entra no lugar; {}", notes.join(", "))
    }
}

fn until(chars: &[char], i: &mut usize, delim: char) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.get(*i) {
        *i += 1;
        if c == '\\' {
            if let Some(&next) = chars.get(*i) {
                *i += 1;
                if next != delim {
                    out.push('\\');
                }
                out.push(next);
            }
            continue;
        }
        if c == delim {
            break;
        }
        out.push(c);
    }
    out
}

fn take_while(chars: &[char], i: &mut usize, f: impl Fn(char) -> bool) -> String {
    let start = *i;
    while chars.get(*i).is_some_and(|c| f(*c)) {
        *i += 1;
    }
    collect(&chars[start..*i])
}

/// Text after a/i/c/r/w: GNU one-liner form (`a texto`) or `a\` + text.
fn text_argument(chars: &[char], i: &mut usize) -> String {
    if chars.get(*i) == Some(&'\\') {
        *i += 1;
    }
    let start = *i;
    while chars.get(*i).is_some_and(|c| *c != '\n') {
        *i += 1;
    }
    collect(&chars[start..*i]).trim().to_string()
}

fn label_argument(chars: &[char], i: &mut usize) -> String {
    while chars.get(*i) == Some(&' ') {
        *i += 1;
    }
    take_while(chars, i, |c| {
        !matches!(c, ';' | '\n' | '}') && !c.is_whitespace()
    })
}

fn collect(chars: &[char]) -> String {
    chars.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(script: &str) -> Vec<(String, &'static str)> {
        explain(script)
            .into_iter()
            .map(|p| (p.text, p.label))
            .collect()
    }

    #[test]
    fn substitution() {
        let parts = explain("s/foo/bar/g");
        assert_eq!(parts[0].label, "comando");
        assert!(!parts[0].description.contains("primeira"));
        assert_eq!(parts[1].regex.as_deref(), Some("foo"));
        assert_eq!(parts[2].text, "bar");
        assert_eq!(parts[3].description, "Todas as ocorrências da linha");

        let parts = explain("s/foo/bar/");
        assert!(parts[0].description.contains("primeira ocorrência"));
    }

    #[test]
    fn replacement_specials() {
        let parts = explain(r"s/\(ab\)c/[&] \1/");
        assert!(parts[2].description.contains("& = o trecho inteiro"));
        assert!(
            parts[2]
                .description
                .contains("\\1 = o que o grupo 1 capturou")
        );
        assert!(explain("s/x//")[2].description.contains("apagado"));
    }

    #[test]
    fn custom_delimiter() {
        let parts = explain("s|/usr/local|/opt|g");
        assert_eq!(parts[1].regex.as_deref(), Some("/usr/local"));
        assert_eq!(parts[2].text, "/opt");
        assert!(parts[0].description.contains("delimitador"));
    }

    #[test]
    fn addresses() {
        assert_eq!(
            labels("/^#/d"),
            [("/^#/".into(), "endereço"), ("d".into(), "comando")]
        );
        assert_eq!(explain("/^#/d")[0].regex.as_deref(), Some("^#"));
        let p = explain("10,20p");
        assert!(p[0].description.starts_with("Da linha 10 até a linha 20"));
        assert!(explain("$d")[0].description.contains("última"));
        assert!(explain("/^$/!d")[1].label == "negação");
    }

    #[test]
    fn several_commands() {
        let l = labels("1d; $d; s/a/b/");
        let commands: Vec<&str> = l
            .iter()
            .filter(|(_, k)| *k == "comando")
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(commands, ["d", "d", "s"]);
        assert!(explain("2i\\Olá")[0].description.contains("linha 2"));
        assert_eq!(explain("2i\\Olá")[1].text, "i Olá");
    }
}
