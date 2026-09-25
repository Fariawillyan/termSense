#!/usr/bin/env bash
# Gera o pacote pré-compilado para instalar sem clonar o repositório:
#   dist/termsense-<versão>-x86_64-linux.tar.gz
#
# O binário é estático (musl): roda em qualquer Linux x86_64, inclusive WSL,
# sem depender da glibc da distribuição. A base de conhecimento vai embutida.
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET="x86_64-unknown-linux-musl"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
NAME="termsense-$VERSION-x86_64-linux"

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
command -v rustup >/dev/null 2>&1 && rustup target add "$TARGET" >/dev/null

cargo build --release --locked --target "$TARGET"

rm -rf "dist/$NAME" "dist/$NAME.tar.gz"
mkdir -p "dist/$NAME"
cp "target/$TARGET/release/ts" install.sh README.md "dist/$NAME/"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "dist/$NAME"
(cd dist && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")

echo "Pacote: dist/$NAME.tar.gz"
cat "dist/$NAME.tar.gz.sha256"
