#!/bin/sh
set -eu
unset ERL_AFLAGS ERL_ZFLAGS ELIXIR_ERL_OPTIONS
if [ "${JOBS_PARITY_OBAN_UNSET_INETRC:-0}" = 1 ]; then
    unset ERL_INETRC
else
    export ERL_INETRC=/proof/oban.inetrc
    test -r "$ERL_INETRC"
fi
ROOTDIR=/opt/otp
BINDIR=/opt/otp/erts-17.1/bin
EMU=beam
PROGNAME=erl
export ROOTDIR BINDIR EMU PROGNAME
exec "$BINDIR/erlexec" -noshell -elixir_root /opt/elixir \
  -pa /opt/elixir/*/ebin /opt/fixtures/*/ebin -s elixir start_cli -extra /proof/fixture.exs -- "$@"
