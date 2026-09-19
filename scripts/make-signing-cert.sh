#!/usr/bin/env bash
# One-off: makes the self-signed code-signing identity release.yml signs the
# macOS app with, and stores it as repo secrets.
#
# Why: an ad-hoc signature is its own hash, so every release is a "new app" to
# macOS and it forgets every Files & Folders grant. A fixed certificate makes
# the grant stick across updates. Not a Developer ID: Gatekeeper still warns on
# first open, as it does today. Keep the .p12 somewhere safe; losing it means
# users are prompted once more after the next release.
set -euo pipefail

name="Zyplus Self-Signed"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
pass=$(openssl rand -hex 16)

cat >"$dir/cfg" <<EOF
[req]
distinguished_name=dn
x509_extensions=ext
prompt=no
[dn]
CN=$name
[ext]
basicConstraints=critical,CA:false
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,codeSigning
EOF

openssl req -x509 -newkey rsa:2048 -nodes -days 7300 -config "$dir/cfg" \
  -keyout "$dir/key.pem" -out "$dir/cert.pem"
# -legacy: macOS `security import` cannot read OpenSSL 3's default p12 cipher.
openssl pkcs12 -export -legacy -inkey "$dir/key.pem" -in "$dir/cert.pem" \
  -out "$dir/cert.p12" -passout "pass:$pass"

base64 <"$dir/cert.p12" | tr -d '\n' | gh secret set MACOS_SIGNING_CERT
printf %s "$pass" | gh secret set MACOS_SIGNING_CERT_PASSWORD
cp "$dir/cert.p12" "./zyplus-signing.p12"
echo "Secrets set. Backup: ./zyplus-signing.p12 (password: $pass) — move it out of the repo."
