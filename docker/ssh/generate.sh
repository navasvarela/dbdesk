#!/bin/sh
# Regenerates the keys of the `ssh` service in docker-compose.yml and of the clients the tests
# sign in with. For tests only: every private key is committed, so nothing they protect is
# secret. `client_ed25519_encrypted`'s passphrase is `dbdesk`.
#
# Run from this directory: ./generate.sh
set -eu

rm -f ssh_host_ed25519_key ssh_host_ed25519_key.pub client_ed25519 client_ed25519_encrypted
ssh-keygen -q -t ed25519 -N '' -C 'dbdesk test SSH server' -f ssh_host_ed25519_key
ssh-keygen -q -t ed25519 -N '' -C 'dbdesk test client' -f client_ed25519
ssh-keygen -q -t ed25519 -N 'dbdesk' -C 'dbdesk test client, encrypted' -f client_ed25519_encrypted
cat client_ed25519.pub client_ed25519_encrypted.pub > authorized_keys
rm client_ed25519.pub client_ed25519_encrypted.pub
