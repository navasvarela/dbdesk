#!/bin/sh
# Regenerates the test certificate authority and the server certificate the `postgres-tls`
# service in docker-compose.yml serves. For tests only: the server's private key is committed,
# so nothing this certificate protects is secret. The authority's key is deleted once it has
# signed, so running this again makes a new authority as well.
#
# Run from this directory: ./generate.sh
set -eu

days=36500 # About a hundred years, so the fixtures never expire under CI.

openssl req -x509 -new -nodes -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
    -keyout ca.key -out ca.crt -days "$days" -subj "/CN=dbdesk test CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign"

openssl req -new -nodes -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
    -keyout server.key -out server.csr -subj "/CN=localhost"

# rustls (webpki) checks the host against subjectAltName only, never the CN, and refuses a
# server certificate that is also a CA.
cat > server.ext <<'EXT'
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost, IP:127.0.0.1
EXT
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
    -out server.crt -days "$days" -extfile server.ext

rm server.csr server.ext ca.srl ca.key
