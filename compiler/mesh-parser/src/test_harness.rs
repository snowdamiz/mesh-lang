//! `meshc test` files as Mesh programs.
//!
//! A `.test.mpl` file writes its tests as `test("label") do ... end` blocks,
//! grouped by `describe` with `setup` and `teardown`. `meshc test` compiles
//! it after `preprocess_test_source`, and the language server analyzes it
//! the same way.

use std::ops::Range;

use crate::{SyntaxKind, SyntaxNode};

/// Original UTF-8 byte offsets and inclusive, one-based source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSpan {
    pub bytes: Range<usize>,
    pub start_line: usize,
    pub end_line: usize,
}

impl TestSpan {
    fn new(source: &str, bytes: Range<usize>) -> Self {
        Self {
            start_line: line_of(source, bytes.start),
            end_line: line_of(source, bytes.end.saturating_sub(1)),
            bytes,
        }
    }

    pub fn contains_line(&self, line: usize) -> bool {
        self.start_line <= line && line <= self.end_line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestPolicy {
    #[default]
    Synchronous,
    Asynchronous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFixture {
    pub scope: TestSpan,
    pub setups: Vec<TestSpan>,
    pub teardown: Option<TestSpan>,
}

#[derive(Debug, Clone)]
pub struct TestCase {
    /// Unique within this file, in declaration order, including duplicate names.
    pub id: usize,
    pub name: String,
    pub span: TestSpan,
    pub tags: Vec<String>,
    pub policy: TestPolicy,
    /// Outer scope first. Teardown runs in reverse scope order.
    pub fixtures: Vec<TestFixture>,
    label: String,
    run: String,
}

#[derive(Debug, Clone)]
pub struct TestPlan {
    pub cases: Vec<TestCase>,
    source: String,
    parsed: bool,
}

impl TestPlan {
    pub fn has_parse_errors(&self) -> bool {
        !self.parsed
    }

    /// Keep the LSP's all-cases program and its original source lines.
    pub fn program(&self) -> String {
        self.render(false)
    }

    /// One compilation supports every selection. The runner supplies comma-delimited IDs.
    pub fn executable(&self) -> String {
        self.render(true)
    }

    fn render(&self, selected: bool) -> String {
        let mut out = self.source.clone();
        if self.cases.is_empty() || !self.parsed {
            return out;
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\nfn main() do\n");
        if selected {
            out.push_str("  let __selected = Env.get(\"MESH_TEST_CASES\", \"\")\n");
        }
        for case in &self.cases {
            if selected {
                out.push_str(&format!(
                    "  if String.contains(__selected, \",{},\") do\n",
                    case.id
                ));
            }
            out.push_str(&format!(
                "  test_cleanup_actors()\n  test_begin(\"{}\")\n  test_run_body(fn() do {} end)\n  test_end()\n",
                case.label, case.run
            ));
            if selected {
                out.push_str("  end\n");
            }
        }
        out.push_str("  test_cleanup_actors()\n  test_summary(test_pass_count(), test_fail_count(), 0)\nend\n");
        out
    }
}

/// The editor and CLI share preprocessing; the editor need not select cases.
pub fn preprocess_test_source(source: &str) -> Result<String, String> {
    Ok(plan_test_source(source)?.program())
}

pub fn plan_test_source(source: &str) -> Result<TestPlan, String> {
    let parse = crate::parse(source);
    if !parse.errors().is_empty() {
        return Ok(TestPlan {
            source: source.to_string(),
            cases: Vec::new(),
            parsed: false,
        });
    }
    let mut edits = Vec::new();
    let mut tests = Vec::new();
    let mut describes = 0;
    for node in parse.syntax().children() {
        let Some(call) = BlockCall::of(&node) else {
            continue;
        };
        match call.name.as_str() {
            "test" => {
                let n = tests.len();
                let metadata = call.metadata(source, &Metadata::default())?;
                call.rewrite(&mut edits, &format!("fn __test_body_{n}() do"), "end");
                tests.push(call.case(
                    source,
                    n,
                    &call.label("unnamed"),
                    metadata,
                    Vec::new(),
                    format!("__test_body_{n}()"),
                ));
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
                let teardown =
                    group.describe(&call, &call.label("describe"), &Metadata::default(), &[])?;
                call.rewrite(
                    &mut edits,
                    &format!("fn __test_describe_{d}(__case :: Int) do"),
                    teardown_end(teardown),
                );
            }
            _ => continue,
        }
        for node in call.block.descendants() {
            if node.kind() == SyntaxKind::ASSERT_RECEIVE_EXPR {
                edits.push((range_of(&node), expand_assert_receive(&node)));
            }
        }
    }
    if tests.is_empty() {
        return Ok(TestPlan {
            source: source.to_string(),
            cases: tests,
            parsed: true,
        });
    }
    let mut out = source.to_string();
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, text) in edits {
        out.replace_range(range, &text);
    }
    Ok(TestPlan {
        source: out,
        cases: tests,
        parsed: true,
    })
}

/// Marked source-documentation fences. Unmarked examples stay illustrative.
#[derive(Debug, Clone)]
pub struct DocumentationTest {
    pub plan: TestPlan,
    pub compile_fail: bool,
}

pub fn plan_documentation_tests(source: &str) -> Result<Vec<DocumentationTest>, String> {
    use mesh_common::token::TokenKind;
    struct Fence {
        span: Range<usize>,
        compile_fail: bool,
        edits: Vec<(Range<usize>, String)>,
    }
    let mut fences = Vec::new();
    let mut open: Option<Fence> = None;
    let mut previous_line = 0;
    for token in mesh_lexer::Lexer::new(source) {
        if !matches!(
            token.kind,
            TokenKind::DocComment | TokenKind::ModuleDocComment
        ) {
            continue;
        }
        let range = token.span.start as usize..token.span.end as usize;
        let line = line_of(source, range.start);
        if open.is_some() && line != previous_line + 1 {
            return Err(format!(
                "line {previous_line}: doctest fence must contain consecutive documentation lines"
            ));
        }
        previous_line = line;
        let comment = &source[range.clone()];
        let text = comment
            .strip_prefix("##!")
            .or_else(|| comment.strip_prefix("##"))
            .expect("documentation token prefix");
        let text = text.strip_prefix(' ').unwrap_or(text);
        if let Some(fence) = &mut open {
            if text.trim() == "```" {
                fence.span.end = range.end;
                fence.edits.push((range, "end".into()));
                expand_doc_expectations(&mut fence.edits, source)?;
                fences.push(open.take().unwrap());
            } else {
                let padding = " ".repeat(source[range.clone()].len() - text.len());
                fence.edits.push((range, format!("{padding}{text}")));
            }
        } else if matches!(text.trim(), "```mesh,doctest" | "```mesh,compile_fail") {
            open = Some(Fence {
                span: range.clone(),
                compile_fail: text.trim() == "```mesh,compile_fail",
                edits: vec![(range, format!("test(\"doctest line {line}\") do"))],
            });
        }
    }
    if let Some(fence) = open {
        return Err(format!(
            "line {}: unclosed doctest fence",
            line_of(source, fence.span.start)
        ));
    }
    let parse = crate::parse(source);
    if parse
        .syntax()
        .children()
        .filter_map(|node| BlockCall::of(&node))
        .any(|call| matches!(call.name.as_str(), "test" | "describe"))
    {
        return Err(
            "documentation tests require an ordinary module; run test DSL files without --doc"
                .into(),
        );
    }
    for fence in &fences {
        if parse.syntax().children().any(|node| {
            let range = range_of(&node);
            range.start <= fence.span.start && fence.span.end <= range.end
        }) {
            return Err(format!(
                "line {}: documentation test fences must be at module scope",
                line_of(source, fence.span.start)
            ));
        }
    }
    let without_main: Vec<_> = parse
        .syntax()
        .children()
        .filter(|n| n.kind() == SyntaxKind::FN_DEF)
        .filter(|node| {
            node.children()
                .any(|n| n.kind() == SyntaxKind::NAME && n.text().to_string() == "main")
        })
        .map(|node| {
            let span = range_of(&node);
            (
                span.clone(),
                source[span]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' })
                    .collect::<String>(),
            )
        })
        .collect();
    let mut groups = Vec::new();
    let runnable: Vec<_> = fences
        .iter()
        .enumerate()
        .filter(|(_, fence)| !fence.compile_fail)
        .collect();
    if !runnable.is_empty() {
        groups.push(runnable);
    }
    groups.extend(
        fences
            .iter()
            .enumerate()
            .filter(|(_, fence)| fence.compile_fail)
            .map(|fence| vec![fence]),
    );
    groups
        .into_iter()
        .map(|group| {
            let mut edits = without_main.clone();
            edits.extend(group.iter().flat_map(|(_, fence)| fence.edits.clone()));
            edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
            let mut generated = source.to_string();
            for (range, text) in edits {
                generated.replace_range(range, &text);
            }
            let mut plan = plan_test_source(&generated)?;
            if !plan.parsed {
                plan.cases = group
                    .iter()
                    .map(|(id, fence)| TestCase {
                        id: *id,
                        name: String::new(),
                        span: TestSpan::new(source, fence.span.clone()),
                        tags: Vec::new(),
                        policy: TestPolicy::Synchronous,
                        fixtures: Vec::new(),
                        label: String::new(),
                        run: String::new(),
                    })
                    .collect();
            }
            for (case, (id, fence)) in plan.cases.iter_mut().zip(&group) {
                case.id = *id;
                case.span = TestSpan::new(source, fence.span.clone());
                case.name = format!("doctest line {}", case.span.start_line);
                case.label = case.name.clone();
            }
            Ok(DocumentationTest {
                plan,
                compile_fail: group[0].1.compile_fail,
            })
        })
        .collect()
}

fn expand_doc_expectations(
    edits: &mut [(Range<usize>, String)],
    source: &str,
) -> Result<(), String> {
    use mesh_common::token::TokenKind;
    let mut starts = Vec::new();
    let mut snippet = String::new();
    for (_, code) in edits.iter() {
        starts.push(snippet.len());
        snippet.push_str(code);
        snippet.push('\n');
    }
    let markers: Vec<_> = mesh_lexer::Lexer::new(&snippet)
        .filter(|token| {
            token.kind == TokenKind::Comment
                && snippet[token.span.start as usize..token.span.end as usize].starts_with("# =>")
        })
        .map(|token| {
            let offset = token.span.start as usize;
            let index = starts.partition_point(|start| *start <= offset) - 1;
            (index, offset - starts[index])
        })
        .collect();
    for (index, at) in markers {
        let code = &edits[index].1;
        let expected = code[at + 4..].trim().to_string();
        let line = line_of(source, edits[index].0.start);
        if expected.is_empty() {
            return Err(format!("line {line}: # => needs an expected expression"));
        }
        let expression = code[..at].trim();
        let target = if expression.is_empty() {
            if index <= 1 {
                return Err(format!("line {line}: # => needs a preceding expression"));
            }
            index - 1
        } else {
            index
        };
        let expression = if target == index {
            expression.to_string()
        } else {
            edits[target].1.trim().to_string()
        };
        let spaces = " ".repeat(edits[target].1.len() - edits[target].1.trim_start().len());
        edits[target].1 = format!("{spaces}assert_eq({expression}, {expected})");
    }
    Ok(())
}

#[derive(Default, Clone)]
struct Metadata {
    tags: Vec<String>,
    policy: TestPolicy,
}

/// The tests of one top-level `describe`: cases of one function.
struct Group<'a> {
    source: &'a str,
    /// The `__test_describe_N` function's N.
    function: usize,
    /// Cases numbered so far.
    cases: usize,
    edits: &'a mut Vec<(Range<usize>, String)>,
    tests: &'a mut Vec<TestCase>,
}

impl Group<'_> {
    /// Rewrite the setup, teardown, tests and nested describes of the
    /// describe `call`, labelled `label`, and say whether it has a teardown.
    fn describe(
        &mut self,
        call: &BlockCall,
        label: &str,
        inherited: &Metadata,
        fixtures: &[TestFixture],
    ) -> Result<bool, String> {
        let metadata = call.metadata(self.source, inherited)?;
        let mut fixtures = fixtures.to_vec();
        let mut fixture = TestFixture {
            scope: TestSpan::new(self.source, range_of(&call.call)),
            setups: Vec::new(),
            teardown: None,
        };
        for inner in call
            .block
            .children()
            .filter_map(|node| BlockCall::of(&node))
        {
            let span = TestSpan::new(self.source, range_of(&inner.call));
            match inner.name.as_str() {
                "setup" => fixture.setups.push(span),
                "teardown" => fixture.teardown = Some(span),
                _ => {}
            }
        }
        fixtures.push(fixture);
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
                    let metadata = inner.metadata(self.source, &metadata)?;
                    self.tests.push(inner.case(
                        self.source,
                        self.tests.len(),
                        &format!("{label} > {}", inner.label("unnamed")),
                        metadata,
                        fixtures.clone(),
                        format!("__test_describe_{}({case})", self.function),
                    ));
                    self.cases += 1;
                }
                "describe" => {
                    has_tests = true;
                    let first = self.cases;
                    let nested = format!("{label} > {}", inner.label("describe"));
                    let teardown = self.describe(&inner, &nested, &metadata, &fixtures)?;
                    let header = format!(
                        "if __case >= {first} && __case < {} do test_run_body(fn() do",
                        self.cases
                    );
                    let end = if teardown {
                        "test_run_body(__teardown) end) end"
                    } else {
                        "end) end"
                    };
                    inner.rewrite(self.edits, &header, end);
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

    fn case(
        &self,
        source: &str,
        id: usize,
        label: &str,
        metadata: Metadata,
        fixtures: Vec<TestFixture>,
        run: String,
    ) -> TestCase {
        TestCase {
            id,
            name: decode_string(label),
            label: label.to_string(),
            span: TestSpan::new(source, range_of(&self.call)),
            tags: metadata.tags,
            policy: metadata.policy,
            fixtures,
            run,
        }
    }

    fn metadata(&self, source: &str, inherited: &Metadata) -> Result<Metadata, String> {
        let fail =
            |message: &str| format!("line {}: {message}", line_of(source, self.header.start));
        let args = self
            .call
            .children()
            .find(|n| n.kind() == SyntaxKind::ARG_LIST)
            .ok_or_else(|| fail("test and describe need a literal name"))?;
        let mut args = args.children();
        let name = args
            .next()
            .ok_or_else(|| fail("test and describe need a literal name"))?;
        literal_string(&name)
            .ok_or_else(|| fail("test and describe names must be static strings"))?;
        let mut metadata = inherited.clone();
        if let Some(options) = args.next() {
            if options.kind() != SyntaxKind::MAP_LITERAL || args.next().is_some() {
                return Err(fail("expected tags: [\"tag\"] and/or async: true"));
            }
            let mut keys = std::collections::HashSet::new();
            for entry in options
                .children()
                .filter(|n| n.kind() == SyntaxKind::MAP_ENTRY)
            {
                let mut parts = entry.children();
                let key = parts
                    .next()
                    .map(|n| n.text().to_string())
                    .unwrap_or_default();
                let value = parts
                    .next()
                    .ok_or_else(|| fail("missing test option value"))?;
                if !keys.insert(key.clone()) {
                    return Err(fail("duplicate test option"));
                }
                match key.as_str() {
                    "tags" if value.kind() == SyntaxKind::LIST_LITERAL => {
                        for tag in value.children() {
                            let tag = literal_string(&tag)
                                .filter(|s| !s.is_empty())
                                .ok_or_else(|| fail("tags must be nonempty static strings"))?;
                            if !metadata.tags.contains(&tag) {
                                metadata.tags.push(tag);
                            }
                        }
                    }
                    "async" => {
                        metadata.policy = match value.text().to_string().as_str() {
                            "true" => TestPolicy::Asynchronous,
                            "false" => TestPolicy::Synchronous,
                            _ => return Err(fail("async must be true or false")),
                        }
                    }
                    _ => return Err(fail("unknown test option; expected tags or async")),
                }
            }
        }
        Ok(metadata)
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

fn literal_string(node: &SyntaxNode) -> Option<String> {
    if node.kind() != SyntaxKind::STRING_EXPR || node.children().next().is_some() {
        return None;
    }
    let text = node.text().to_string();
    Some(decode_string(text.strip_prefix('"')?.strip_suffix('"')?))
}

fn decode_string(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some('0') => result.push('\0'),
            Some('u') => {
                chars.next();
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    result.push(c);
                }
            }
            Some(c) => result.push(c),
            None => {}
        }
    }
    result
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
        assert_eq!(
            lines[7], "  if __case >= 1 && __case < 3 do test_run_body(fn() do",
            "{out}"
        );
        assert_eq!(lines[8], "    let __teardown = fn() do", "{out}");
        assert_eq!(lines[13], "    end) end", "{out}");
        assert_eq!(lines[17], "  test_run_body(__teardown) end) end", "{out}");
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

    #[test]
    fn preprocess_test_source_leaves_what_is_not_a_test() {
        // No tests: the file is returned as it is.
        let plain = "fn f() -> Int do\n  1\nend\n";
        assert_eq!(preprocess_test_source(plain).unwrap(), plain);
        // A block call other than a test or a describe, at the top or in a
        // describe, and a statement in a describe, stay as they are; a file
        // that does not end with a new line gets one before `main`.
        let source = "run() do\n  1\nend\ndescribe(\"g\") do\n  let x = 1\n  run() do\n    x\n  end\n  test(\"a\") do\n    assert(x == 1)\n  end\nend";
        let out = preprocess_test_source(source).unwrap();
        assert!(out.starts_with("run() do\n  1\nend\n"), "{out}");
        assert!(
            out.contains("\n  let x = 1\n  run() do\n    x\n  end\n"),
            "{out}"
        );
        assert!(out.contains("  end) end\nend\n\nfn main() do"), "{out}");
    }

    #[test]
    fn preprocess_test_source_refuses_a_second_teardown() {
        let source = "describe(\"g\") do\n  teardown do\n    1\n  end\n  teardown do\n    2\n  end\n  test(\"a\") do\n    assert(true)\n  end\nend\n";
        let err = preprocess_test_source(source).unwrap_err();
        assert!(
            err.starts_with("line 5: a describe has one `teardown`"),
            "{err}"
        );
    }

    #[test]
    fn plan_records_origins_inherited_tags_policy_and_fixtures() {
        let source = "describe(\"outer\", tags: [\"shared\"], async: true) do\n  setup do\n    let value = 1\n  end\n  teardown do\n    println(value)\n  end\n  test(\"same\", tags: [\"one\"]) do\n    assert(true)\n  end\n  describe(\"nested\", tags: [\"two\"], async: false) do\n    test(\"same\") do\n      assert(true)\n    end\n  end\nend\n";
        let plan = plan_test_source(source).unwrap();
        assert_eq!(plan.cases.len(), 2);
        let first = &plan.cases[0];
        assert_eq!(first.id, 0);
        assert_eq!(first.name, "outer > same");
        assert_eq!(first.tags, ["shared", "one"]);
        assert_eq!(first.policy, TestPolicy::Asynchronous);
        assert_eq!(first.span.start_line, 8);
        assert_eq!(first.span.end_line, 10);
        assert_eq!(
            &source[first.span.bytes.clone()],
            "test(\"same\", tags: [\"one\"]) do\n    assert(true)\n  end"
        );
        assert_eq!(first.fixtures[0].setups[0].start_line, 2);
        assert_eq!(first.fixtures[0].teardown.as_ref().unwrap().start_line, 5);
        let second = &plan.cases[1];
        assert_eq!(second.id, 1);
        assert_eq!(second.tags, ["shared", "two"]);
        assert_eq!(second.policy, TestPolicy::Synchronous);
        assert_eq!(second.fixtures.len(), 2);
        assert!(
            crate::parse(&plan.executable()).errors().is_empty(),
            "{}",
            plan.executable()
        );
    }

    #[test]
    fn metadata_rejects_dynamic_unknown_and_duplicate_options() {
        for header in [
            "test(name)",
            "test(\"#{name}\")",
            "test(\"name\", tags: [tag])",
            "test(\"name\", async: 1)",
            "test(\"name\", unknown: true)",
            "test(\"name\", async: true, async: false)",
        ] {
            assert!(
                plan_test_source(&format!("{header} do\nend\n")).is_err(),
                "{header}"
            );
        }
        let plan = plan_test_source(
            "test(\"a\\n\\u{1F389}\") do\nend\ntest(\"a\\n\\u{1F389}\") do\nend\n",
        )
        .unwrap();
        assert_eq!(plan.cases[0].name, "a\n🎉");
        assert_eq!(plan.cases[0].name, plan.cases[1].name);
        assert_ne!(plan.cases[0].id, plan.cases[1].id);
    }

    #[test]
    fn documentation_fences_keep_lines_and_ignore_strings_and_unmarked_examples() {
        let source = "## ```mesh\n## ignored()\n## ```\n## ```mesh,doctest\n## 20 + 22 # => 42\n## \"# => not a marker\"\n## ```\nfn main() do\n  println(\"## ```mesh,doctest\")\nend\n";
        let docs = plan_documentation_tests(source).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].plan.cases[0].span.start_line, 4);
        assert_eq!(
            docs[0].plan.program().lines().nth(4).unwrap(),
            "   assert_eq(20 + 22, 42)"
        );
        assert!(docs[0].plan.program().contains("\"# => not a marker\""));
        assert_eq!(docs[0].plan.program().matches("fn main()").count(), 1);
        assert!(plan_documentation_tests("## ```mesh,doctest\nfn x() do\nend\n").is_err());
        let nested = "fn main() do\n  ## ```mesh,doctest\n  ## 1 + 1 # => 2\n  ## ```\nend\n";
        assert!(plan_documentation_tests(nested)
            .unwrap_err()
            .contains("module scope"));
        let broken =
            plan_documentation_tests("## ```mesh,compile_fail\n## let value = (1\n## ```\n")
                .unwrap();
        assert_eq!(broken[0].plan.cases[0].span.start_line, 1);
        assert!(!broken[0].plan.program().contains("__test_body"));
    }
    #[test]
    fn documentation_markers_inside_multiline_strings_are_not_expectations() {
        let source = "## ```mesh,doctest\n## let text = \"first\n## # => this is string content\n## last\"\n## assert(String.contains(text, \"# =>\"))\n## ```\n";
        let docs = plan_documentation_tests(source).unwrap();
        let program = docs[0].plan.program();
        assert!(program.contains("# => this is string content"));
        assert!(!program.contains("assert_eq"));
        assert!(crate::parse(&program).errors().is_empty(), "{program}");
    }
}
