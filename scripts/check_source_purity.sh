#!/usr/bin/env bash
# CI guard mirroring src-tauri/tests/source_purity.rs: source modules must not
# import host IO. Fails loudly with the offending lines.
set -euo pipefail

cd "$(dirname "$0")/.."
SOURCES_DIR="src-tauri/src/sources"
TOKENS='std::fs|std::process|std::net|std::env|tokio::fs|tokio::process|tokio::net|reqwest|rusqlite|keyring|tauri|Command::new'

if hits=$(grep -RnE "$TOKENS" "$SOURCES_DIR" --include='*.rs' | grep -vE '^\S+:[0-9]+:\s*//'); then
  echo "source purity violation(s):"
  echo "$hits"
  exit 1
fi
echo "sources are pure"
