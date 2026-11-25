#!/usr/bin/env bash
set -euo pipefail
[[ $# == 2 && "$1" == /* && "$2" == /* && ! -e "$2" && ! -L "$2" ]]
pg_config=$1
output=$2
[[ "$("$pg_config" --version)" =~ ^PostgreSQL\ 17\.11(\ \(Homebrew\))?$ ]]
server=$("$pg_config" --includedir-server)
pkglibdir=$("$pg_config" --pkglibdir)
bindir=$("$pg_config" --bindir)
"$bindir/postgres" --version >&2
printf 'includedir-server=%s\npkglibdir=%s\nbindir=%s\n' "$server" "$pkglibdir" "$bindir" >&2
# This builds a standalone file at the controller's private path. PGXS install is unnecessary.
case "$(uname -s)" in
    Linux) link=(-shared) ;;
    Darwin) link=(-bundle -undefined dynamic_lookup) ;;
    *) exit 2 ;;
esac
read -r -a cppflags <<< "$("$pg_config" --cppflags)"
cc "${cppflags[@]}" -O2 -D_GNU_SOURCE -fPIC -Wall -Wextra -Werror -Wno-unused-parameter \
    -I "$server" -I "$("$pg_config" --includedir)/internal" \
    "${link[@]}" "$(dirname "$0")/postflush.c" -o "$output"
