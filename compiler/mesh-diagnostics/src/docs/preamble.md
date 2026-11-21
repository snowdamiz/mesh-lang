# Mesh diagnostics

Start with the diagnostic's code and message. The code identifies the kind of
problem. The message describes this occurrence. Use the primary location to
find the affected code, then read any related labels and notes. The repair
describes what to change. Run `meshc explain <CODE>` for the catalog entry
and `meshc guide <topic>` for the language feature it covers.

## Diagnostic JSON v2

The diagnostic stream uses NDJSON on stderr, with one JSON object per line.
Command results go to stdout. `meshc explain --json` emits a catalog object
on stdout with its own schema.

| Fields | Meaning |
| --- | --- |
| `code`, `name`, `severity`, `category` | Stable ID, kebab-case name, `error` or `warning`, and the code's family. |
| `message`, `fix`, `notes` | The observed problem, a repair instruction that is always a string, and an array of extra details. |
| `file`, `line`, `col` | Primary location. `file` is an empty string for invocation-level diagnostics. `line` and `col` can be null. |
| `spans` | The primary span followed by related spans. Each has `file`, `start`, `end`, `label`, `line`, `col`, `end_line`, and `end_col`. A diagnostic without a span has an empty array. |
| `guide`, `docs_url` | `guide.explain` is the CLI command, `guide.skill` is the local reference and anchor, and `guide.topic` is the guide topic or null. `docs_url` links to the website entry. |
| `edits` | Suggested edits with `file`, `start`, `end`, `replacement`, and `applicability`. |

Offsets are zero-based UTF-8 bytes. Lines and columns are one-based, and
columns count Unicode scalar values. End positions are exclusive. An edit
marked `machine-applicable` can be applied as written to the source that
produced it. Review `maybe-incorrect` edits before applying them.

## Repair loop

With a compiler that supports diagnostic JSON v2, use this loop.

1. Run `meshc check --json` and read diagnostics from stderr.
2. Start with the first occurrence of each code. Use its `fix` and source
   location to make the repair.
3. Run `meshc explain <CODE>` when you need the full repair or an example.
4. Repeat the check and repair cycle until the errors clear.
5. Run `meshc lint --json` and address the lint findings.
6. Run `meshc test`.
