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

if [ -f src/models/file_info.rs ]; then
  sed -i.bak 's/pub size: i32/pub size: i64/; s/size: i32,/size: i64,/' src/models/file_info.rs
  rm src/models/file_info.rs.bak
fi

for response_model in api_key_list api_key_response organization_role; do
  response_file="src/models/${response_model}.rs"
  if [ -f "$response_file" ]; then
    sed -i.bak 's/#\[serde(rename = "unknown_default_open_api")\]/#[serde(other)]/' "$response_file"
    rm "${response_file}.bak"
  fi
done

# The spec inlines the sandbox class enum in some models, so the generator emits
# duplicate SandboxClass types. Alias each copy to the shared models::SandboxClass
# so one type flows through the API, and stop if a copy's variants ever diverge.
enum_body='print $1 if /pub enum SandboxClass \{(.*?)\n\}/s'
if [ -f src/models/sandbox_class.rs ]; then
  shared_variants=$(perl -0ne "$enum_body" src/models/sandbox_class.rs)
  for model in sandbox snapshot_dto; do
    model_file="src/models/${model}.rs"
    [ -f "$model_file" ] || continue
    inline_variants=$(perl -0ne "$enum_body" "$model_file")
    [ -n "$inline_variants" ] || continue
    if [ "$inline_variants" != "$shared_variants" ]; then
      echo "error: $model_file declares SandboxClass variants that differ from models::SandboxClass" >&2
      exit 1
    fi
    perl -0pi -e 's{(?:///[^\n]*\n)*#\[derive\([^\n]*\)\]\npub enum SandboxClass \{.*?\n\}\n\nimpl Default for SandboxClass \{.*?\n\}\n}{pub use crate::models::SandboxClass;\n}s' "$model_file"
  done
fi

# Add module-level clippy allows to lib.rs if not already present
if ! grep -q "clippy::all" src/lib.rs; then
  HEADER='#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(non_camel_case_types)]
'
  TEMP=$(mktemp)
  echo "$HEADER" > "$TEMP"
  cat src/lib.rs >> "$TEMP"
  mv "$TEMP" src/lib.rs
fi

# Re-format after modifications
cargo fmt --manifest-path Cargo.toml 2>/dev/null || true
