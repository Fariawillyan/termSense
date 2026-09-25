# Arquitetura do TermSense

Este documento descreve como o `ts` é organizado, como uma tecla vira resultado na tela e onde cada tipo de mudança deve entrar.

## Princípios

- **Local e determinístico.** Nenhuma chamada de rede, nenhum banco de dados, nenhuma IA. Mesma entrada, mesmo resultado.
- **Somente consulta.** O único contato com o sistema é verificar se um executável existe no `PATH` (`stat`, em `system.rs`). Nada é executado.
- **Baixo acoplamento.** A UI só apresenta. Busca, regex e redes não sabem que existe um terminal.
- **Conhecimento é dado, não código.** Comandos, conceitos e receitas ficam em JSON. Um tema novo não exige mexer no núcleo.
- **Poucas dependências.** `ratatui` + `crossterm` (TUI), `serde` + `serde_json` (dados), `regex` (matching) e `unicode-width` (largura de texto).

## Camadas

```text
┌──────────────────────────────────────────────────────────────┐
│ main.rs      argumentos, terminal, loop de eventos           │
├──────────────────────────────────────────────────────────────┤
│ ui.rs        Ratatui: layout, realce, Document → linhas      │  apresentação
├──────────────────────────────────────────────────────────────┤
│ app.rs       estado: consulta, seleção, pilha de detalhes    │  aplicação
│ input.rs     tecla → Action, editor de linha UTF-8           │
├──────────────────────────────────────────────────────────────┤
│ assistant/   orquestra: modos, sugestões, páginas, explicação│  núcleo
├──────────────┬──────────────┬──────────────┬─────────────────┤
│ search/      │ regex/       │ networking/  │ document.rs     │  serviços
│ tokenizer    │ parser       │ analyzer     │ modelo neutro   │
│ context      │ analyzer     │ knowledge    │ de documento    │
│ engine       │ matcher      │              │                 │
│ ranking      │              │              │                 │
├──────────────┴──────────────┴──────────────┴─────────────────┤
│ knowledge/   model, loader, repository, template             │  dados
│ knowledge/*.json (embutidos com include_str!)                │
└──────────────────────────────────────────────────────────────┘
```

Regra de dependência: cada camada só conhece as de baixo. `regex/` e `networking/` não dependem de `search/`, da UI nem uns dos outros, com uma exceção: `networking` usa `knowledge::Vars` para preencher templates. `document.rs` não depende de nada.

## Fluxo de uma tecla

```text
KEY EVENT (crossterm, leitura bloqueante: zero CPU parado)
   ↓ input::map_key          → Action
   ↓ App::handle             → edita a linha (LineEditor)
   ↓ Assistant::respond      → detecta o modo e monta sugestões
       ├─ networking::scan   → portas/hosts viram variáveis de template
       ├─ search::context    → tokenize + papéis semânticos
       ├─ SearchEngine       → hits ranqueados
       └─ regex / pages      → análises (documentos)
   ↓ App::update_preview     → Assistant::preview(sugestão selecionada)
   ↓ ui::draw                → Document → linhas estilizadas
RENDER
```

Tudo roda de forma síncrona a cada tecla. Medido em release, com 253 entradas, `respond` + pré-visualização leva **~0,2 ms por tecla**. `ts --print` completo (carregar a base, responder e imprimir) leva ~10 ms e ~6 MB de RAM. O custo dominante é desenhar o terminal. Para medir de novo:

```bash
cargo test --release -- --ignored keystroke_latency --nocapture
```

## Módulos

### `knowledge/`

| Arquivo | Papel |
|---|---|
| `model.rs` | Tipos serde: `Entry` (command, concept ou recipe), `CommandOption`, `Argument`, `Example`, `Step`, `Section`, `Category`, `KnowledgeFile`. `deny_unknown_fields` pega erros de digitação no JSON. |
| `loader.rs` | `EMBEDDED` lista os JSON compilados no binário. `load()` junta a base embutida e os arquivos do usuário. Um id repetido substitui o anterior. Erro em arquivo do usuário vira aviso; erro na base embutida é fatal (e os testes o impedem). |
| `repository.rs` | Acesso somente leitura, indexado por posição. Mapas por id, por comando de topo e por pai (subcomandos). `concept()` anota valores como `POST` e `MX`. |
| `template.rs` | Placeholders `{{nome}}` e `{{nome:padrão}}` nos comandos. Só identificadores minúsculos contam, então `awk '{print $1}'` e `docker inspect -f '{{.State}}'` passam intactos. |

Uma única forma de `Entry` serve para tudo. A diferença entre comando, conceito e receita é o campo `kind` mais os campos preenchidos: receitas usam `steps` e `examples`; comandos usam `options`, `arguments` e `usage`. Subcomandos são comandos com `parent`.

### `search/`

| Arquivo | Papel |
|---|---|
| `tokenizer.rs` | **Lexer de shell**: aspas, escapes, `$(...)`, crases, `<(...)`, pipes, `&&`, `;`, redirecionamentos com descritor (`2>&1`). Tolera entrada incompleta (aspa aberta). Classifica cada token pela forma: `COMMAND`, `OPTION`, `STRING`, `PATH`, `URL`, `HOST`, `NUMBER`, `VARIABLE`, `SUBSTITUTION`, `GLOB`, `ASSIGNMENT`, `PIPE`, `OPERATOR`, `REDIRECT`, `WORD`. Também normaliza texto de busca (minúsculas, sem acento) e remove stop words em português e inglês. |
| `context.rs` | **Análise semântica** de uma linha usando a base. Dá a cada token um `Role`: comando, subcomando, opção conhecida, grupo de flags (`-rin` → `-r -i -n`), valor de opção (`-X POST`), argumento posicional com tipo (`PADRÃO` do grep é regex) e redirecionamentos. Entende wrappers (`sudo`, `nohup`, `xargs`, `env`): o comando embrulhado vira o contexto. Também calcula o `Cursor` (o que está sendo digitado) e os candidatos de completar (opções, subcomandos e exemplos compatíveis com as flags já digitadas). |
| `engine.rs` | Índice pré-normalizado por campo e pontuação. |
| `ranking.rs` | Níveis de match e ordem total para desempate. |

**Ranking.** Primeiro a consulta inteira é comparada ao nome, em níveis:

```text
exato 10000 > prefixo 8000 > palavra do nome 6000 > trecho 5000 > erro de digitação 4500
> alias 4000 > tag 3000 > frase de alias contida na consulta ~2000+
```

Depois, cada termo recebe o peso do melhor campo em que aparece (nome > alias > tag > descrição > opções > exemplos > relacionados). Palavras parecidas contam com peso menor (`processo`/`processos`, `testar`/`teste`), e a cobertura parcial é penalizada ao quadrado. Consultas de mais de um termo precisam casar pelo menos metade dos termos. Empates são resolvidos por tipo (comando < receita < conceito), nome mais curto, ordem alfabética e índice, o que dá uma ordem total e, portanto, determinística.

Com duas letras, só valem prefixo e erro de digitação (mesma primeira letra, distância de edição 1). Os níveis "palavra" e "trecho" exigem três caracteres, para evitar ruído.

### `regex/`

| Arquivo | Papel |
|---|---|
| `mod.rs` | Trait `RegexAnalyzer { parse, analyze, suggest, explain }`. O resto do sistema depende do trait. |
| `parser.rs` | Padrão → **IR plana** de `RegexToken` (`START_ANCHOR`, `CHARACTER_CLASS`, `QUANTIFIER`, `GROUP_OPEN`…) com profundidade, spans e detalhes (classe, limites do quantificador, tipo de grupo). É leniente: nunca falha, e problemas viram tokens `INVALID`. Divide `abc+` em `ab`, `c`, `+`. |
| `analyzer.rs` | Explica cada token, monta uma árvore (alternativas → sequência de itens quantificados) para a **interpretação em português**, com gênero gramatical, e gera **exemplos**. |
| `matcher.rs` | Única porta para a crate `regex`, com limite de tamanho do programa compilado. |

Exemplos que casam e que não casam são *gerados* a partir da estrutura: variações de repetição, caracteres representativos de cada classe, mutações da amostra base e uma lista fixa de sondas. Depois são *verificados* com a crate `regex`, então nunca aparece um exemplo errado. Padrões que a crate não suporta (lookaround, retrorreferência) continuam explicados token a token, com uma nota sobre `grep -P`.

### `networking/`

| Arquivo | Papel |
|---|---|
| `analyzer.rs` | `scan()` extrai da consulta portas (`porta 8080`, `:8080`, `host:8080`), hosts, URLs e blocos CIDR. Números de porta explícitos viram parâmetros e saem dos termos de busca. Também faz a matemática de sub-rede IPv4 (máscara, wildcard, total × utilizáveis com as exceções /31 e /32, rede, broadcast, faixa) e calcula a rede IPv6. |
| `knowledge.rs` | Tabelas usadas em cálculos: portas conhecidas, faixas de porta da IANA e blocos IPv4 especiais (RFC 1918, loopback, link-local, CGNAT, documentação…). Os conceitos de rede em si ficam no JSON. |

As variáveis extraídas (`port`, `host`, `url`) preenchem os templates das receitas. Por isso `quem usa a porta 5432` mostra `ss -ltnp | grep ':5432'`.

### `document.rs`

Modelo de documento neutro: `Document { title, subtitle, blocks }`, com os blocos `Heading`, `Paragraph`, `Code` (com legenda e link), `Table`, `List`, `Steps`, `Tree`, `Flow` e `Note`, e tons semânticos (`Info`, `Warning`, `Danger`…). `Link` aponta para uma entrada, uma linha de comando a explicar ou uma categoria. `Document::links()` define a ordem de navegação com Tab. O renderer da UI percorre os blocos na mesma ordem e um teste garante que as duas contagens batem.

Analisadores e páginas produzem `Document`. A UI decide cores, quebra de linha e rolagem. O mesmo documento serve para o TUI e para `ts --print`.

### `assistant/`

O núcleo da aplicação: transforma o texto digitado em uma `Response { mode, suggestions }`.

| Modo | Quando | Sugestões |
|---|---|---|
| `Home` | entrada vazia | guia de uso + categorias |
| `Regex` | começa com `regex` | análise, completar a regex, padrões prontos, conceitos usados |
| `Network` | a entrada inteira é IP ou CIDR | calculadora/classificação + receitas e conceitos |
| `Command` | comando conhecido seguido de algo, ou há operadores de shell | explicação, subcomandos/opções/exemplos conforme o cursor, documentação dos comandos da linha, relacionados |
| `Search` | o resto (inclusive linguagem natural) | hits ranqueados; as 3 primeiras receitas são expandidas em comandos; análise de porta/CIDR depois do primeiro grupo |

| Arquivo | Papel |
|---|---|
| `suggestion.rs` | `Suggestion` uniforme: tipo, título, subtítulo, `completion` (linha inteira após Tab) e `Target` (link, opção ou documento pronto). |
| `pages.rs` | Documentos de entrada, categoria, opção, regex, CIDR, IP, porta e boas-vindas. Cada página segue a spec: descrição, uso, disponibilidade, opções, argumentos, passos, exemplos, seções e árvore de relacionamentos. |
| `explain.rs` | Explicador de linha de comando: tabela de partes (papel + descrição + anotações de porta/URL/regex/conceito), análise léxica, fluxo de dados do pipeline e avisos para padrões perigosos (`rm -rf /`, `curl \| sh`, `chmod -R 777`, `git push --force`…). |

A disponibilidade dos comandos (`which`) é consultada sob demanda e guardada em cache (`RefCell<HashMap>`). No WSL, os diretórios `/mnt/c` do `PATH` são lentos para consultar.

### `app.rs`, `input.rs` e `ui.rs`

- `App` é uma máquina de estados pura: visão de busca, ou pilha de `DetailView` com rolagem e link focado. Toda a interação é testável sem terminal.
- `input.rs` mapeia teclas para `Action` (independente da visão) e implementa um `LineEditor` com cursor em fronteira de caractere UTF-8.
- `ui.rs` faz o layout responsivo: lado a lado a partir de 100 colunas, empilhado abaixo disso. Também realça a sintaxe da entrada com o próprio tokenizer (ou com o parser de regex), renderiza documentos com quebra de linha por largura Unicode e mantém o link focado visível. Os testes usam o `TestBackend` do Ratatui em vários tamanhos, inclusive 10×5.

### `config.rs` e `system.rs`

- `config.rs` resolve `$XDG_CONFIG_HOME/termsense` (ou `~/.config/termsense`) e carrega `knowledge/*.json` de lá. O `config.toml` (tema, atalhos, ranking, layout) está **reservado** para uma versão futura e hoje não é lido.
- `system.rs` implementa o `which()` somente leitura.

## Extensibilidade planejada

A arquitetura foi pensada para receber estas evoluções sem reescrever o núcleo. **Nenhuma delas está implementada no MVP.**

| Evolução | Onde entra |
|---|---|
| IA local (Ollama) | Um trait `AssistantProvider` consumido pelo `Assistant`, ao lado das fontes locais. O motor de busca continua sem depender de IA: a IA recebe o contexto já resolvido (hits, análise da linha) e devolve um `Document`. |
| Histórico e favoritos | Novo módulo de armazenamento em `~/.local/share/termsense`, lido pelo `Assistant` na `Home` e como sinal extra de ranking. |
| Busca semântica local | Outra estratégia de pontuação em `search/`, combinada com o ranking lexical sem substituí-lo. |
| Modo treinamento | Novo `Mode` e uma visão na UI. As perguntas podem ser geradas da própria base (opções ↔ descrições, receitas ↔ comandos). |
| Integração com shell | Um widget Bash/Zsh/Fish que chama `ts --print` ou um futuro modo de seleção que devolve a linha escolhida no stdout. |
| Novos temas | Só JSON: novo `category` em `categories.json` e novas entradas. |
| Novos analisadores | Uma função no `Assistant` que devolve `Suggestion::analysis(...)`, no mesmo molde de `network_insights`. |

## Segurança

- Nenhum caminho do código executa processos. Não existem `std::process::Command` nem chamadas de rede.
- Comandos perigosos são tratados como documentação: aparecem com avisos (`Tone::Danger`) e nunca são executados.
- A única escrita em disco é a feita pelo `install.sh`, que o usuário executa explicitamente.
