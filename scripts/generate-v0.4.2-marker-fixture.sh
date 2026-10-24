#!/usr/bin/env bash
set -euo pipefail

repo="$(cd -- "$(dirname -- "$0")/.." && pwd)"
legacy="$repo/target/legacy-v0.4.2-fixture-source-$$"
test_file="$legacy/crates/tinymemes-module/tests/remix_markers.rs"
output="$repo/crates/tinymemes-module/tests/fixtures/v0.4.2-remix-marker.json"

if ! git -C "$repo" rev-parse --verify 'v0.4.2^{commit}' >/dev/null 2>&1; then
  git -C "$repo" fetch origin tag v0.4.2
fi
mkdir -p "$legacy" "$(dirname -- "$output")"
git -C "$repo" archive v0.4.2 | tar -x -C "$legacy"
cat "$repo/scripts/legacy-v0.4.2-marker-fixture-test.rs" >> "$test_file"

cd -- "$legacy"
cargo build --locked -p tinymemes-module
case "$(uname -s)" in
  Darwin) library="libtinymemes_module.dylib" ;;
  MINGW*|MSYS*|CYGWIN*) library="tinymemes_module.dll" ;;
  *) library="libtinymemes_module.so" ;;
esac
FIXTURE_OUTPUT="$output" \
TINYMEMES_MODULE="$legacy/target/debug/$library" \
  cargo test --locked \
    --manifest-path Cargo.toml \
    -p tinymemes-module --test remix_markers generate_legacy_public_marker_fixture
