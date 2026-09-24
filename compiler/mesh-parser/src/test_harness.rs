//! `meshc test` files as Mesh programs.
//!
//! A `.test.mpl` file writes its tests as `test("label") do ... end` blocks,
//! grouped by `describe` with `setup` and `teardown`. `meshc test` compiles
//! it after `preprocess_test_source`.

use std::ops::Range;

use crate::{SyntaxKind, SyntaxNode};

/// Preprocess a .test.mpl source file into a valid Mesh program. The file is
/// rewritten in place, so each of its lines keeps its line and column and a
/// diagnostic points at the test as written:
///
/// - `test("label") do` becomes `fn __test_body_N() do`.
/// - `describe("group") do` becomes `fn __test_describe_N(__case :: Int) do`. Its
///   `setup` lines run first, so what they bind is in scope for the tests and
///   the teardown; each `test` in it becomes
///   `if __case == I do test_run_body(fn() do ... end) end`; a `teardown`
///   becomes a closure run after the test, whether the test passed or not. A
///   `describe` in it becomes `if __case >= I && __case < J do`, over the
///   cases of its tests, which see both setups and run both teardowns.
/// - `assert_receive PATTERN[, TIMEOUT]` becomes a `receive` on its line.
/// - A `fn main()` is appended that runs each test (`test_begin`,
///   `test_run_body`, `test_end`) and then `test_summary`.
///
/// A file that does not parse is returned as it is, for the build to report
/// its errors where they are.
pub fn preprocess_test_source(source: &str) -> Result<String, String> {
    let parse = crate::parse(source);
    if !parse.errors().is_empty() {
        return Ok(source.to_string());
    }

    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    // Each test: its label, and the call in `main` that runs it.
    let mut tests: Vec<(String, String)> = Vec::new();
    let mut describes = 0;
    for node in parse.syntax().children() {
        let Some(call) = BlockCall::of(&node) else {
            continue;
        };
        match call.name.as_str() {
            "test" => {
                let n = tests.len();
                call.rewrite(&mut edits, &format!("fn __test_body_{n}() do"), "end");
                tests.push((call.label("unnamed"), format!("__test_body_{n}()")));
            }
            "describe" => {
                let d = describes;
                describes += 1;
                let mut group = Group {
                    source,
                    function: d,
                    cases: 0,
                    edits: &mut edits,
                    tests: &mut tests,
                };
                let teardown = group.describe(&call, &call.label("describe"))?;
                call.rewrite(
                    &mut edits,
                    &format!("fn __test_describe_{d}(__case :: Int) do"),
                    teardown_end(teardown),
                );
            }
            _ => continue,
        }
        // An `assert_receive` becomes a `receive` on its own lines.
        for node in call.block.descendants() {
            if node.kind() == SyntaxKind::ASSERT_RECEIVE_EXPR {
                edits.push((range_of(&node), expand_assert_receive(&node)));
            }
        }
    }

    if tests.is_empty() {
        // Not a test file or no test blocks — pass through unchanged.
        return Ok(source.to_string());
    }

    let mut out = source.to_string();
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in edits {
        out.replace_range(range, &text);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }

    // The harness, after every line of the file; `test_end` counts the test
    // once. The label is the test's string literal, as written.
    out.push_str("\nfn main() do\n");
    for (label, run) in &tests {
        out.push_str(&format!(
            "  test_cleanup_actors()\n  test_begin(\"{label}\")\n  test_run_body(fn() do {run} end)\n  test_end()\n"
        ));
    }
    // Pass 0 for elapsed_ms; accurate timing is cosmetic and can be added later.
    out.push_str("  test_summary(test_pass_count(), test_fail_count(), 0)\n");
    out.push_str("end\n");
    Ok(out)
}

/// The tests of one top-level `describe`: cases of one function.
struct Group<'a> {
    source: &'a str,
    /// The `__test_describe_N` function's N.
    function: usize,
    /// Cases numbered so far.
    cases: usize,
    edits: &'a mut Vec<(Range<usize>, String)>,
    tests: &'a mut Vec<(String, String)>,
}

impl Group<'_> {
    /// Rewrite the setup, teardown, tests and nested describes of the
    /// describe `call`, labelled `label`, and say whether it has a teardown.
    fn describe(&mut self, call: &BlockCall, label: &str) -> Result<bool, String> {
        let mut teardown = false;
        let mut has_tests = false;
        for stmt in call.block.children() {
            let Some(inner) = BlockCall::of(&stmt) else {
                continue;
            };
            let line = line_of(self.source, inner.header.start);
            match inner.name.as_str() {
                "setup" if has_tests => {
                    return Err(format!(
                        "line {line}: `setup` must come before the tests of its describe"
                    ));
                }
                "setup" => inner.rewrite(self.edits, "", ""),
                "teardown" if teardown => {
                    return Err(format!("line {line}: a describe has one `teardown`"));
                }
                "teardown" => {
                    teardown = true;
                    inner.rewrite(self.edits, "let __teardown = fn() do", "end");
                }
                "test" => {
                    has_tests = true;
                    let case = self.cases;
                    inner.rewrite(
                        self.edits,
                        &format!("if __case == {case} do test_run_body(fn() do"),
                        "end) end",
                    );
                    self.tests.push((
                        format!("{label} > {}", inner.label("unnamed")),
                        format!("__test_describe_{}({case})", self.function),
                    ));
                    self.cases += 1;
                }
                "describe" => {
                    has_tests = true;
                    let first = self.cases;
                    let nested = format!("{label} > {}", inner.label("describe"));
                    let teardown = self.describe(&inner, &nested)?;
                    let header = format!("if __case >= {first} && __case < {} do", self.cases);
                    inner.rewrite(self.edits, &header, teardown_end(teardown));
                }
                _ => {}
            }
        }
        Ok(teardown)
    }
}

/// A describe's `end`: its teardown runs after the test.
fn teardown_end(teardown: bool) -> &'static str {
    if teardown {
        "test_run_body(__teardown) end"
    } else {
        "end"
    }
}

/// A `name(...) do ... end` or `name do ... end` statement, as `test`,
/// `describe`, `setup` and `teardown` are written.
struct BlockCall {
    name: String,
    call: SyntaxNode,
    /// From the call's start through its `do`.
    header: Range<usize>,
    /// The statements of its `do` block.
    block: SyntaxNode,
    /// The block's `end`.
    end: Range<usize>,
}

impl BlockCall {
    fn of(node: &SyntaxNode) -> Option<Self> {
        if node.kind() != SyntaxKind::CALL_EXPR {
            return None;
        }
        let name = node
            .children()
            .find(|child| child.kind() == SyntaxKind::NAME_REF)?
            .text()
            .to_string();
        let closure = node
            .children()
            .find(|child| child.kind() == SyntaxKind::TRAILING_CLOSURE)?;
        let block = closure
            .children()
            .find(|child| child.kind() == SyntaxKind::BLOCK)?;
        let end = closure
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::END_KW)?
            .text_range();
        Some(BlockCall {
            name,
            header: range_of(node).start..range_of(&block).start,
            end: end.start().into()..end.end().into(),
            call: node.clone(),
            block,
        })
    }

    /// The first argument's string literal, as written between its quotes.
    fn label(&self, default: &str) -> String {
        self.call
            .children()
            .find(|child| child.kind() == SyntaxKind::ARG_LIST)
            .and_then(|args| {
                args.children()
                    .find(|child| child.kind() == SyntaxKind::STRING_EXPR)
            })
            .map(|string| {
                string
                    .descendants_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| {
                        !matches!(
                            token.kind(),
                            SyntaxKind::STRING_START | SyntaxKind::STRING_END
                        )
                    })
                    .map(|token| token.text().to_string())
                    .collect()
            })
            .unwrap_or_else(|| default.to_string())
    }

    /// Replace the header and the `end`, keeping the header's line breaks.
    fn rewrite(&self, edits: &mut Vec<(Range<usize>, String)>, header: &str, end: &str) {
        let header = (
            self.header.clone(),
            keep_lines(header, &self.call, self.header.clone()),
        );
        edits.push(header);
        edits.push((self.end.clone(), end.to_string()));
    }
}

fn range_of(node: &SyntaxNode) -> Range<usize> {
    let range = node.text_range();
    range.start().into()..range.end().into()
}

/// `text` with as many line breaks as `range` (within `node`) had.
fn keep_lines(text: &str, node: &SyntaxNode, range: Range<usize>) -> String {
    let start = range_of(node).start;
    let original = &node.text().to_string()[range.start - start..range.end - start];
    format!("{text}{}", "\n".repeat(original.matches('\n').count()))
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

// ── assert_receive ────────────────────────────────────────────────────────

/// `assert_receive PATTERN[, TIMEOUT_MS]` (timeout 100ms by default) as a
/// `receive` on the same lines:
///
///   receive do
///     PATTERN -> ()
///     __assert_receive_other -> test_fail_msg("assert_receive PATTERN received another message")
///   after TIMEOUT_MS -> test_fail_msg("assert_receive PATTERN timed out after TIMEOUT_MSms")
///   end
///
/// The catch-all arm fails the test on a message the pattern does not match
/// (the type checker does not report it as redundant).
fn expand_assert_receive(node: &SyntaxNode) -> String {
    let mut parts = node.children().map(|child| child.text().to_string());
    let pattern = parts.next().unwrap_or_default();
    let timeout_ms = parts.next().unwrap_or_else(|| "100".to_string());
    // Escape the pattern for embedding in the failure messages.
    let escaped = pattern.replace('\\', "\\\\").replace('"', "\\\"");
    let expansion = format!(
        "receive do {pattern} -> () __assert_receive_other -> test_fail_msg(\"assert_receive {escaped} received another message\") after {timeout_ms} -> test_fail_msg(\"assert_receive {escaped} timed out after {timeout_ms}ms\") end"
    );
    keep_lines(&expansion, node, range_of(node))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_test_source_keeps_every_line_in_place() {
        // Diagnostics named the generated file's lines, which had moved: the
        // tests were emitted after the other items, re-indented.
        let source = "fn pick(n :: Int) -> Int do\n  if n < 0 do\n    0\n  else if n > 9 do\n    9\n  else\n    n\n  end\nend\n\ntest(\"pick\") do\n  if pick(3) == 3 do\n    assert(true)\n  else if pick(3) == 0 do\n    assert(false)\n  else\n    assert(false)\n  end\nend\n\nfn later() -> Int do\n  1\nend\n";

        let out = preprocess_test_source(source).unwrap();

        let out_lines: Vec<&str> = out.lines().collect();
        for (i, line) in source.lines().enumerate() {
            if i == 10 {
                assert_eq!(out_lines[i], "fn __test_body_0() do", "{out}");
            } else {
                assert_eq!(out_lines[i], line, "{out}");
            }
        }
        assert!(
            out[source.len()..].contains(
                "test_begin(\"pick\")\n  test_run_body(fn() do __test_body_0() end)\n  test_end()"
            ),
            "{out}"
        );
    }

    #[test]
    fn preprocess_test_source_reads_strings_in_interpolations() {
        // The text scanner took the `end` in `" end "` for the test's own.
        let source = "test(\"interp\") do\n  let s = \"#{String.join([\"a\"], \" end \")}\"\n  assert_eq(s, \"a\")\nend\n";
        let out = preprocess_test_source(source).unwrap();
        assert!(
            out.starts_with(&source.replace("test(\"interp\") do", "fn __test_body_0() do")),
            "{out}"
        );
    }

    #[test]
    fn preprocess_test_source_runs_a_describe_by_case() {
        let source = "describe(\"group\") do\n  setup() do\n    let base = if true do\n      1\n    else if false do\n      2\n    else\n      3\n    end\n  end\n  teardown do\n    println(\"#{base}\")\n  end\n  test(\"one\") do\n    assert_receive 40, 50\n  end\nend\n\ntest(\"two\") do\n  assert(true)\nend\n";

        let out = preprocess_test_source(source).unwrap();

        let out_lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            out_lines[0], "fn __test_describe_0(__case :: Int) do",
            "{out}"
        );
        assert_eq!(out_lines[1], "  ", "{out}");
        assert_eq!(out_lines[2], "    let base = if true do", "{out}");
        assert_eq!(out_lines[9], "  ", "{out}");
        assert_eq!(out_lines[10], "  let __teardown = fn() do", "{out}");
        assert_eq!(
            out_lines[13], "  if __case == 0 do test_run_body(fn() do",
            "{out}"
        );
        assert!(
            out_lines[14].starts_with("    receive do 40 -> () "),
            "{out}"
        );
        assert!(
            out_lines[14].ends_with(
                "after 50 -> test_fail_msg(\"assert_receive 40 timed out after 50ms\") end"
            ),
            "{out}"
        );
        assert_eq!(out_lines[15], "  end) end", "{out}");
        assert_eq!(out_lines[16], "test_run_body(__teardown) end", "{out}");
        assert_eq!(out_lines[18], "fn __test_body_1() do", "{out}");
        assert!(
            out.contains(
                "test_begin(\"group > one\")\n  test_run_body(fn() do __test_describe_0(0) end)"
            ),
            "{out}"
        );
        assert!(out.contains("test_begin(\"two\")"), "{out}");
    }

    #[test]
    fn preprocess_test_source_nests_describes() {
        // A describe in a describe was left as it was: `test` was undefined.
        let source = "describe(\"outer\") do\n  setup do\n    let x = 1\n  end\n  test(\"a\") do\n    assert(x == 1)\n  end\n  describe(\"inner\") do\n    teardown do\n      println(\"t\")\n    end\n    test(\"b\") do\n      assert(x == 1)\n    end\n    test(\"c\") do\n      assert(true)\n    end\n  end\nend\n";

        let out = preprocess_test_source(source).unwrap();

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[7], "  if __case >= 1 && __case < 3 do", "{out}");
        assert_eq!(lines[8], "    let __teardown = fn() do", "{out}");
        assert_eq!(lines[13], "    end) end", "{out}");
        assert_eq!(lines[17], "  test_run_body(__teardown) end", "{out}");
        assert_eq!(lines[18], "end", "{out}");
        for (label, case) in [
            ("outer > a", 0),
            ("outer > inner > b", 1),
            ("outer > inner > c", 2),
        ] {
            let run = format!(
                "test_begin(\"{label}\")\n  test_run_body(fn() do __test_describe_0({case}) end)"
            );
            assert!(out.contains(&run), "{out}");
        }
    }

    #[test]
    fn preprocess_test_source_refuses_a_late_setup_and_passes_parse_errors() {
        let late = "describe(\"g\") do\n  test(\"a\") do\n    assert(true)\n  end\n  setup do\n    let x = 1\n  end\nend\n";
        let err = preprocess_test_source(late).unwrap_err();
        assert!(err.starts_with("line 5: `setup` must come before"), "{err}");

        // The build reports the parse error where it is.
        let broken = "test(\"a\") do\n  let x = (1\nend\n";
        assert_eq!(preprocess_test_source(broken).unwrap(), broken);
    }
}
