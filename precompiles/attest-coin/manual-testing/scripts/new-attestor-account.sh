#!/usr/bin/env bash
# Step 5.3 — generate the attestor operator account (sr25519) and record it in .env.
#
# Replaces the ATTESTOR_SS58 / ATTESTOR_SEED lines in place rather than appending, so re-running
# this does not leave a second, stale copy of the pair behind. dotenv and `source` both take the
# last occurrence, which makes an accidental duplicate look like it worked while a human editing
# the first copy sees no effect.
set -euo pipefail

cd "$(dirname "$0")/.."
ENV_FILE=".env"

command -v subkey >/dev/null || { echo "subkey not found — install it first (cargo install subkey)" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq not found — install it first (brew install jq)" >&2; exit 1; }
[ -f "$ENV_FILE" ] || { echo "$ENV_FILE not found — run from precompiles/attest-coin/manual-testing" >&2; exit 1; }

OUT=$(subkey generate --output-type json)
SS58=$(echo "$OUT" | jq -r .ss58Address)
SEED=$(echo "$OUT" | jq -r .secretSeed)

# Refuse to write empty values; a failed subkey/jq must not look like success.
[ -n "$SS58" ] && [ "$SS58" != "null" ] || { echo "subkey produced no ss58Address" >&2; exit 1; }
[ -n "$SEED" ] && [ "$SEED" != "null" ] || { echo "subkey produced no secretSeed" >&2; exit 1; }

set_env_key() {
    local key="$1" value="$2"
    if grep -qE "^${key}=" "$ENV_FILE"; then
        # Replace every occurrence, collapsing any duplicates a previous append left behind.
        awk -v k="$key" -v v="$value" '
            $0 ~ "^" k "=" { if (!seen) { print k "=" v; seen = 1 } ; next }
            { print }
        ' "$ENV_FILE" > "$ENV_FILE.tmp" && mv "$ENV_FILE.tmp" "$ENV_FILE"
    else
        printf '%s=%s\n' "$key" "$value" >> "$ENV_FILE"
    fi
}

set_env_key ATTESTOR_SS58 "$SS58"
set_env_key ATTESTOR_SEED "$SEED"

grep -E '^ATTESTOR_' "$ENV_FILE"
