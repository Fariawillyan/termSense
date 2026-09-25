//! awk programs: `NR>1 {sum+=$2} END {print sum}` → the rules (pattern +
//! action) and a glossary of the fields, variables and functions used.
//! A light scanner, not a full awk parser: good enough for one-liners.

/// One explained piece of an awk program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub label: &'static str,
    pub description: String,
}

fn part(text: impl Into<String>, label: &'static str, description: impl Into<String>) -> Part {
    Part {
        text: text.into(),
        label,
        description: description.into(),
    }
}

/// Rules first (pattern and action), then the glossary.
pub fn explain(program: &str) -> Vec<Part> {
    let mut out = Vec::new();
    for (pattern, action) in rules(program) {
        let pattern = pattern.trim();
        let when = match pattern {
            "" => None,
            "BEGIN" => Some(part(
                "BEGIN",
                "padrão",
                "Roda uma vez, antes de ler a entrada",
            )),
            "END" => Some(part(
                "END",
                "padrão",
                "Roda uma vez, depois de ler toda a entrada",
            )),
            "BEGINFILE" | "ENDFILE" => Some(part(
                pattern,
                "padrão",
                "Roda no início ou no fim de cada arquivo (gawk)",
            )),
            p if is_regex_literal(p) => Some(part(
                p,
                "padrão",
                format!("Só as linhas que casam com a regex {}", &p[1..p.len() - 1]),
            )),
            p if p.contains(',') && !p.contains('(') => Some(part(
                p,
                "padrão",
                "Intervalo: da linha que satisfaz o primeiro padrão até a que satisfaz o segundo",
            )),
            p => Some(part(
                p,
                "padrão",
                "Condição: a ação só roda nas linhas em que ela é verdadeira",
            )),
        };
        out.extend(when);
        match action {
            Some(a) => {
                let scope = match pattern {
                    "BEGIN" | "END" | "BEGINFILE" | "ENDFILE" => "Ação",
                    "" => "Ação executada para cada linha",
                    _ => "Ação executada nas linhas selecionadas",
                };
                out.push(part(a, "ação", scope));
            }
            None if !pattern.is_empty() => out.push(part(
                "",
                "ação",
                "Sem ação: imprime a linha inteira ($0) quando o padrão casa",
            )),
            None => {}
        }
    }
    out.extend(glossary(program));
    out
}

fn is_regex_literal(p: &str) -> bool {
    p.len() >= 2 && p.starts_with('/') && p.ends_with('/')
}

/// Splits a program into `(pattern, action)` rules at the top level.
fn rules(program: &str) -> Vec<(String, Option<String>)> {
    let chars: Vec<char> = program.chars().collect();
    let mut out = Vec::new();
    let mut pattern = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let end = skip_string(&chars, i);
                pattern.extend(&chars[i..end]);
                i = end;
                continue;
            }
            '/' if pattern.trim().is_empty()
                || pattern.trim_end().ends_with(['~', '(', ',', '!', '&', '|']) =>
            {
                let end = skip_regex(&chars, i);
                pattern.extend(&chars[i..end]);
                i = end;
                continue;
            }
            '{' => {
                let end = skip_block(&chars, i);
                let action: String = chars[i..end].iter().collect();
                out.push((std::mem::take(&mut pattern), Some(action)));
                i = end;
                continue;
            }
            '\n' | ';' => {
                if !pattern.trim().is_empty() {
                    out.push((std::mem::take(&mut pattern), None));
                }
                pattern.clear();
            }
            _ => pattern.push(c),
        }
        i += 1;
    }
    if !pattern.trim().is_empty() {
        out.push((pattern, None));
    }
    out
}

fn skip_string(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

fn skip_regex(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '/' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

fn skip_block(chars: &[char], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                i = skip_string(chars, i);
                continue;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

const VARIABLES: &[(&str, &str)] = &[
    (
        "NR",
        "número da linha (registro) atual, contando todos os arquivos",
    ),
    ("FNR", "número da linha dentro do arquivo atual"),
    ("NF", "número de campos da linha"),
    (
        "FS",
        "separador de campos da entrada (padrão: espaços; mude com -F)",
    ),
    (
        "OFS",
        "separador de campos da saída, usado pelo print com vírgula",
    ),
    ("RS", "separador de registros (padrão: quebra de linha)"),
    ("ORS", "separador de registros na saída"),
    ("FILENAME", "nome do arquivo sendo lido"),
    ("ENVIRON", "array com as variáveis de ambiente"),
    ("ARGV", "array com os argumentos da linha de comando"),
    ("SUBSEP", "separador de índices compostos em arrays"),
];

const FUNCTIONS: &[(&str, &str)] = &[
    (
        "print",
        "imprime os valores; separados por vírgula saem com OFS entre eles",
    ),
    (
        "printf",
        "imprime com formato (%s texto, %d inteiro, %.2f decimal); não quebra linha sozinho",
    ),
    ("sprintf", "formata como o printf, mas devolve o texto"),
    ("length", "tamanho do texto (ou do array)"),
    (
        "substr",
        "substr(s, início, n): pedaço do texto (começa em 1)",
    ),
    ("index", "index(s, t): posição de t em s (0 se não achar)"),
    (
        "split",
        "split(s, arr, sep): divide o texto em um array e devolve quantos pedaços",
    ),
    (
        "sub",
        "sub(regex, troca[, alvo]): troca a primeira ocorrência",
    ),
    (
        "gsub",
        "gsub(regex, troca[, alvo]): troca todas as ocorrências",
    ),
    (
        "match",
        "match(s, regex): posição do casamento (RSTART, RLENGTH)",
    ),
    ("tolower", "converte para minúsculas"),
    ("toupper", "converte para maiúsculas"),
    ("int", "parte inteira de um número"),
    ("sqrt", "raiz quadrada"),
    ("system", "executa um comando do shell: cuidado"),
    ("getline", "lê a próxima linha"),
    (
        "next",
        "pula para a próxima linha sem rodar o resto das regras",
    ),
    ("exit", "termina (ainda roda o bloco END)"),
    ("delete", "remove um elemento do array"),
    ("strftime", "formata data e hora (gawk)"),
];

/// Fields, variables, functions and operators in order of appearance.
fn glossary(program: &str) -> Vec<Part> {
    let chars: Vec<char> = program.chars().collect();
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut add = |text: String, label: &'static str, desc: String, out: &mut Vec<Part>| {
        if !seen.contains(&text) {
            seen.push(text.clone());
            out.push(part(text, label, desc));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            i = skip_string(&chars, i);
            continue;
        }
        if c == '$' {
            let start = i;
            i += 1;
            let word: String = if chars.get(i) == Some(&'(') {
                let end = chars[i..]
                    .iter()
                    .position(|&c| c == ')')
                    .map_or(chars.len(), |p| i + p + 1);
                let w: String = chars[i..end].iter().collect();
                i = end;
                w
            } else {
                let end = chars[i..]
                    .iter()
                    .position(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
                    .map_or(chars.len(), |p| i + p);
                let w: String = chars[i..end].iter().collect();
                i = end;
                w
            };
            let text: String = chars[start..i].iter().collect();
            let desc = match word.as_str() {
                "0" => "a linha inteira".to_string(),
                "NF" => "o último campo da linha".to_string(),
                "(NF-1)" => "o penúltimo campo".to_string(),
                n if n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() => {
                    format!("o {n}º campo da linha")
                }
                _ => "o campo de número calculado pela expressão".to_string(),
            };
            add(text, "campo", desc, &mut out);
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let end = chars[i..]
                .iter()
                .position(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map_or(chars.len(), |p| i + p);
            let word: String = chars[i..end].iter().collect();
            let next = chars[end..].iter().find(|c| !c.is_whitespace()).copied();
            i = end;
            let keyword = matches!(
                word.as_str(),
                "BEGIN"
                    | "END"
                    | "BEGINFILE"
                    | "ENDFILE"
                    | "if"
                    | "else"
                    | "for"
                    | "while"
                    | "in"
                    | "do"
            );
            if keyword {
                continue;
            }
            if let Some((_, d)) = VARIABLES.iter().find(|(v, _)| *v == word) {
                add(word, "variável", (*d).to_string(), &mut out);
            } else if let Some((_, d)) = FUNCTIONS.iter().find(|(f, _)| *f == word) {
                add(word, "função", (*d).to_string(), &mut out);
            } else if next == Some('[') {
                add(
                    word,
                    "array",
                    "array associativo: índices podem ser textos (ex.: contar[$1]++ conta por valor)".into(),
                    &mut out,
                );
            } else {
                add(
                    word,
                    "variável",
                    "variável do programa: começa vazia (0 em contas) e dura até o fim".into(),
                    &mut out,
                );
            }
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op = match two.as_str() {
            "+=" | "-=" | "*=" | "/=" => Some((two.clone(), "acumula: x += y é x = x + y")),
            "++" => Some((two.clone(), "soma 1 (contador)")),
            "!~" => Some((two.clone(), "não casa com a regex")),
            "==" => Some((two.clone(), "igual a")),
            "!=" => Some((two.clone(), "diferente de")),
            "&&" => Some((two.clone(), "E lógico")),
            "||" => Some((two.clone(), "OU lógico")),
            _ if c == '~' => Some(("~".to_string(), "casa com a regex")),
            _ => None,
        };
        if let Some((text, desc)) = op {
            i += text.chars().count();
            add(text, "operador", desc.to_string(), &mut out);
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(program: &str) -> Vec<(String, &'static str)> {
        explain(program)
            .into_iter()
            .map(|p| (p.text, p.label))
            .collect()
    }

    #[test]
    fn print_fields() {
        let t = texts("{print $1, $3}");
        assert_eq!(t[0], ("{print $1, $3}".into(), "ação"));
        assert!(t.contains(&("$1".into(), "campo")));
        assert!(t.contains(&("$3".into(), "campo")));
        assert!(t.contains(&("print".into(), "função")));
    }

    #[test]
    fn begin_end_and_condition() {
        let parts = explain("NR>1 {sum+=$2} END {print sum}");
        let labels: Vec<(&str, &str)> = parts.iter().map(|p| (p.text.as_str(), p.label)).collect();
        assert_eq!(labels[0], ("NR>1", "padrão"));
        assert_eq!(labels[1], ("{sum+=$2}", "ação"));
        assert_eq!(labels[2], ("END", "padrão"));
        assert_eq!(labels[3], ("{print sum}", "ação"));
        assert!(
            parts
                .iter()
                .any(|p| p.text == "NR" && p.description.contains("número da linha"))
        );
        assert!(
            parts
                .iter()
                .any(|p| p.text == "sum" && p.label == "variável")
        );
        assert!(parts.iter().any(|p| p.text == "+="));
    }

    #[test]
    fn regex_pattern_without_action() {
        let parts = explain("/ERROR/");
        assert!(parts[0].description.contains("regex ERROR"));
        assert!(parts[1].description.contains("imprime a linha inteira"));
    }

    #[test]
    fn arrays_and_last_field() {
        let parts = explain("{c[$1]++} END {for (k in c) print k, c[k]}");
        assert!(parts.iter().any(|p| p.text == "c" && p.label == "array"));
        assert!(parts.iter().any(|p| p.text == "++"));
        let parts = explain("{print $NF}");
        assert!(
            parts
                .iter()
                .any(|p| p.text == "$NF" && p.description.contains("último"))
        );
    }
}
