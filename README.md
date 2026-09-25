# TermSense (`ts`)

Assistente de conhecimento para o terminal. Enquanto você digita, o `ts` mostra comandos, opções, exemplos e conceitos de **Linux, shell, regex, redes, SSH, Git, Docker, pacotes e WSL**, explica o que cada parte de um comando faz e diz o que fazer com uma mensagem de erro colada.

- Funciona **offline**, é **local** e **determinístico**: a mesma consulta dá sempre o mesmo resultado.
- Não usa servidor, banco de dados, IA nem rede.
- Só consulta. **Nunca executa comandos**, não altera arquivos e não mexe no shell.

```text
┌ TermSense ─────────────────────────────────────── comando · 409 entradas ┐
│ > ss -ltnp | grep ':8080'                                               │
└─────────────────────────────────────────────────────────────────────────┘
┌ Sugestões (9) ───────────────┐┌ Pré-visualização ───────────────────────┐
│▸ análise  Explicar: ss -ltnp…││ ss -ltnp | grep ':8080'  · explicação   │
│  exemplo  grep "ERROR" app.log││ PARTES                                  │
│  comando  grep  Busca padrões…││   ss      comando  Investiga sockets…   │
│  comando  ss    Investiga so… ││     -ltnp opções   -l -t -n -p          │
│                               ││       -l  opção    --listening · …      │
└───────────────────────────────┘└─────────────────────────────────────────┘
 ↑↓ navegar   Enter abrir   Tab completar   PgUp/PgDn rolar   Esc sair
```

## Por que existe

Em man pages e buscas na web, você encontra o comando mas raramente entende *por que* ele funciona. O TermSense junta autocomplete, documentação, cheat sheets e explicação num só lugar, dentro do terminal. Assim você aprende fazendo: `ss -ltnp | grep ':8080'` vira "`ss` consulta sockets, `-l` só os que estão em escuta, `-t` TCP, `-n` sem resolver nomes, `-p` mostra o processo, `|` envia o stdout para o `grep`, que filtra a porta 8080 (HTTP alternativo)".

## Instalação

### Opção 1 — Executável pronto (sem clonar, sem Rust)

Use esta opção em outra máquina, como o WSL do trabalho. O pacote traz um binário estático para Linux x86_64 com a base de conhecimento embutida.

```bash
tar -xzf termsense-0.1.0-x86_64-linux.tar.gz
cd termsense-0.1.0-x86_64-linux
./install.sh            # copia o ts para ~/.local/bin
```

Para conferir a integridade do arquivo baixado: `sha256sum -c termsense-0.1.0-x86_64-linux.tar.gz.sha256`.

Para gerar o pacote numa máquina que tem o repositório e o Rust:

```bash
./scripts/package.sh    # cria dist/termsense-<versão>-x86_64-linux.tar.gz
```

### Opção 2 — A partir do código-fonte

Requisitos: Rust estável (1.88+). Para instalar o Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

```bash
git clone https://github.com/Fariawillyan/termSense.git
cd termSense
./install.sh            # verifica o cargo, compila em release e instala em ~/.local/bin/ts
```

O `install.sh`:

1. verifica o Rust/Cargo (também procura em `~/.cargo/bin`);
2. compila com `cargo build --release`;
3. cria o diretório de destino;
4. instala o `ts`;
5. confere se o diretório está no `PATH`.

Ele **não modifica** o `.bashrc`. Se `~/.local/bin` não estiver no `PATH`, o script mostra a linha para você adicionar. Para instalar em outro lugar, use `INSTALL_DIR=/outro/dir ./install.sh`.

> O pacote `moreutils` também tem um comando `ts` (timestamp). Se ele vier antes no `PATH`, o `install.sh` avisa.

## Como executar

```bash
ts                                  # interface interativa
ts grep -r                          # abre já com uma consulta
ts -p "quem usa a porta 8080"       # imprime o resultado, sem interface
ts -p "ss -ltnp | grep ':8080'"     # explicação de um comando
ts --check                          # valida os seus JSON de conhecimento
eval "$(ts --init bash)"            # integração com o shell (veja abaixo)
ts --help
```

### Teclas

| Tecla | Na busca | Nos detalhes |
|---|---|---|
| ↑ ↓ | navega nos resultados | rola |
| Enter | abre os detalhes | abre o link focado |
| Tab | completa a entrada com a sugestão | próximo link |
| Shift+Tab | — | link anterior |
| ← → | move o cursor | ← volta, → próximo link |
| PgUp / PgDn | rola a pré-visualização | rola uma página |
| Ctrl+U / Ctrl+W | apaga a linha / a palavra | — |
| Esc | sai | volta (também `q`; `j`/`k` rolam) |
| Ctrl+C | sai | sai |

### Integração com o shell

Com a integração, **Alt+H** abre o `ts` com a linha que você está digitando. Esc devolve a linha editada ao prompt; Ctrl+C mantém a original. Quem executa é o shell, quando você apertar Enter nele: o `ts` continua sem executar nada.

```bash
eval "$(ts --init bash)"            # no ~/.bashrc
eval "$(ts --init zsh)"             # no ~/.zshrc
ts --init fish | source             # no ~/.config/fish/config.fish
```

Para usar outra tecla no bash, defina antes `TERMSENSE_KEY='\C-g'` (no zsh, `TERMSENSE_KEY='^G'`). O `install.sh` não altera esses arquivos: a linha é você quem adiciona.

### O que digitar

| Entrada | O que acontece |
|---|---|
| `gr` | comandos pelo prefixo, tolerando erros de digitação (`gerp` → `grep`) |
| `grep` | documentação completa: descrição, uso, opções, exemplos, relacionados |
| `grep -` | opções do comando; Tab completa |
| `grep -r` | exemplos que usam as flags digitadas (inclusive `-rin`) |
| `git s` | subcomandos (`git stash`, `git status`, `git switch`) |
| `ss -ltnp \| grep ':8080'` | explicação parte por parte, análise léxica e fluxo do pipe |
| `quem usa a porta 8080` | linguagem natural: receita com os comandos já com a porta 8080 |
| `porta 5432` | informações da porta (PostgreSQL) e como investigá-la |
| `não consigo acessar servidor` | troubleshooting guiado: DNS → ping → rota → porta → socket → HTTP → TLS |
| `regex ^[0-9]+$` | análise da regex: tokens, interpretação, exemplos que casam e não casam |
| `/24`, `10.0.0.0/8` | calculadora CIDR (máscara, total × hosts utilizáveis, rede, broadcast) |
| `192.168.1.10` | classificação do IP (privado, loopback, público…) |
| `755`, `rwxr-xr-x`, `-rw-r--r--` | calculadora de permissões: octal ↔ simbólico, quem pode o quê, bits especiais |
| `chmod u+x,go-w f`, `umask 027` | o modo simbólico em palavras; arquivos e diretórios que a umask produz |
| `*/5 * * * *`, `cron 0 3 * * 1` | o agendamento em português, campo a campo, e os cuidados do cron |
| `exit 137`, `curl exit 7` | significado do exit code, sinal (128 + N) e o que cada programa quer dizer |
| `for f in *.log; do gzip "$f"; done` | estruturas do shell: `for`, `while`, `if`, `case`, `[[ ]]`, `$(( ))`, heredoc |
| `echo "${f%.*}" $#` | expansões `${...}` e variáveis especiais explicadas |
| `sed 's/a/b/g' f`, `awk '{print $1}' f` | o script sed e o programa awk decompostos peça por peça |
| `Permission denied (publickey)` | cole a mensagem de erro: causa provável e passos para resolver |
| `kubectl` (fora da base) | diz que não conhece, mostra se está instalado e como ensinar ao `ts` |

## Conteúdo da base

São 409 entradas (214 comandos, 109 conceitos e 86 receitas), divididas por tema:

| Tema | Entradas |
|---|---|
| Linux (arquivos, permissões, discos, usuários) | 65 |
| Shell (estruturas, variáveis, expansões) | 40 |
| Redes: comandos e firewall | 38 |
| Mensagens de erro | 33 |
| Git | 33 |
| HTTP e TLS | 32 |
| Docker | 29 |
| Processos, serviços e agendamento | 27 |
| Conceitos de rede | 25 |
| Texto (grep, sed, awk, jq…) | 23 |
| SSH | 17 |
| Pacotes (apt, dpkg, dnf) | 13 |
| WSL | 12 |
| Regex | 12 |
| Editores e sessões (vim, nano, tmux) | 5 |
| Troubleshooting de rede | 5 |

Quando um comando pode não vir instalado por padrão (`dig`, `htop`, `lsof`, `traceroute`, `nmap`…), a página dele diz qual pacote instalar. Ela também mostra se o executável existe no `PATH` desta máquina: o `ts` confere isso lendo o sistema de arquivos, sem executar nada.

## Como adicionar conhecimento

A base fica em `knowledge/*.json` e é compilada dentro do binário. Para acrescentar conhecimento **sem recompilar**, crie arquivos JSON em:

```text
~/.config/termsense/knowledge/      (ou $XDG_CONFIG_HOME/termsense/knowledge/)
```

Uma entrada com `id` já existente substitui a original; ids novos são acrescentados. Um arquivo inválido não impede o `ts` de abrir: o erro aparece como aviso na barra inferior. Para ver todos os problemas de uma vez (JSON inválido, `related` apontando para o nada, categoria inexistente), rode `ts --check`.

```json
{
  "category": "linux",
  "entries": [
    {
      "name": "ncdu",
      "kind": "command",
      "summary": "Mostra interativamente o que ocupa espaço em disco",
      "usage": "ncdu [OPÇÕES] [DIRETÓRIO]",
      "install": "Debian/Ubuntu: sudo apt install ncdu.",
      "aliases": ["o que ocupa espaço", "disco cheio interativo"],
      "tags": ["discos", "espaço"],
      "options": [
        { "short": "-x", "description": "Não atravessa para outros sistemas de arquivos" }
      ],
      "examples": [
        { "command": "ncdu -x /", "description": "Navega pelas pastas maiores da raiz" }
      ],
      "related": ["du", "df"]
    }
  ]
}
```

O formato completo, com todos os campos, está em [CONTRIBUTING.md](CONTRIBUTING.md).

## Desenvolvimento

```bash
cargo run                 # executa em modo debug
cargo run -- -p "grep -"  # modo texto, útil para inspecionar resultados
cargo build --release     # binário otimizado em target/release/ts
cargo test                # testes unitários
cargo clippy --all-targets
cargo fmt
```

- Arquitetura e decisões de design: [ARCHITECTURE.md](ARCHITECTURE.md)
- Como contribuir com código e conhecimento: [CONTRIBUTING.md](CONTRIBUTING.md)

## Como testar

`cargo test` roda 148 testes unitários. Eles cobrem tokenizer, motor de busca e ranking, loader e integridade da base, parser e analisador de regex, matcher, redes (CIDR, portas, IPs), permissões, cron, exit codes, sed e awk, gramática do shell, autocomplete contextual, explicador de comandos, integração com o shell, estado da aplicação (teclas, Tab, Enter, Esc) e renderização da interface num terminal simulado.

Alguns casos que os testes garantem:

- `grep` retorna `grep` em primeiro lugar;
- `gr` retorna `grep`, `groups` e `git`, sempre na mesma ordem;
- uma lista fixa de consultas reais tem o primeiro resultado esperado (`golden_queries`): `agendar tarefa`, `como sair do vim`, mensagens de erro coladas, `755`, `exit 127`…;
- uma palavra fora da base é admitida, não "adivinhada" (`crontab` nunca vira "Contar ocorrências");
- `for f in *.log; do gzip "$f"; done` reconhece `for`, a variável do laço, `in`, `do` e `done`;
- `^[0-9]+$` produz `START_ANCHOR`, `CHARACTER_CLASS`, `QUANTIFIER`, `END_ANCHOR`;
- `porta 8080` chega a `ss`, `lsof` e `nc`;
- todo `related` aponta para uma entrada que existe e toda receita tem comandos.

## Roadmap

O TermSense continua local e determinístico: não há planos de IA.

| Versão | Escopo |
|---|---|
| v0.1 | TUI, busca, base local, Linux, shell, regex, redes, Git, Docker, SSH |
| **v0.2** (em desenvolvimento) | confiança na busca (admite o que não sabe), gramática do shell, analisadores de permissões, cron, exit codes, sed e awk, catálogo de mensagens de erro, pacotes, WSL, editores, `ts --check` e integração com Bash, Zsh e Fish |
| v0.3 | histórico, favoritos e personalização (`config.toml`) |
| v0.4 | sinônimos e aliases em inglês mais completos |
| v0.5 | modo treinamento (Linux, regex, redes) |
| v1.0 | assistente de conhecimento de terminal completo |

## Licença

MIT.
