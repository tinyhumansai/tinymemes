#!/usr/bin/env bash
set -euo pipefail

blocked='^(rusqlite|libsqlite3-sys|mongodb|bson|aws-sdk-s3|lapin|amq-protocol|sqlx|redb|sled|rocksdb|tinystoragedrivers-(document|kv|blob|queue|secrets|connect)|tinystoragedrivers)$'

check_tree() {
  package="$1"
  names="$2"
  if [[ -z "$names" ]] || ! printf '%s\n' "$names" | grep -Fxq "$package"; then
    printf 'failed closed: cargo tree for %s did not contain its root package\n' "$package" >&2
    return 1
  fi
  offender="$(printf '%s\n' "$names" | grep -Ex "$blocked" || true)"
  if [[ -n "$offender" ]]; then
    printf '%s has a normal/build dependency on a storage engine: %s\n' "$package" "$offender" >&2
    return 1
  fi
}

library_packages() {
  python3 -c '
import json, sys
data = json.load(sys.stdin)
members = set(data["workspace_members"])
library_kinds = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
for package in data["packages"]:
    if package["id"] in members and any(library_kinds.intersection(target["kind"]) for target in package["targets"]):
        print(package["name"])
'
}

if [[ "${1:-}" == "--self-test" ]]; then
  check_tree tinymemes $'tinymemes\nserde\ntinystoragedrivers-api\ntinystoragedrivers-types'
  if check_tree tinymemes $'tinymemes\nserde\ntinystoragedrivers-document'; then
    printf 'self-test failed: a document engine was accepted\n' >&2
    exit 1
  fi
  if check_tree tinymemes $'tinymemes\nserde\ntinystoragedrivers'; then
    printf 'self-test failed: the facade package was accepted\n' >&2
    exit 1
  fi
  if check_tree tinymemes $'serde\ntinystoragedrivers-api'; then
    printf 'self-test failed: a missing root package was accepted\n' >&2
    exit 1
  fi
  metadata='{"workspace_members":["dylib-id","binary-id"],"packages":[{"id":"dylib-id","name":"dylib-only","targets":[{"kind":["dylib"]}]},{"id":"binary-id","name":"binary-only","targets":[{"kind":["bin"]}]},{"id":"external-id","name":"external-dylib","targets":[{"kind":["dylib"]}]}]}'
  selected="$(printf '%s' "$metadata" | library_packages)"
  if [[ "$selected" != "dylib-only" ]]; then
    printf 'self-test failed: library enumeration did not select only the workspace dylib target\n' >&2
    exit 1
  fi
  printf 'storage engine tree gate self-test passed\n'
  exit 0
fi

metadata="$(cargo metadata --format-version 1 --no-deps)"
packages="$(printf '%s' "$metadata" | library_packages)"
if [[ -z "$packages" ]]; then
  printf 'failed closed: workspace metadata contained no native library packages\n' >&2
  exit 1
fi

while IFS= read -r package; do
  [[ -n "$package" ]] || continue
  tree="$(cargo tree -p "$package" --all-features -e normal,build --prefix none)"
  names="$(printf '%s\n' "$tree" | awk 'NF {print $1}')"
  check_tree "$package" "$names"
done <<< "$packages"

printf 'storage engine dependency boundary passed for all native library packages\n'
