"""Parse the stock pg_config fingerprint without relying on whitespace padding."""
import shlex
import sys


def verify(prefix, flags):
    if shlex.split(flags) != [f"--prefix={prefix}", "--with-openssl"]:
        raise ValueError(f"unexpected stock PostgreSQL configure flags: {flags!r}")


if __name__ == "__main__":
    verify(*sys.argv[1:])
