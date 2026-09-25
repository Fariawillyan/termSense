# TermSense (`ts`)

Assistente de conhecimento para o terminal. Enquanto você digita, o `ts` mostra comandos, opções, exemplos e conceitos de **Linux, shell, regex, redes, SSH, Git e Docker**, e explica o que cada parte de um comando faz.

- Funciona **offline**, é **local** e **determinístico**: a mesma consulta dá sempre o mesmo resultado.
- Não usa servidor, banco de dados, IA nem rede.
- Só consulta. **Nunca executa comandos**, não altera arquivos e não mexe no shell.

```text
┌ TermSense ─────────────────────────────────────── comando · 253 entradas ┐
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

## Conteúdo da base

São 253 entradas (126 comandos, 90 conceitos e 37 receitas), divididas por tema:

| Tema | Entradas |
|---|---|
| Linux (arquivos, permissões, discos) | 38 |
| Redes: comandos | 34 |
| HTTP e TLS | 32 |
| Conceitos de rede | 25 |
| Git | 22 |
| Processos e serviços | 20 |
| Shell | 18 |
| SSH | 17 |
| Docker | 17 |
| Texto (grep, sed, awk…) | 13 |
| Regex | 12 |
| Troubleshooting de rede | 5 |

Quando um comando pode não vir instalado por padrão (`dig`, `htop`, `lsof`, `traceroute`, `nmap`…), a página dele diz qual pacote instalar. Ela também mostra se o executável existe no `PATH` desta máquina: o `ts` confere isso lendo o sistema de arquivos, sem executar nada.

## Como adicionar conhecimento

A base fica em `knowledge/*.json` e é compilada dentro do binário. Para acrescentar conhecimento **sem recompilar**, crie arquivos JSON em:

```text
~/.config/termsense/knowledge/      (ou $XDG_CONFIG_HOME/termsense/knowledge/)
```

Uma entrada com `id` já existente substitui a original; ids novos são acrescentados. Um arquivo inválido não impede o `ts` de abrir: o erro aparece como aviso na barra inferior.

```json
{
  "category": "linux",
  "entries": [
    {
      "name": "rsync",
      "kind": "command",
      "summary": "Sincroniza arquivos local ou remotamente, copiando só o que mudou",
      "usage": "rsync [OPÇÕES] ORIGEM DESTINO",
      "aliases": ["sincronizar pastas", "backup incremental"],
      "tags": ["arquivos", "backup"],
      "options": [
        { "short": "-a", "long": "--archive", "description": "Preserva permissões, datas e links" },
        { "short": "-z", "long": "--compress", "description": "Comprime na transferência" }
      ],
      "examples": [
        { "command": "rsync -az src/ user@host:/srv/app/", "description": "Envia só as diferenças" }
      ],
      "related": ["scp", "cp"]
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

`cargo test` roda 111 testes unitários. Eles cobrem tokenizer, motor de busca e ranking, loader e integridade da base, parser e analisador de regex, matcher, redes (CIDR, portas, IPs), autocomplete contextual, explicador de comandos, estado da aplicação (teclas, Tab, Enter, Esc) e renderização da interface num terminal simulado.

Alguns casos que os testes garantem:

- `grep` retorna `grep` em primeiro lugar;
- `gr` retorna `grep`, `groups` e `git`, sempre na mesma ordem;
- `^[0-9]+$` produz `START_ANCHOR`, `CHARACTER_CLASS`, `QUANTIFIER`, `END_ANCHOR`;
- `porta 8080` chega a `ss`, `lsof` e `nc`;
- todo `related` aponta para uma entrada que existe e toda receita tem comandos.

## Roadmap

| Versão | Escopo |
|---|---|
| **v0.1** (atual) | TUI, busca, base local, Linux, shell, regex, redes, Git, Docker, SSH |
| v0.2 | autocomplete contextual e tokenizer mais completos, mais opções e relacionamentos |
| v0.3 | histórico, favoritos, personalização |
| v0.4 | busca semântica local |
| v0.5–v0.6 | provedor de IA local (Ollama) opcional para explicações |
| v0.7 | modo treinamento (Linux, regex, redes) |
| v0.8 | integração com Bash, Zsh e Fish |
| v1.0 | assistente de conhecimento de terminal completo |

## Licença

MIT.
