#!/usr/bin/env bash

set -euo pipefail

# Ops-only helper script.
# This script intentionally calls Kubo HTTP API endpoints directly via curl.
# Runtime world/agent code should use ma-core wrappers/publisher flow instead.

KUBO_API_RAW="${KUBO_API:-http://127.0.0.1:5001}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
WORLD_DIR="$(cd -- "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd -- "$WORLD_DIR/.." >/dev/null 2>&1 && pwd)"
TMP_DIR="${TMP_DIR:-$REPO_ROOT/tmp}"
DEFAULT_CID_FILE="${LANG_CID_FILE:-$WORLD_DIR/.lang_cid}"
LANG_DIR_DEFAULT="$(cd -- "$SCRIPT_DIR/../lang" >/dev/null 2>&1 && pwd)"
LANG_DIR="${1:-$LANG_DIR_DEFAULT}"

normalize_kubo_api_base() {
    local raw="$1"
    local base

    base="${raw%/}"
    if [[ -z "$base" ]]; then
        echo "error: KUBO_API is empty" >&2
        exit 2
    fi

    # Accept both base URL and /api/v0 form from env.
    if [[ "$base" == */api/v0 ]]; then
        base="${base%/api/v0}"
    fi

    if [[ "$base" != http://* && "$base" != https://* ]]; then
        echo "error: KUBO_API must start with http:// or https:// (got: $raw)" >&2
        exit 2
    fi

    printf '%s\n' "$base"
}

KUBO_API="$(normalize_kubo_api_base "$KUBO_API_RAW")"

if [[ ! -d "$LANG_DIR" ]]; then
    echo "error: language directory not found: $LANG_DIR" >&2
    exit 2
fi

if ! command -v curl >/dev/null 2>&1; then
    echo "error: curl is required" >&2
    exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 2
fi

if ! command -v mktemp >/dev/null 2>&1; then
    echo "error: mktemp is required" >&2
    exit 2
fi

mkdir -p "$TMP_DIR"

ipfs_add_file() {
    local file_path="$1"
    local hash
    hash="$(curl -sS -X POST "$KUBO_API/api/v0/add?pin=true&cid-version=1&hash=sha2-256" -F "file=@$file_path" | jq -r '.Hash')"
    if [[ -z "$hash" || "$hash" == "null" ]]; then
        echo "error: failed to add file to IPFS: $file_path" >&2
        exit 1
    fi
    printf '%s\n' "$hash"
}

ipfs_pin_cid() {
    local cid="$1"
    local mode="${2:-recursive}"
    local pinned
    pinned="$(curl -sS -X POST "$KUBO_API/api/v0/pin/add?arg=$cid&recursive=$([[ "$mode" == "recursive" ]] && echo true || echo false)" | jq -r '.Pins[0] // empty')"
    if [[ -z "$pinned" ]]; then
        echo "error: failed to pin cid locally: $cid" >&2
        exit 1
    fi
}

ipfs_dag_put_json_file() {
    local json_path="$1"
    local cid
    cid="$(curl -sS -X POST "$KUBO_API/api/v0/dag/put?store-codec=dag-cbor&input-codec=json&pin=true&hash=sha2-256" -F "file=@$json_path" | jq -r '.Cid."/"')"
    if [[ -z "$cid" || "$cid" == "null" ]]; then
        echo "error: failed to put DAG-CBOR JSON to IPFS: $json_path" >&2
        exit 1
    fi
    printf '%s\n' "$cid"
}

mapfile -t ftl_files < <(find "$LANG_DIR" -maxdepth 1 -type f -name '*.ftl' | sort)
if [[ ${#ftl_files[@]} -eq 0 ]]; then
    echo "error: no .ftl files found in $LANG_DIR" >&2
    exit 2
fi

declare -a lang_pairs
for ftl in "${ftl_files[@]}"; do
    lang_tag="$(basename "$ftl" .ftl)"
    if [[ -z "$lang_tag" ]]; then
        echo "error: invalid ftl filename: $ftl" >&2
        exit 2
    fi
    cid="$(ipfs_add_file "$ftl")"
    ipfs_pin_cid "$cid" recursive
    lang_pairs+=("$lang_tag:$cid")
done

tmp_yaml="$(mktemp -p "$TMP_DIR" lang-map.XXXXXX.yaml)"
tmp_json="$(mktemp -p "$TMP_DIR" lang-map.XXXXXX.json)"
trap 'rm -f "$tmp_yaml" "$tmp_json"' EXIT

{
    echo "lang:"
    for pair in "${lang_pairs[@]}"; do
        lang="${pair%%:*}"
        cid="${pair#*:}"
        echo "  $lang: $cid"
    done
} > "$tmp_yaml"

{
    echo "{" 
    first=1
    for pair in "${lang_pairs[@]}"; do
        lang="${pair%%:*}"
        cid="${pair#*:}"
        if [[ $first -eq 0 ]]; then
            echo ","
        fi
        first=0
        printf '  "%s": {"/": "%s"}' "$lang" "$cid"
    done
    echo
    echo "}"
} > "$tmp_json"

manifest_cid="$(ipfs_dag_put_json_file "$tmp_json")"
ipfs_pin_cid "$manifest_cid" recursive

printf '%s\n' "$manifest_cid" > "$DEFAULT_CID_FILE"

echo "Published language files from: $LANG_DIR"
for pair in "${lang_pairs[@]}"; do
    lang="${pair%%:*}"
    cid="${pair#*:}"
    echo "  $lang -> $cid"
done

echo
cat "$tmp_yaml"

echo
echo "lang map dag-cbor cid: $manifest_cid"
echo "wrote default cid file: $DEFAULT_CID_FILE"
echo "set in world: @world.lang_cid $manifest_cid"
echo "dag inspect example: ipfs dag get /ipfs/$manifest_cid/en_UK"
