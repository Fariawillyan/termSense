//! Exit codes: what `$?` means. General conventions, death by signal
//! (128 + N) and the codes some programs define (`curl` 7, `grep` 1...).

/// Meaning of an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: u8,
    /// One-line meaning.
    pub meaning: String,
    pub detail: Option<&'static str>,
    /// The process was killed by this signal (codes 129–159).
    pub signal: Option<Signal>,
    /// Program-specific meanings of this code, `(program, meaning)`.
    pub programs: Vec<(&'static str, &'static str)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    pub number: u8,
    pub name: &'static str,
    pub description: &'static str,
}

const SIGNALS: &[(u8, &str, &str)] = &[
    (
        1,
        "SIGHUP",
        "o terminal foi fechado (a sessão caiu); use nohup ou tmux para sobreviver",
    ),
    (2, "SIGINT", "interrompido com Ctrl+C"),
    (3, "SIGQUIT", "Ctrl+\\: encerra e pode gerar core dump"),
    (
        4,
        "SIGILL",
        "instrução inválida (binário para outra CPU ou corrompido)",
    ),
    (5, "SIGTRAP", "armadilha de depuração"),
    (
        6,
        "SIGABRT",
        "o próprio programa abortou (assert falhou, erro interno, double free)",
    ),
    (
        7,
        "SIGBUS",
        "erro de barramento: acesso a memória mapeada inválida",
    ),
    (
        8,
        "SIGFPE",
        "erro aritmético, como divisão inteira por zero",
    ),
    (
        9,
        "SIGKILL",
        "morto à força: kill -9, OOM killer (falta de memória) ou limite de memória do container",
    ),
    (10, "SIGUSR1", "sinal definido pelo usuário 1"),
    (
        11,
        "SIGSEGV",
        "falha de segmentação: acesso inválido à memória (bug no programa)",
    ),
    (12, "SIGUSR2", "sinal definido pelo usuário 2"),
    (
        13,
        "SIGPIPE",
        "escreveu num pipe já fechado; comum e inofensivo em `cmd | head`",
    ),
    (14, "SIGALRM", "alarme (temporizador) expirou"),
    (
        15,
        "SIGTERM",
        "pedido de encerramento: kill, systemctl stop, docker stop",
    ),
    (
        16,
        "SIGSTKFLT",
        "falha de pilha do coprocessador (obsoleto)",
    ),
    (17, "SIGCHLD", "um processo filho terminou"),
    (18, "SIGCONT", "continuar após pausa"),
    (19, "SIGSTOP", "pausado (não pode ser tratado)"),
    (20, "SIGTSTP", "suspenso com Ctrl+Z"),
    (
        21,
        "SIGTTIN",
        "leitura do terminal por processo em segundo plano",
    ),
    (
        22,
        "SIGTTOU",
        "escrita no terminal por processo em segundo plano",
    ),
    (23, "SIGURG", "dados urgentes no socket"),
    (
        24,
        "SIGXCPU",
        "estourou o limite de tempo de CPU (ulimit -t)",
    ),
    (
        25,
        "SIGXFSZ",
        "estourou o limite de tamanho de arquivo (ulimit -f)",
    ),
    (26, "SIGVTALRM", "temporizador virtual expirou"),
    (27, "SIGPROF", "temporizador de profiling expirou"),
    (28, "SIGWINCH", "a janela do terminal mudou de tamanho"),
    (29, "SIGIO", "E/S disponível"),
    (30, "SIGPWR", "falha de energia"),
    (31, "SIGSYS", "chamada de sistema inválida (filtro seccomp)"),
];

/// `(program, code, meaning)`.
const PROGRAMS: &[(&str, u8, &str)] = &[
    ("grep", 1, "nenhuma linha casou (não é erro)"),
    (
        "grep",
        2,
        "erro: arquivo inexistente, sem permissão ou regex inválida",
    ),
    ("diff", 1, "os arquivos são diferentes"),
    ("diff", 2, "erro: arquivo inexistente ou ilegível"),
    ("cmp", 1, "os arquivos são diferentes"),
    ("test", 1, "a condição é falsa"),
    ("[", 1, "a condição é falsa"),
    ("[[", 1, "a condição é falsa"),
    ("curl", 1, "protocolo não suportado"),
    ("curl", 3, "URL malformada"),
    ("curl", 5, "não resolveu o proxy"),
    ("curl", 6, "não resolveu o host (DNS)"),
    (
        "curl",
        7,
        "falha ao conectar: porta fechada, serviço fora do ar ou firewall",
    ),
    ("curl", 22, "resposta HTTP 400 ou maior com --fail (-f)"),
    (
        "curl",
        23,
        "erro ao gravar a saída (disco cheio, sem permissão)",
    ),
    ("curl", 28, "tempo esgotado (timeout)"),
    ("curl", 35, "erro no handshake TLS/SSL"),
    ("curl", 47, "redirecionamentos demais"),
    ("curl", 52, "o servidor fechou a conexão sem responder nada"),
    ("curl", 56, "falha ao receber dados (conexão resetada)"),
    (
        "curl",
        60,
        "certificado do servidor não verificado (CA desconhecida ou nome errado)",
    ),
    ("curl", 67, "login recusado pelo servidor"),
    ("wget", 4, "falha de rede"),
    ("wget", 5, "falha na verificação TLS"),
    ("wget", 6, "falha de autenticação"),
    ("wget", 8, "o servidor respondeu com erro (4xx ou 5xx)"),
    (
        "ssh",
        255,
        "erro do próprio ssh: conexão recusada, timeout, autenticação ou host key",
    ),
    (
        "scp",
        1,
        "erro na cópia (arquivo inexistente, sem permissão)",
    ),
    (
        "rsync",
        12,
        "erro no protocolo (rsync ausente no destino ou saída estranha no login)",
    ),
    (
        "rsync",
        23,
        "transferência parcial: alguns arquivos falharam",
    ),
    ("rsync", 24, "arquivos sumiram durante a cópia"),
    ("rsync", 255, "erro de conexão SSH"),
    (
        "timeout",
        124,
        "o tempo limite estourou e o comando foi encerrado",
    ),
    ("timeout", 125, "falha do próprio timeout"),
    (
        "timeout",
        137,
        "o comando foi morto com SIGKILL (timeout -s KILL ou -k)",
    ),
    (
        "git",
        1,
        "erro ou diferença encontrada (git diff --exit-code, git merge com conflito)",
    ),
    (
        "git",
        128,
        "erro fatal (mensagens fatal:, como not a git repository)",
    ),
    ("git", 129, "uso incorreto: opção inválida"),
    (
        "docker",
        125,
        "erro do próprio docker: opção inválida ou daemon",
    ),
    (
        "docker",
        126,
        "o comando do container não pôde ser executado",
    ),
    ("docker", 127, "comando não encontrado dentro do container"),
    (
        "docker",
        137,
        "container morto por SIGKILL ou falta de memória (veja .State.OOMKilled)",
    ),
    (
        "docker",
        143,
        "container encerrado com SIGTERM (docker stop)",
    ),
    ("systemctl", 1, "a operação falhou"),
    (
        "systemctl",
        3,
        "a unidade não está ativa (status, is-active)",
    ),
    ("systemctl", 4, "a unidade não existe"),
    ("ping", 1, "nenhuma resposta recebida"),
    ("ping", 2, "erro: nome não resolve ou rede inalcançável"),
    ("nc", 1, "a conexão falhou"),
    (
        "apt",
        100,
        "erro do apt: pacote não encontrado, lock ocupado ou falha no download",
    ),
    (
        "apt-get",
        100,
        "erro do apt: pacote não encontrado, lock ocupado ou falha no download",
    ),
    ("make", 2, "algum alvo falhou"),
    (
        "ls",
        1,
        "problema menor (ex.: sem acesso a um subdiretório)",
    ),
    ("ls", 2, "problema sério (ex.: o arquivo não existe)"),
    (
        "find",
        1,
        "algum erro no caminho (ex.: Permission denied em subdiretórios)",
    ),
    ("kill", 1, "o processo não existe ou falta permissão"),
    ("pgrep", 1, "nenhum processo encontrado"),
    ("pkill", 1, "nenhum processo encontrado"),
    (
        "sudo",
        1,
        "senha errada, comando não permitido ou erro de configuração",
    ),
    ("bash", 2, "erro de sintaxe ou uso incorreto de um builtin"),
    ("sh", 2, "erro de sintaxe ou uso incorreto de um builtin"),
];

pub fn signal(number: u8) -> Option<Signal> {
    SIGNALS
        .iter()
        .find(|(n, ..)| *n == number)
        .map(|&(number, name, description)| Signal {
            number,
            name,
            description,
        })
}

/// Explains `code`; `None` above 255 (exit codes are one byte).
pub fn explain(code: u16) -> Option<ExitInfo> {
    let code = u8::try_from(code).ok()?;
    let programs: Vec<(&'static str, &'static str)> = PROGRAMS
        .iter()
        .filter(|(_, c, _)| *c == code)
        .map(|&(p, _, m)| (p, m))
        .collect();
    let mut signal_info = None;
    let (meaning, detail): (String, Option<&'static str>) = match code {
        0 => (
            "Sucesso".into(),
            Some("Por convenção, 0 é o único código de sucesso. É o que && testa."),
        ),
        1 => (
            "Erro genérico: o comando falhou".into(),
            Some(
                "O motivo costuma estar na saída de erro (stderr). Alguns programas usam 1 para \"não encontrado\" ou \"diferente\", sem ser erro (veja a tabela).",
            ),
        ),
        2 => (
            "Uso incorreto: argumentos inválidos ou erro de sintaxe".into(),
            Some(
                "No bash, 2 indica mau uso de um builtin. Muitos programas usam 2 para erros de linha de comando.",
            ),
        ),
        126 => (
            "Encontrado, mas não pode ser executado".into(),
            Some(
                "Falta permissão de execução (chmod +x), é um diretório, ou o arquivo não é um executável válido.",
            ),
        ),
        127 => (
            "Comando não encontrado".into(),
            Some(
                "O nome está errado ou o programa não está no PATH. Em scripts executados pelo cron, o PATH é mínimo: use caminhos absolutos.",
            ),
        ),
        128 => ("Argumento inválido para exit".into(), None),
        129..=159 => {
            let n = code - 128;
            match signal(n) {
                Some(s) => {
                    signal_info = Some(s);
                    (
                        format!("Terminado pelo sinal {n} ({}): {}", s.name, s.description),
                        Some(
                            "Códigos acima de 128 indicam que o processo morreu por um sinal: código = 128 + número do sinal.",
                        ),
                    )
                }
                None => (
                    format!("Terminado pelo sinal {n} (sinal de tempo real)"),
                    None,
                ),
            }
        }
        255 => (
            "Código fora da faixa ou erro de conexão".into(),
            Some("exit -1 vira 255 (o código é um byte). O ssh usa 255 para os próprios erros."),
        ),
        3..=125 => (
            "Código definido pelo próprio programa".into(),
            Some(
                "Não há convenção geral: consulte a documentação do comando (man, seção EXIT STATUS).",
            ),
        ),
        _ => ("Código definido pelo próprio programa".into(), None),
    };
    Some(ExitInfo {
        code,
        meaning,
        detail,
        signal: signal_info,
        programs,
    })
}

const KEYWORDS: &[&str] = &[
    "exit", "code", "status", "codigo", "código", "saida", "saída", "retorno", "rc",
];
const FILLERS: &[&str] = &[
    "de", "do", "da", "o", "com", "the", "of", "with", "is", "=", ":",
];

/// Recognizes `exit 127`, `exit code 137`, `curl exit 7`,
/// `código de saída 1 do grep`: returns the code and the program, if named.
pub fn parse_query(input: &str) -> Option<(u16, Option<String>)> {
    let mut code = None;
    let mut program = None;
    let mut keyword = false;
    for word in input.split_whitespace() {
        let lower = word.to_lowercase();
        if let Some(rest) = lower.strip_prefix("$?") {
            keyword = true;
            let rest = rest.trim_start_matches(['=', ':']);
            if !rest.is_empty() {
                code = Some(rest.parse().ok()?);
            }
            continue;
        }
        let w = lower.trim_matches(|c: char| matches!(c, ',' | '?' | ':' | '='));
        if w.is_empty() {
            continue;
        }
        if KEYWORDS.contains(&w) {
            keyword = true;
        } else if w.bytes().all(|b| b.is_ascii_digit()) && w.len() <= 3 {
            if code.is_some() {
                return None;
            }
            code = Some(w.parse().ok()?);
        } else if !FILLERS.contains(&w) {
            let name_like = w
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.[".contains(&b));
            if program.is_some() || !name_like {
                return None;
            }
            program = Some(w.to_string());
        }
    }
    match (keyword, code) {
        (true, Some(c)) => Some((c, program)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_codes() {
        assert!(explain(0).unwrap().meaning.starts_with("Sucesso"));
        assert!(explain(127).unwrap().meaning.contains("não encontrado"));
        assert!(
            explain(126)
                .unwrap()
                .meaning
                .contains("não pode ser executado")
        );
        assert!(explain(256).is_none());
    }

    #[test]
    fn signals_above_128() {
        let e = explain(137).unwrap();
        assert_eq!(e.signal.unwrap().name, "SIGKILL");
        assert!(e.meaning.contains("OOM"));
        assert_eq!(explain(130).unwrap().signal.unwrap().name, "SIGINT");
        assert_eq!(explain(143).unwrap().signal.unwrap().name, "SIGTERM");
        assert!(e.programs.iter().any(|(p, _)| *p == "docker"));
    }

    #[test]
    fn program_codes() {
        let e = explain(7).unwrap();
        assert!(
            e.programs
                .iter()
                .any(|(p, m)| *p == "curl" && m.contains("conectar"))
        );
    }

    #[test]
    fn queries() {
        assert_eq!(parse_query("exit 127"), Some((127, None)));
        assert_eq!(parse_query("exit code 137"), Some((137, None)));
        assert_eq!(parse_query("curl exit 7"), Some((7, Some("curl".into()))));
        assert_eq!(
            parse_query("código de saída 1 do grep"),
            Some((1, Some("grep".into())))
        );
        assert_eq!(parse_query("$? = 130"), Some((130, None)));
        assert_eq!(parse_query("exit"), None);
        assert_eq!(parse_query("porta 8080"), None);
        assert_eq!(parse_query("exit 1 2"), None);
        assert_eq!(parse_query("quero sair do vim com exit 1 agora"), None);
    }
}
