# Contribuindo com o TermSense

Há dois tipos de contribuição: **conhecimento** (JSON, sem Rust) e **código**. As duas passam pelas mesmas verificações automáticas.

## Preparando o ambiente

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust 1.88+
git clone https://github.com/Fariawillyan/termSense.git && cd termSense
cargo test
cargo run
```

Antes de abrir um PR, rode e deixe tudo limpo:

```bash
cargo fmt
cargo clippy --all-targets      # sem avisos
cargo test                      # todos passando
cargo build --release
```

## Contribuindo com conhecimento

Os arquivos ficam em `knowledge/`, um por tema:

| Arquivo | Tema |
|---|---|
| `categories.json` | lista de categorias (id, nome, descrição) |
| `linux.json`, `text.json`, `shell.json`, `processes.json` | Linux, texto, shell (inclusive estruturas como `for` e `[[`), processos e agendamento |
| `regex.json` | regex e padrões prontos |
| `networking.json` | comandos de rede, firewall e troubleshooting |
| `network-concepts.json`, `http.json` | conceitos de rede, HTTP e TLS |
| `ssh.json`, `git.json`, `docker.json` | SSH, Git, Docker |
| `packages.json`, `editors.json`, `wsl.json` | apt/dpkg/dnf; vim, nano e tmux; WSL |
| `openshift.json`, `java.json`, `cpp.json`, `node.json` | OpenShift (`oc`/`kubectl`); Java e Maven; C/C++, CMake e GoogleTest; Node.js e npm |
| `errors.json` | catálogo de mensagens de erro (veja abaixo) |

Para testar sem recompilar, coloque um JSON em `~/.config/termsense/knowledge/` e rode `ts --check`, depois `ts`. Para incorporar à base, edite o arquivo do tema. Se criar um arquivo novo, registre-o em `EMBEDDED` (`src/knowledge/loader.rs`).

O índice de busca da base embutida é montado na compilação (`build.rs`): qualquer `cargo build` depois de editar `knowledge/` o refaz sozinho. Um JSON embutido inválido faz a compilação falhar, com a mensagem do erro.

### Estrutura de um arquivo

```json
{
  "category": "networking",
  "categories": [],
  "entries": []
}
```

O `category` do arquivo é o padrão das entradas que não declaram o seu.

### Campos de uma entrada

| Campo | Obrigatório | Descrição |
|---|---|---|
| `name` | sim | Nome exibido (`grep`, `git status`, `TCP`) |
| `kind` | sim | `command`, `concept` ou `recipe` |
| `summary` | sim | Uma linha, **sem ponto final** |
| `id` | não | Padrão: `name` em slug (`git status` → `git-status`). Declare quando houver conflito (`ip` comando × `ip-protocol` conceito) |
| `category` | não | Id de uma categoria declarada |
| `description` | não | Texto mais longo, com o porquê e o como |
| `usage` | não | Sintaxe (`grep [OPÇÕES] PADRÃO [ARQUIVO...]`) |
| `parent` | não | Id do comando pai, para subcomandos |
| `wrapper` | não | `true` se o comando executa outro (`sudo`, `nohup`, `xargs`) |
| `builtin` | não | `true` para recursos do shell sem executável no `PATH` (`cd`, `export`, `for`, `[[`): a página diz isso em vez de "não encontrado" |
| `names` | não | Outros nomes do mesmo comando: `["mvnw"]` em `mvn`, `["kubectl"]` em `oc`. Contam como nome exato na busca e na explicação (`./mvnw` resolve pelo nome do arquivo) |
| `phases` | não | `true` se os subcomandos podem vir em sequência: `mvn clean install` |
| `flag_prefix` | não | Flags com este prefixo identificam o programa mesmo com um executável do projeto: `"--gtest_"` no GoogleTest |
| `aliases` | não | Sinônimos e frases em linguagem natural (pt/en): é o que faz `quem usa a porta` achar a receita |
| `tags` | não | Palavras-chave curtas |
| `options` | não | `{ "short": "-i", "long": "--ignore-case", "arg": "VALOR", "kind": "...", "prefix": false, "description": "..." }` (`kind` opcional, exige `arg`; `prefix: true` para flags coladas ao valor, como `-Xmx` em `-Xmx512m`) |
| `arguments` | não | `{ "name": "PADRÃO", "kind": "regex", "description": "...", "repeat": false }` |
| `examples` | não | `{ "command": "...", "description": "..." }` |
| `steps` | não | `{ "title": "...", "command": "...", "why": "..." }`: passos ordenados de receitas |
| `sections` | não | `{ "title": "...", "text": "...", "items": [], "rows": [["a", "b"]] }` |
| `related` | não | Ids relacionados (viram a árvore de relacionamentos) |
| `install` | não | Como instalar, quando o comando pode não vir por padrão |
| `warnings` | não | Riscos (aparecem em destaque nas páginas e nas explicações) |

Os tipos de `arguments[].kind` (e de `options[].kind`) são `text`, `regex`, `path`, `host`, `url`, `port`, `command`, `number`, `user`, `mode`, `umask`, `sed`, `awk` e `resource`. O explicador usa o tipo:

- `regex`: recebe a interpretação do padrão;
- `url` e `host`: são decompostos em esquema, host, porta e caminho;
- `mode`: permissões do `chmod` (`755` → `rwxr-xr-x`, `u+x` em palavras), inclusive as formas `-644` e `/111` do `find -perm`;
- `umask`: mostra as permissões que arquivos e diretórios novos recebem;
- `sed` e `awk`: o script ou programa é decomposto peça por peça (`sed -e SCRIPT` usa `"kind": "sed"` na opção);
- `resource`: recurso do OpenShift/Kubernetes (`pods`, `deployment/api`). O tipo é procurado entre os conceitos com a tag `recurso`: nome, aliases (`deployments`) ou tags com os nomes curtos (`deploy`). Ponha nomes curtos só em tags, para eles não anotarem argumentos de outros comandos.

### Opções no estilo do GCC e da JVM

- `-std=c++17`: declare a opção `-std` com `arg`; o `=` é separado sozinho.
- `-lpthread`, `-Iinclude`, `-O2`, `-DskipTests`: opção de uma letra com `arg`; o resto do token vira o valor.
- `-Xmx512m`, `-XX:+UseG1GC`, `-Wl,-rpath`: declare com `"prefix": true`. Entre um prefixo genérico (`-W`) e um específico (`-Wl,`), vence o mais longo, e uma opção exata (`-Wall`) vence qualquer prefixo.

### Opções que recebem valor

Declare `arg` em toda opção que consome um valor (`-X MÉTODO`, `-p PORTA`). Sem isso, o explicador trata o valor como argumento posicional. Opções de um hífen com várias letras (`find -name`, `ip -br`) funcionam: o nome inteiro é comparado antes de tentar separar letras.

### Templates

Comandos em `examples` e `steps` podem usar placeholders:

```text
ss -ltnp | grep ':{{port:8080}}'
nc -vz {{host:host}} {{port:8080}}
```

`{{port:8080}}` usa a porta citada na consulta (`porta 5432`) ou 8080 como padrão. As variáveis disponíveis são `port`, `host` e `url`. Chaves de shell e de Go template não são afetadas.

### Regras verificadas pelos testes

`cargo test` falha se:

- um JSON embutido não fizer parse ou tiver campo desconhecido (erro de digitação);
- houver id duplicado;
- um `related` ou `parent` apontar para id inexistente;
- uma entrada usar categoria não declarada;
- um `summary` terminar com ponto ou estiver vazio;
- uma flag não começar com `-`, ou uma opção tiver `kind` sem `arg`;
- uma receita não tiver `examples` nem `steps`;
- alguma entrada exigida pela especificação sumir;
- uma consulta da lista `golden_queries` (`src/assistant/mod.rs`) mudar de primeiro resultado.

As mesmas regras (menos as duas últimas) valem para os seus arquivos: `ts --check` valida `~/.config/termsense/knowledge/*.json`, ou os arquivos passados como argumento, e sai com código 1 se houver problema.

### Mensagens de erro

As receitas de `errors.json` são achadas quando a pessoa cola a mensagem inteira. O que faz isso funcionar são os `aliases`: coloque trechos **literais** da mensagem, sem as partes que variam (nomes de arquivo, hosts, PIDs). Quanto mais longo o trecho, mais específico: se duas receitas casam com a mesma linha, vence a que tem mais palavras de alias contidas nela (as frases se somam). Por exemplo, `bad interpreter: no such file or directory` faz a receita de CRLF vencer a genérica `no such file or directory`. Mensagens com `->` ou `|` passam pelo modo comando, mas, sem nenhum comando conhecido na linha, a busca vem primeiro.

### Boas práticas de conteúdo

- **Português do Brasil**, direto, explicando o *porquê*, e não só o *o quê*.
- Exemplos reais e copiáveis. Prefira nomes neutros (`app.log`, `user@host`, `example.com`).
- Em `aliases`, pense em como alguém descreveria a tarefa: "liberar porta", "address already in use", "disco cheio".
- Comandos perigosos entram como conhecimento, com `warnings`. Nunca sugira `curl | sh` sem aviso.
- Diga quando um comando é legado (`netstat`, `route`, `arp`) e qual é a alternativa moderna.
- Não use `id` com acento nem espaços; o slug automático cuida disso.

## Contribuindo com código

Leia [ARCHITECTURE.md](ARCHITECTURE.md) antes. Resumo das regras:

1. **A UI só apresenta.** Lógica nova vai para `assistant/`, `search/`, `regex/` ou `networking/`, produzindo `Suggestion`/`Document`. A UI não decide conteúdo.
2. **Serviços não dependem de UI** nem uns dos outros sem necessidade. `regex/` e `networking/` não importam `search/`.
3. **Determinismo.** Nada de aleatoriedade, relógio ou ordem de `HashMap` influenciando resultados. Toda ordenação precisa de desempate total.
4. **Somente consulta.** Não use `std::process::Command`, chamadas de rede nem escritas em disco. A única interação com o sistema é `system::which` (e, no modo widget, desenhar em `/dev/tty`).
5. **Dependências.** Não adicione crates sem necessidade clara; discuta no PR.
6. **Sem abstrações artificiais.** Crie trait ou módulo quando houver duas implementações reais ou uma fronteira clara de responsabilidade.
7. **Testes junto do código**, em `#[cfg(test)] mod tests`, cobrindo o comportamento (não detalhes internos).

### Receitas comuns

| Quero… | Onde |
|---|---|
| mudar a pontuação da busca | `search/ranking.rs` (níveis e pesos) e `search/engine.rs` (campos e índice invertido); rode os testes de `engine` e o `golden_queries` |
| reconhecer uma nova sintaxe de shell | `search/tokenizer.rs` (lexer/forma) e `search/context.rs` (papel; palavras reservadas em `Grammar`) |
| um novo tipo de análise (como permissões, cron, exit code) | módulo em `analysis/`, função no `Assistant` que devolve `Suggestion::analysis(...)` e página em `pages.rs` |
| mudar o que vai para o índice (campos, tokens) | `search/index.rs`; o `build.rs` e o teste `precompiled_index_matches_a_fresh_build` acompanham. Se mudar o formato binário, aumente `FORMAT` |
| um novo bloco visual | variante em `document::Block`, renderização em `ui.rs` (e contagem de links, se tiver links) |
| uma nova tecla | `input::map_key` → `Action` → `App::handle_*` |
| um novo aviso de comando perigoso | lista `checks` em `assistant/explain.rs` |

### Estilo

- `cargo fmt` (configuração padrão) e `cargo clippy` sem avisos.
- Identificadores e comentários de código em inglês; textos exibidos ao usuário em português.
- Comentários explicam o *porquê*, não repetem o código.
- Mensagens de commit no imperativo, curtas ("Adiciona receita para liberar porta").

## Empacotando uma versão

```bash
./scripts/package.sh
```

Gera `dist/termsense-<versão>-x86_64-linux.tar.gz` (binário estático musl + `install.sh` + README) e o `.sha256`. Esse pacote instala em qualquer Linux x86_64 ou WSL sem Rust. Atualize a versão em `Cargo.toml` antes.
