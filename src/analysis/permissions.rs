//! Unix permissions: octal ↔ symbolic (`755` ↔ `rwxr-xr-x`), what each class
//! may do, special bits, umask and chmod's symbolic clauses (`u+x,go-w`).
//! Pure computation; nothing touches the file system.

/// Permission bits, special bits included (`0..=0o7777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode(pub u16);

pub const SETUID: u16 = 0o4000;
pub const SETGID: u16 = 0o2000;
pub const STICKY: u16 = 0o1000;

impl Mode {
    /// `755`, `0644`, `4755`, `00644`: three to five octal digits.
    pub fn parse_octal(s: &str) -> Option<Mode> {
        let digits = s.len();
        if !(3..=5).contains(&digits) || !s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return None;
        }
        if digits == 5 && !s.starts_with('0') {
            return None;
        }
        u16::from_str_radix(s, 8)
            .ok()
            .filter(|v| *v <= 0o7777)
            .map(Mode)
    }

    /// `rwxr-xr-x`, or the 10-column form of `ls -l` (`-rw-r--r--`,
    /// `drwxrwxrwt`). Returns the file-type character of the 10-column form.
    pub fn parse_symbolic(s: &str) -> Option<(Option<char>, Mode)> {
        let chars: Vec<char> = s.chars().collect();
        let (kind, perms) = match chars.len() {
            9 => (None, &chars[..]),
            10 if "-dlcbps".contains(chars[0]) => (Some(chars[0]), &chars[1..]),
            _ => return None,
        };
        let mut bits = 0u16;
        for (class, triple) in perms.chunks(3).enumerate() {
            let shift = 6 - 3 * class as u16;
            match triple[0] {
                'r' => bits |= 4 << shift,
                '-' => {}
                _ => return None,
            }
            match triple[1] {
                'w' => bits |= 2 << shift,
                '-' => {}
                _ => return None,
            }
            let special = [SETUID, SETGID, STICKY][class];
            let special_char = if class == 2 { 't' } else { 's' };
            match triple[2] {
                'x' => bits |= 1 << shift,
                '-' => {}
                c if c == special_char => bits |= (1 << shift) | special,
                c if c == special_char.to_ascii_uppercase() => bits |= special,
                _ => return None,
            }
        }
        Some((kind, Mode(bits)))
    }

    /// `755`, or `4755` when a special bit is set.
    pub fn octal(self) -> String {
        if self.0 & 0o7000 != 0 {
            format!("{:04o}", self.0)
        } else {
            format!("{:03o}", self.0)
        }
    }

    /// `rwxr-xr-x`, with `s`/`S`/`t`/`T` for the special bits.
    pub fn symbolic(self) -> String {
        let mut out = String::with_capacity(9);
        for class in 0..3 {
            let digit = self.digit(class);
            out.push(if digit & 4 != 0 { 'r' } else { '-' });
            out.push(if digit & 2 != 0 { 'w' } else { '-' });
            let exec = digit & 1 != 0;
            let special = self.0 & [SETUID, SETGID, STICKY][class] != 0;
            let mark = if class == 2 { 't' } else { 's' };
            out.push(match (exec, special) {
                (true, true) => mark,
                (false, true) => mark.to_ascii_uppercase(),
                (true, false) => 'x',
                (false, false) => '-',
            });
        }
        out
    }

    /// Octal digit of a class: 0 = owner, 1 = group, 2 = others.
    pub fn digit(self, class: usize) -> u8 {
        ((self.0 >> (6 - 3 * class)) & 0o7) as u8
    }
}

pub const CLASSES: [&str; 3] = ["dono", "grupo", "outros"];

/// `rwx` → "ler, escrever e executar"; `0` → "nenhuma permissão".
pub fn digit_meaning(digit: u8) -> String {
    let mut parts = Vec::new();
    if digit & 4 != 0 {
        parts.push("ler");
    }
    if digit & 2 != 0 {
        parts.push("escrever");
    }
    if digit & 1 != 0 {
        parts.push("executar");
    }
    if parts.is_empty() {
        "nenhuma permissão".into()
    } else {
        join_pt(&parts)
    }
}

/// The same digit applied to a directory: r lists, x enters, w (with x)
/// creates and deletes entries.
pub fn digit_meaning_dir(digit: u8) -> String {
    let (r, w, x) = (digit & 4 != 0, digit & 2 != 0, digit & 1 != 0);
    let mut parts = Vec::new();
    if r {
        parts.push("listar");
    }
    if w && x {
        parts.push("criar/apagar arquivos");
    }
    if x {
        parts.push(if r {
            "entrar (cd)"
        } else {
            "entrar (cd) sem poder listar"
        });
    }
    let mut text = if parts.is_empty() {
        "nenhum acesso".to_string()
    } else {
        join_pt(&parts)
    };
    if w && !x {
        text.push_str(" (w sem x não tem efeito)");
    }
    text
}

/// What the special bits of `mode` do.
pub fn special_bits(mode: Mode) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    if mode.0 & SETUID != 0 {
        out.push((
            "setuid (4000)",
            "o programa executa com os privilégios do dono do arquivo (ex.: /usr/bin/passwd)",
        ));
    }
    if mode.0 & SETGID != 0 {
        out.push((
            "setgid (2000)",
            "em programas: executa com o grupo do arquivo; em diretórios: arquivos novos herdam o grupo do diretório",
        ));
    }
    if mode.0 & STICKY != 0 {
        out.push((
            "sticky (1000)",
            "em diretórios: só o dono de um arquivo pode apagá-lo ou renomeá-lo (ex.: /tmp)",
        ));
    }
    out
}

/// Where a mode is typically used.
pub fn typical_use(mode: Mode) -> Option<&'static str> {
    Some(match mode.0 {
        0o644 => "arquivos comuns: o dono edita, todos leem",
        0o755 => "executáveis, scripts e diretórios: todos leem e executam, só o dono altera",
        0o600 => "arquivos privados, como chaves SSH (~/.ssh/id_*) e arquivos com senhas",
        0o700 => "diretórios e scripts privados, como ~/.ssh",
        0o400 => "somente leitura para o dono (ex.: chave .pem de nuvem)",
        0o440 => "somente leitura para dono e grupo (ex.: /etc/sudoers)",
        0o640 => "configurações legíveis por um grupo de serviço",
        0o750 => "diretórios e programas liberados só para um grupo",
        0o775 => "diretórios compartilhados por um grupo de trabalho",
        0o664 => "arquivos editados por um grupo de trabalho",
        0o777 => "todos podem tudo: quase sempre um erro de segurança",
        0o666 => "todos leem e escrevem: evite",
        0o1777 => "diretórios temporários compartilhados, como /tmp",
        0o4755 => "programas setuid, como passwd e sudo",
        0o2775 => "diretório de projeto em grupo: arquivos novos herdam o grupo",
        _ => return None,
    })
}

/// Security remarks: world-writable, setuid...
pub fn risks(mode: Mode) -> Vec<&'static str> {
    let mut out = Vec::new();
    if mode.0 & 0o777 == 0o777 {
        out.push("Qualquer usuário do sistema pode ler, alterar e executar. Prefira 755 (ou 775 para um grupo).");
    } else if mode.digit(2) & 2 != 0 {
        out.push("Outros usuários podem alterar este arquivo (escrita para \"outros\").");
    }
    if mode.0 & SETUID != 0 {
        out.push("setuid em programas próprios é um risco: uma falha no programa vira acesso com os privilégios do dono (muitas vezes root).");
    }
    out
}

/// Default permissions of new files and directories under `mask`.
pub fn umask_result(mask: Mode) -> (Mode, Mode) {
    let m = mask.0 & 0o777;
    (Mode(0o666 & !m), Mode(0o777 & !m))
}

/// One clause of chmod's symbolic syntax: `u+x`, `go-w`, `a=r`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub text: String,
    pub description: String,
}

/// Parses `u+x,go-w` into described clauses. `None` if it is not symbolic.
pub fn parse_symbolic_change(s: &str) -> Option<Vec<Clause>> {
    if s.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for clause in s.split(',') {
        let who_len = clause.chars().take_while(|c| "ugoa".contains(*c)).count();
        let (who, rest) = clause.split_at(who_len);
        if rest.is_empty() {
            return None;
        }
        let mut actions = Vec::new();
        let mut chars = rest.chars().peekable();
        while let Some(op) = chars.next() {
            if !"+-=".contains(op) {
                return None;
            }
            let mut perms = String::new();
            while let Some(&c) = chars.peek() {
                if "+-=".contains(c) {
                    break;
                }
                if !"rwxXstugo".contains(c) {
                    return None;
                }
                perms.push(c);
                chars.next();
            }
            actions.push(describe_action(op, &perms, who));
        }
        out.push(Clause {
            text: clause.to_string(),
            description: actions.join("; "),
        });
    }
    Some(out)
}

fn describe_action(op: char, perms: &str, who: &str) -> String {
    let verb = match op {
        '+' => "adiciona",
        '-' => "remove",
        _ => "define exatamente",
    };
    let names: Vec<&str> = perms
        .chars()
        .map(|c| match c {
            'r' => "leitura",
            'w' => "escrita",
            'x' => "execução",
            'X' => "execução só em diretórios (ou onde alguém já executa)",
            's' => "setuid/setgid",
            't' => "sticky bit",
            'u' => "as permissões do dono",
            'g' => "as permissões do grupo",
            _ => "as permissões dos outros",
        })
        .collect();
    let what = if names.is_empty() {
        "nenhuma permissão".to_string()
    } else {
        join_pt(&names)
    };
    let target = who_text(who, op);
    let mut text = format!("{verb} {what} {target}");
    if op == '=' {
        text.push_str(" (o que não foi citado é removido)");
    }
    if who.is_empty() {
        text.push_str(", respeitando a umask");
    }
    text
}

fn who_text(who: &str, op: char) -> String {
    let prep = if op == '-' { "de" } else { "para" };
    if who.is_empty()
        || who.contains('a')
        || (who.contains('u') && who.contains('g') && who.contains('o'))
    {
        return format!("{prep} todos");
    }
    let names: Vec<&str> = who
        .chars()
        .map(|c| match (c, op == '-') {
            ('u', false) => "o dono",
            ('u', true) => "do dono",
            ('g', false) => "o grupo",
            ('g', true) => "do grupo",
            (_, false) => "os outros",
            (_, true) => "dos outros",
        })
        .collect();
    if op == '-' {
        join_pt(&names)
    } else {
        format!("para {}", join_pt(&names))
    }
}

/// One-line meaning of a chmod/find mode argument: `755`, `u+x,go-w`,
/// `-644` (find: at least these bits), `/111` (find: any of these bits).
pub fn describe_mode_argument(value: &str) -> Option<String> {
    let (prefix, rest) = match value.chars().next() {
        Some('-') if value.len() > 1 && value.as_bytes()[1].is_ascii_digit() => {
            ("pelo menos estas permissões: ", &value[1..])
        }
        Some('/') => ("qualquer uma destas permissões: ", &value[1..]),
        _ => ("", value),
    };
    if let Some(mode) = Mode::parse_octal(rest) {
        let classes: Vec<String> = (0..3)
            .map(|c| format!("{} {}", CLASSES[c], digit_meaning(mode.digit(c))))
            .collect();
        let mut text = format!(
            "{prefix}{} = {}: {}",
            mode.octal(),
            mode.symbolic(),
            classes.join("; ")
        );
        for (bit, _) in special_bits(mode) {
            text.push_str(&format!("; {bit}"));
        }
        return Some(text);
    }
    let clauses = parse_symbolic_change(rest)?;
    let described: Vec<String> = clauses
        .iter()
        .map(|c| format!("{}: {}", c.text, c.description))
        .collect();
    Some(format!("{prefix}{}", described.join("; ")))
}

/// `a`, `a e b`, `a, b e c`.
pub fn join_pt(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [one] => (*one).to_string(),
        [init @ .., last] => format!("{} e {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_and_symbolic_round_trip() {
        let m = Mode::parse_octal("755").unwrap();
        assert_eq!(m.symbolic(), "rwxr-xr-x");
        assert_eq!(m.octal(), "755");
        assert_eq!(Mode::parse_octal("0644").unwrap().symbolic(), "rw-r--r--");
        assert_eq!(Mode::parse_octal("4755").unwrap().symbolic(), "rwsr-xr-x");
        assert_eq!(Mode::parse_octal("1777").unwrap().symbolic(), "rwxrwxrwt");
        assert_eq!(Mode::parse_octal("2640").unwrap().symbolic(), "rw-r-S---");
        for bad in ["8080", "75", "abc", "77777", "0"] {
            assert!(Mode::parse_octal(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn parses_ls_listing() {
        let (kind, m) = Mode::parse_symbolic("drwxr-xr-x").unwrap();
        assert_eq!((kind, m.octal()), (Some('d'), "755".to_string()));
        let (kind, m) = Mode::parse_symbolic("rwsr-xr-x").unwrap();
        assert_eq!((kind, m.octal()), (None, "4755".to_string()));
        assert_eq!(
            Mode::parse_symbolic("drwxrwxrwt").unwrap().1.octal(),
            "1777"
        );
        assert!(Mode::parse_symbolic("rwxr-xr-q").is_none());
        assert!(Mode::parse_symbolic("readme.md").is_none());
    }

    #[test]
    fn meanings() {
        assert_eq!(digit_meaning(7), "ler, escrever e executar");
        assert_eq!(digit_meaning(5), "ler e executar");
        assert_eq!(digit_meaning(0), "nenhuma permissão");
        assert_eq!(
            digit_meaning_dir(7),
            "listar, criar/apagar arquivos e entrar (cd)"
        );
        assert_eq!(digit_meaning_dir(5), "listar e entrar (cd)");
        assert!(
            digit_meaning_dir(6).contains("sem efeito")
                || digit_meaning_dir(6).contains("não tem efeito")
        );
    }

    #[test]
    fn umask_defaults() {
        let (file, dir) = umask_result(Mode(0o022));
        assert_eq!((file.octal(), dir.octal()), ("644".into(), "755".into()));
        let (file, dir) = umask_result(Mode(0o077));
        assert_eq!((file.octal(), dir.octal()), ("600".into(), "700".into()));
    }

    #[test]
    fn symbolic_changes() {
        let c = parse_symbolic_change("u+x").unwrap();
        assert_eq!(c[0].description, "adiciona execução para o dono");
        let c = parse_symbolic_change("go-w").unwrap();
        assert_eq!(c[0].description, "remove escrita do grupo e dos outros");
        let c = parse_symbolic_change("a=r,u+w").unwrap();
        assert_eq!(c.len(), 2);
        assert!(
            c[0].description
                .starts_with("define exatamente leitura para todos")
        );
        assert!(
            parse_symbolic_change("+x").unwrap()[0]
                .description
                .contains("umask")
        );
        assert!(parse_symbolic_change("755").is_none());
        assert!(parse_symbolic_change("arquivo.txt").is_none());
    }

    #[test]
    fn risky_modes() {
        assert!(!risks(Mode(0o777)).is_empty());
        assert!(risks(Mode(0o755)).is_empty());
        assert!(!risks(Mode(0o4755)).is_empty());
    }
}
