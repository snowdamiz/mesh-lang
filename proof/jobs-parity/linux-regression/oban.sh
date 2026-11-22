#!/bin/sh
set -eu
ROOTDIR=/opt/otp
BINDIR=/opt/otp/erts-17.1/bin
EMU=beam
PROGNAME=erl
export ROOTDIR BINDIR EMU PROGNAME
exec "$BINDIR/erlexec" +S 2:2 -noshell -elixir_root /opt/elixir \
  -pa /opt/elixir/*/ebin /opt/fixtures/*/ebin -s elixir start_cli -extra /proof/fixture.exs -- "$@"
