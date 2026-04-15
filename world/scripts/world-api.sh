#!/usr/bin/env bash

set -euo pipefail

# Helper functions for ma-world status API.
# Usage:
#   source scripts/world-api.sh
#   mw_set_base http://127.0.0.1:5002
#   mw_status

MW_BASE_URL="${MW_BASE_URL:-http://127.0.0.1:5002}"
MW_ADMIN_API_TOKEN="${MW_ADMIN_API_TOKEN:-}"

mw_set_token() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_set_token <token>" >&2
        return 2
    fi
    MW_ADMIN_API_TOKEN="$1"
}

mw_require_token() {
    if [[ -z "${MW_ADMIN_API_TOKEN}" ]]; then
        echo "mw: missing admin API token (set MW_ADMIN_API_TOKEN or run mw_set_token <token>)" >&2
        return 2
    fi
}

mw_auth_args() {
    mw_require_token || return 2
    printf '%s\n' "-H" "X-MA-API-Token: ${MW_ADMIN_API_TOKEN}"
}

mw_set_base() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_set_base <base_url>" >&2
        return 2
    fi
    MW_BASE_URL="$1"
}

mw_base() {
    printf '%s\n' "$MW_BASE_URL"
}

mw_status() {
    curl -s "$MW_BASE_URL/status.json"
}

mw_openapi() {
    curl -s "$MW_BASE_URL/openapi.json"
}

mw_set_slug() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_set_slug <slug>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/world/slug" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "slug=$1"
}

mw_set_kubo() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_set_kubo <kubo_url>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/world/kubo" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "kubo_url=$1"
}

mw_create_bundle() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_create_bundle <passphrase>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/bundle/create" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "passphrase=$1"
}

mw_create_bundle_value() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_create_bundle_value <passphrase>" >&2
        return 2
    fi
    mw_create_bundle "$1" | jq -r '.bundle'
}

mw_unlock() {
    if [[ $# -ne 2 ]]; then
        echo "usage: mw_unlock <passphrase> <bundle_json>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/unlock" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "passphrase=$1" \
        --data-urlencode "bundle=$2"
}

mw_unlock_from_passphrase() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_unlock_from_passphrase <passphrase>" >&2
        return 2
    fi
    local bundle
    bundle="$(mw_create_bundle_value "$1")"
    mw_unlock "$1" "$bundle"
}

mw_save() {
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/world/save" "${auth[@]}"
}

mw_save_state_cid() {
    mw_save | jq -r '.state_cid'
}

mw_save_root_cid() {
    mw_save | jq -r '.root_cid'
}

mw_load_state() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_load_state <state_cid>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/world/load" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "state_cid=$1"
}

mw_load_root() {
    if [[ $# -ne 1 ]]; then
        echo "usage: mw_load_root <root_cid>" >&2
        return 2
    fi
    local -a auth
    mapfile -t auth < <(mw_auth_args) || return 2
    curl -s -X POST "$MW_BASE_URL/world/load-root" \
        "${auth[@]}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "root_cid=$1"
}
