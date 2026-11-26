#!/bin/sh
set -eu
export PATH=/jobs-parity/postgres-17.11/bin:/usr/bin:/bin
export PG_BIN_DIR=/jobs-parity/postgres-17.11/bin
export OPENSSL_CONF=/etc/ssl/openssl.cnf
exec /usr/bin/python3.14 -B /proof/trial.py
