#!/usr/bin/env bash
# Generate a fixed self-signed code-signing certificate in the login keychain of the dedicated WeChat Mac.
# Usage: scripts/setup_signing.sh <ssh-host>
set -euo pipefail

usage() { echo "Usage: $0 <ssh-host>" >&2; exit 2; }
[ $# -ge 1 ] || usage
HOST=$1
case "$HOST" in -*|*" "*) usage ;; esac

CERT_NAME="WeChat CS Local Signing"

echo "=== setup code signing certificate on $HOST ==="

# remote script to be executed on target host (or locally if HOST is local/localhost)
REMOTE_SCRIPT=$(cat <<'EOF'
set -euo pipefail
CERT_NAME="WeChat CS Local Signing"

if security find-certificate -c "$CERT_NAME" >/dev/null 2>&1; then
  echo "Certificate '$CERT_NAME' already exists in keychain. Nothing to do."
  exit 0
fi

echo "Generating self-signed code signing certificate '$CERT_NAME'..."
TMP_DIR=$(mktemp -d -t wechat-cert)
trap 'rm -rf "$TMP_DIR"' EXIT

cat <<CNF > "$TMP_DIR/cert.cnf"
[ req ]
default_bits = 2048
prompt = no
default_md = sha256
distinguished_name = dn
x509_extensions = v3_code

[ dn ]
CN = $CERT_NAME

[ v3_code ]
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:FALSE
CNF

openssl req -x509 -new -nodes -keyout "$TMP_DIR/key.pem" -out "$TMP_DIR/cert.pem" -days 3650 -config "$TMP_DIR/cert.cnf" >/dev/null 2>&1
P12_PASS="wechat-cs-local"
openssl pkcs12 -export -out "$TMP_DIR/cert.p12" -inkey "$TMP_DIR/key.pem" -in "$TMP_DIR/cert.pem" -passout "pass:$P12_PASS" -name "$CERT_NAME" >/dev/null 2>&1

KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
if [ ! -f "$KEYCHAIN" ]; then
  KEYCHAIN="$HOME/Library/Keychains/login.keychain"
fi

security import "$TMP_DIR/cert.p12" -k "$KEYCHAIN" -P "$P12_PASS" -T /usr/bin/codesign
# 允许 codesign 访问密钥分区，避免签名时弹窗中断
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "" "$KEYCHAIN" >/dev/null 2>&1 || true

echo "Certificate '$CERT_NAME' installed into $KEYCHAIN."
EOF
)

if [ "$HOST" = "local" ] || [ "$HOST" = "127.0.0.1" ] || [ "$HOST" = "localhost" ]; then
  bash -c "$REMOTE_SCRIPT"
else
  ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" "bash -s" <<<"$REMOTE_SCRIPT"
fi
