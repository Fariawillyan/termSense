#!/usr/bin/env bash
# Instala o TermSense (ts) em ~/.local/bin (ou em $INSTALL_DIR).
#
# Funciona de dois jeitos:
#   - dentro do repositório: compila com cargo e instala;
#   - dentro do pacote pré-compilado (termsense-*.tar.gz): só copia o binário,
#     sem precisar de Rust.
#
# Não altera arquivos do shell: apenas informa se o diretório não está no PATH.
set -euo pipefail

INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

info() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mxx\033[0m %s\n' "$*" >&2; exit 1; }

if [ -f "$SCRIPT_DIR/ts" ] && [ ! -f "$SCRIPT_DIR/Cargo.toml" ]; then
    # Pacote pré-compilado.
    info "Executável pré-compilado encontrado: instalando sem compilar."
    BIN="$SCRIPT_DIR/ts"
else
    # 1. Rust/Cargo (aceita a instalação do rustup mesmo fora do PATH)
    if ! command -v cargo >/dev/null 2>&1; then
        if [ -x "$HOME/.cargo/bin/cargo" ]; then
            export PATH="$HOME/.cargo/bin:$PATH"
        else
            fail "cargo não encontrado. Instale o Rust: https://rustup.rs (ou use o pacote pré-compilado)"
        fi
    fi
    info "Rust: $(cargo --version)"

    # 2. Compilação release
    info "Compilando (cargo build --release)..."
    cargo build --release --locked
    BIN="target/release/ts"
fi

# 3. Diretório de destino
mkdir -p "$INSTALL_DIR"

# 4. Instalação
install -m 0755 "$BIN" "$INSTALL_DIR/ts"
info "Instalado em $INSTALL_DIR/ts ($("$INSTALL_DIR/ts" --version))"

# 5. PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        found="$(command -v ts || true)"
        if [ "$found" != "$INSTALL_DIR/ts" ]; then
            warn "Outro 'ts' vem antes no PATH: $found (ex.: moreutils). Use $INSTALL_DIR/ts ou ajuste o PATH."
        else
            info "Pronto! Execute: ts"
        fi
        ;;
    *)
        warn "$INSTALL_DIR não está no PATH."
        echo "    Adicione ao ~/.bashrc (ou ~/.zshrc) e abra um novo terminal:"
        echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
        echo "    Enquanto isso, execute: $INSTALL_DIR/ts"
        ;;
esac
