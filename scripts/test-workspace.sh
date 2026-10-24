#!/usr/bin/env bash
# Build the cdylib before workspace tests that load it from a separate process.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."

target_dir="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
case "$(uname -s)" in
  Darwin) library="libtinymemes_module.dylib" ;;
  MINGW*|MSYS*|CYGWIN*) library="tinymemes_module.dll" ;;
  *) library="libtinymemes_module.so" ;;
esac

# Build the actual dynamic-module artifact in its production configuration.
# `--all-features` belongs on the test harness and enables `static-link`, which
# intentionally omits the dynamic ABI exports from the cdylib.
cargo build --locked --no-default-features -p tinymemes-module
module_target="$target_dir"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  module_target="$module_target/$CARGO_BUILD_TARGET"
fi
module_path="$module_target/debug/$library"
if [[ ! -f "$module_path" ]]; then
  printf 'compiled TinyMemes module is missing: %s\n' "$module_path" >&2
  exit 1
fi

mkdir -p "$target_dir/compiled-fixtures"
fixture_dir="$(mktemp -d "$target_dir/compiled-fixtures/module.XXXXXX")"
trap 'rm -rf -- "$fixture_dir"' EXIT
cp -- "$module_path" "$fixture_dir/$library"
TINYMEMES_MODULE="$fixture_dir/$library" cargo test --locked --workspace "$@"
