#!/bin/sh
# One-time setup: a self-signed code signing certificate for Blinker.app, in
# its own keychain (the login keychain is not touched).
#
# macOS remembers permissions such as Screen Recording for the app's
# signature. Signed ad hoc, every build is a different app and the switch in
# System Settings stops working; signed with this certificate, every build and
# update keeps the permission. scripts/macos-app.sh picks it up by itself.
#
# Keep a copy of the .p12 and its password (printed below): a new certificate
# makes every user allow screen recording again.
#
#   scripts/macos-signing.sh                 create the certificate
#   scripts/macos-signing.sh import FILE.p12 use an existing one (another Mac, CI)
#       with its password in BLINKER_P12_PASSWORD

set -eu

NAME="Blinker Self-Signed"
DIR="$HOME/Library/Application Support/Blinker Signing"
KEYCHAIN="$HOME/Library/Keychains/blinker-signing.keychain-db"

mkdir -p "$DIR"
chmod 700 "$DIR"

if [ -f "$KEYCHAIN" ]; then
    echo "the signing keychain already exists: $KEYCHAIN"
    security find-identity -p codesigning "$KEYCHAIN" | sed -n '1,4p'
    exit 0
fi

if [ "${1:-}" = import ]; then
    P12=${2:?usage: macos-signing.sh import FILE.p12}
    P12_PASSWORD=${BLINKER_P12_PASSWORD:?set BLINKER_P12_PASSWORD}
else
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT
    cat > "$TMP/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
    openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$TMP/cert.cnf" \
        -keyout "$TMP/key.pem" -out "$TMP/cert.pem" 2>/dev/null
    P12="$DIR/blinker-signing.p12"
    P12_PASSWORD=$(openssl rand -hex 16)
    openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
        -out "$P12" -passout "pass:$P12_PASSWORD"
    chmod 600 "$P12"
    printf '%s\n' "$P12_PASSWORD" > "$DIR/blinker-signing.p12.password"
    chmod 600 "$DIR/blinker-signing.p12.password"
fi

# the keychain's own password lives next to it: the build unlocks it with it
KEYCHAIN_PASSWORD=$(openssl rand -hex 24)
printf '%s\n' "$KEYCHAIN_PASSWORD" > "$DIR/keychain-password"
chmod 600 "$DIR/keychain-password"

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$P12" -k "$KEYCHAIN" -P "$P12_PASSWORD" -T /usr/bin/codesign >/dev/null
# lets codesign use the key without asking
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null

echo "the signing certificate \"$NAME\" is in $KEYCHAIN"
if [ "${1:-}" != import ]; then
    echo "back up these two files (they are the certificate):"
    echo "  $DIR/blinker-signing.p12"
    echo "  $DIR/blinker-signing.p12.password"
fi
