#!/bin/bash

# Generate self-signed certificate for WebTransport/WebSocket
# Valid for 14 days, using EC prime256v1 curve

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "$SCRIPT_DIR" || exit 1

echo "Generating self-signed certificate..."

openssl req -x509 \
    -newkey ec \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -keyout key.pem \
    -out cert.pem \
    -days 14 \
    -nodes \
    -subj "/CN=localhost"

echo "Extracting certificate digest..."

FINGERPRINT=$(openssl x509 -in cert.pem -noout -sha256 -fingerprint | \
    sed 's/^.*=//' | sed 's/://g')

echo -n "$FINGERPRINT" > digest.txt

echo "Certificate generated successfully!"
echo "Digest: $FINGERPRINT"
echo ""
echo "Files created:"
echo "  - cert.pem (certificate)"
echo "  - key.pem (private key)"
echo "  - digest.txt (SHA-256 fingerprint)"
