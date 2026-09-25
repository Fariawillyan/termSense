# Histórico de versões

## 0.2.0 — 25/09/2026

### Novo

- **OpenShift e kubectl**: `oc` com login, projetos, `get`, `describe`, `logs`, `rsh`, `exec`, `port-forward`, `rollout`, `set`, `debug` e outros. Os tipos de recurso (`pods`, `deployment/api`, `svc`) aparecem explicados. `kubectl` é reconhecido como o mesmo comando.
- **Java e Maven**: `java`, `javac`, `jar`, `jps`, `jstack`, `jcmd` e `keytool`; `mvn` com fases e goals encadeados (`mvn clean install`), `./mvnw`, opções da JVM coladas ao valor (`-Xmx2g`, `-XX:+UseG1GC`) e o analisador de versão de class file (`class file version 65.0` = Java 21).
- **C/C++ e testes**: `g++`/`gcc`, `clang++`, `make`, `cmake`, `ctest`, `gdb`, `ldd`, `valgrind` e GoogleTest. Um binário de testes do projeto é reconhecido pelas flags `--gtest_*`.
- **Node.js e npm**: `node`, `npm` com subcomandos e apelidos (`npm i`, `npm t`), `npx`, `nvm`, `package.json`, lockfile e semver.
- **Estruturas do shell**: `if`, `for`, `while`, `case`, `[[ ]]`, `$(( ))`, heredoc, funções e expansões `${...}`, explicadas parte por parte.
- **Analisadores**: permissões (`755`, `rwxr-xr-x`, `u+x`), `umask`, cron (`*/5 * * * *`), exit codes (`exit 137`, `curl exit 7`) e scripts `sed` e `awk`.
- **Catálogo de mensagens de erro** (67 receitas): cole a mensagem e veja a causa provável e os passos.
- **Mais conteúdo**: pacotes (`apt`, `dpkg`, `dnf`), `jq`, `rsync`, firewall, `vim`, `nano`, `tmux`, WSL, Git (`reflog`, `bisect`…) e Docker (`compose up/down`…).
- `ts --check` valida os seus arquivos de conhecimento e mostra quais entradas da base eles substituem.
- `ts --init bash|zsh|fish`: Alt+H abre o `ts` com a linha que você está digitando.
- `ts --version` mostra a data da versão, e o `install.sh` avisa de qual versão para qual atualizou.

### Melhorado

- Quando não conhece algo, o `ts` diz isso em vez de mostrar um palpite: `crontab` não vira mais "Contar ocorrências". Números não são tratados como erro de digitação (`443` não é `403`).
- Busca por índice invertido, com o índice da base montado na compilação: cerca de 80 µs por tecla (antes, ~180 µs) e `ts -p` em ~9 ms (antes, ~11 ms), com mais que o dobro de conteúdo.

## 0.1.0 — 24/09/2026

Primeira versão: interface no terminal, busca, base local de Linux, shell, regex, redes, Git, Docker e SSH, explicação de comandos, calculadora CIDR e análise de regex.
