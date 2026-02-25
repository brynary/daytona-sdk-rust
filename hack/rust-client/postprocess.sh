#!/usr/bin/env bash
set -euo pipefail

# Post-process generated Rust API client
# Usage: bash hack/rust-client/postprocess.sh <crate-directory>

CRATE_DIR="${1:?Usage: postprocess.sh <crate-directory>}"

cd "$CRATE_DIR"

# Format generated code
cargo fmt --manifest-path Cargo.toml 2>/dev/null || true

# Fix common clippy warnings in generated code
# Allow clippy warnings that are unavoidable in generated code
if [ ! -f src/lib.rs ]; then
  echo "Warning: src/lib.rs not found in $CRATE_DIR, skipping clippy attribute injection"
  exit 0
fi

# Add module-level clippy allows to lib.rs if not already present
if ! grep -q "clippy::all" src/lib.rs; then
  HEADER='#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(unused_imports)]
#![allow(dead_code)]
'
  TEMP=$(mktemp)
  echo "$HEADER" > "$TEMP"
  cat src/lib.rs >> "$TEMP"
  mv "$TEMP" src/lib.rs
fi

# Re-format after modifications
cargo fmt --manifest-path Cargo.toml 2>/dev/null || true
