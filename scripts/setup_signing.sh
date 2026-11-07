#!/usr/bin/env bash
# One-time setup of a fixed self-signed code-signing identity on the dedicated WeChat Mac.
# Usage: scripts/setup_signing.sh <ssh-host>
#
# Why a dedicated keychain: over ssh the login keychain is locked, so codesign cannot reach a private key
# stored there. A separate keychain with its own random password (0600 file) can be unlocked by
# deploy_host.sh without the user's login password and without GUI prompts.
# Verified on the device 2026-11-08: an untrusted self-signed identity signs fine once its keychain is in the
# user search list; the designated requirement becomes `certificate leaf = H"…"`, stable across rebuilds.
# The certificate is never marked as trusted (no trust-settings change, no admin prompt).
set -euo pipefail

usage() { echo "Usage: $0 <ssh-host>" >&2; exit 2; }
[ $# -eq 1 ] || usage
HOST=$1
case "$HOST" in -*|*" "*) usage ;; esac

REMOTE_SCRIPT=$(cat <<'EOF'
set -euo pipefail
CERT_NAME="WeChat CS Local Signing"
DIR="$HOME/.wechat-rpa/signing"
KC="$DIR/wechat-cs-signing.keychain-db"
PASSF="$DIR/keychain.pass"

# Current user search list as an array: one quoted, indented path per line. Never word-split it.
read_search_list() {
  SEARCH=()
  while IFS= read -r line; do
    path=$(printf '%s' "$line" | sed -E 's/^[[:space:]]*"(.*)"[[:space:]]*$/\1/')
    [ -n "$path" ] && SEARCH+=("$path")
  done < <(security list-keychains -d user)
}

ensure_in_search_list() {
  read_search_list
  echo "search list before: ${SEARCH[*]}"
  for path in "${SEARCH[@]}"; do
    [ "$path" = "$KC" ] && { echo "signing keychain already in search list"; return 0; }
  done
  security list-keychains -d user -s "${SEARCH[@]}" "$KC"
  read_search_list
  echo "search list after:  ${SEARCH[*]}"
}

mkdir -p "$DIR"; chmod 0700 "$DIR"

if [ -f "$KC" ] && [ -f "$PASSF" ]; then
  security unlock-keychain -p "$(cat "$PASSF")" "$KC"
  if security find-identity -p codesigning "$KC" | grep -q "\"$CERT_NAME\""; then
    ensure_in_search_list
    echo "Signing identity '$CERT_NAME' already set up. Nothing to do."
    exit 0
  fi
fi

[ -e "$KC" ] && { echo "ERROR: $KC exists but has no usable identity; inspect it before re-running." >&2; exit 1; }

umask 077
openssl rand -hex 32 > "$PASSF"; chmod 0600 "$PASSF"
PASS=$(cat "$PASSF")
security create-keychain -p "$PASS" "$KC"
security set-keychain-settings "$KC"          # no auto-lock timeout
security unlock-keychain -p "$PASS" "$KC"

TMP=$(mktemp -d /tmp/wechat-cs-cert.XXXXXX)
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cert.cnf" <<CNF
[ req ]
prompt = no
distinguished_name = dn
x509_extensions = v3
[ dn ]
CN = $CERT_NAME
[ v3 ]
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:FALSE
CNF
openssl req -x509 -new -nodes -newkey rsa:2048 -days 3650 -config "$TMP/cert.cnf" \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" 2>/dev/null
P12PASS=$(openssl rand -hex 16)
openssl pkcs12 -export -out "$TMP/cert.p12" -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -passout "pass:$P12PASS" -name "$CERT_NAME" 2>/dev/null
security import "$TMP/cert.p12" -k "$KC" -P "$P12PASS" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$KC" >/dev/null

ensure_in_search_list
security find-identity -p codesigning "$KC" | grep "\"$CERT_NAME\"" \
  || { echo "ERROR: identity not found after import" >&2; exit 1; }
echo "Signing identity '$CERT_NAME' created in $KC."
EOF
)

echo "=== set up code signing on $HOST ==="
if [ "$HOST" = "local" ]; then
  bash -c "$REMOTE_SCRIPT"
else
  ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" "bash -s" <<<"$REMOTE_SCRIPT"
fi
