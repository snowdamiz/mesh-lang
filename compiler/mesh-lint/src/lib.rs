//! Mesh linter: code the compiler accepts but a reviewer should not.
//!
//! Every rule reads the lossless CST from `mesh-parser`, so linting needs no
//! type information and works on any source that parses. Findings are the
//! catalog's `L` warnings, reported like every other diagnostic.

use mesh_common::span::Span;
use mesh_diagnostics::{Applicability, Code, Diagnostic, SourceFile};
use mesh_parser::{Parse, SyntaxKind, SyntaxNode, SyntaxToken};

/// Control-flow nesting a function body may use before `L0001` fires.
pub const MAX_NESTING: usize = 4;

/// Source text a message or repair quotes; longer text is described instead.
const QUOTE_LIMIT: usize = 60;

/// The lint findings for `parse`, the parse of `file`'s text, in source
/// order. A parse with errors gets none: its parse errors come first.
pub fn lint(parse: &Parse, file: &SourceFile) -> Vec<Diagnostic> {
    if !parse.ok() {
        return Vec::new();
    }
    let root = parse.syntax();
    let mut found = Vec::new();
    deep_nesting(file, &root, &mut Vec::new(), &mut found);
    for node in root.descendants() {
        found.extend(match node.kind() {
            SyntaxKind::ELSE_BRANCH => collapsible_else_if(file, &node),
            SyntaxKind::MATCH_ARM => pass_through_arm(file, &node),
            SyntaxKind::BINARY_EXPR => bool_comparison(file, &node),
            _ => None,
        });
    }
    found.sort_by_key(|diagnostic| diagnostic.span().map(|span| span.start));
    found
}

/// How a node changes the nesting depth of what it holds.
enum Nesting {
    /// A definition: its body starts over at depth 0.
    Restarts,
    /// A control-flow construct, one level deeper.
    Deepens,
    Keeps,
}

fn nesting(node: &SyntaxNode) -> Nesting {
    match node.kind() {
        SyntaxKind::FN_DEF
        | SyntaxKind::ACTOR_DEF
        | SyntaxKind::CALL_HANDLER
        | SyntaxKind::CAST_HANDLER
        | SyntaxKind::TERMINATE_CLAUSE => Nesting::Restarts,
        SyntaxKind::TRAILING_CLOSURE if is_test_block(node) => Nesting::Restarts,
        // `else if` continues its chain rather than nesting inside it.
        SyntaxKind::IF_EXPR if parent_kind(node) == Some(SyntaxKind::ELSE_BRANCH) => Nesting::Keeps,
        SyntaxKind::IF_EXPR
        | SyntaxKind::CASE_EXPR
        | SyntaxKind::FOR_IN_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::RECEIVE_EXPR
        | SyntaxKind::CLOSURE_EXPR
        | SyntaxKind::TRAILING_CLOSURE => Nesting::Deepens,
        _ => Nesting::Keeps,
    }
}

/// `L0001`. `if`, `case`, loops, `receive` and closures each nest one level
/// deeper, counted from the enclosing definition. `levels` holds the span of
/// the keyword of each construct `node` is nested in. Only the outermost construct past
/// [`MAX_NESTING`] is reported, with the levels it is nested in.
fn deep_nesting(
    file: &SourceFile,
    node: &SyntaxNode,
    levels: &mut Vec<Span>,
    found: &mut Vec<Diagnostic>,
) {
    for child in node.children() {
        match nesting(&child) {
            Nesting::Restarts => {
                let outer = std::mem::take(levels);
                deep_nesting(file, &child, levels, found);
                *levels = outer;
            }
            Nesting::Keeps => deep_nesting(file, &child, levels, found),
            Nesting::Deepens => {
                let Some(keyword) = first_token(&child) else {
                    continue;
                };
                if levels.len() < MAX_NESTING {
                    levels.push(token_span(&keyword));
                    deep_nesting(file, &child, levels, found);
                    levels.pop();
                    continue;
                }
                let depth = levels.len() + 1;
                let keyword_text = keyword.text();
                let mut diagnostic = Diagnostic::in_file(
                    Code::DeepNesting,
                    file,
                    format!(
                        "`{keyword_text}` is nested {depth} levels deep (at most {MAX_NESTING})"
                    ),
                )
                .at(token_span(&keyword), format!("level {depth}"))
                .with_repair(format!(
                    "Move this `{keyword_text}` into a function of its own and call it, return \
                     early from the enclosing function, or use `?` in place of a `case` that only \
                     passes an error on."
                ));
                for (level, span) in levels.iter().enumerate() {
                    diagnostic = diagnostic.with_label(*span, format!("level {}", level + 1));
                }
                found.push(diagnostic);
            }
        }
    }
}

/// The body of a top-level `test`, `describe`, `setup` or `teardown` block
/// (or one inside a `describe`) is a definition of its own, like a function's.
fn is_test_block(closure: &SyntaxNode) -> bool {
    closure.parent().is_some_and(|call| {
        matches!(
            callee_name(&call).as_deref(),
            Some("test" | "describe" | "setup" | "teardown")
        ) && at_test_file_top_level(&call)
    })
}

fn at_test_file_top_level(call: &SyntaxNode) -> bool {
    match call.parent() {
        Some(parent) if parent.kind() == SyntaxKind::SOURCE_FILE => true,
        Some(block) if block.kind() == SyntaxKind::BLOCK => block
            .parent()
            .filter(|closure| closure.kind() == SyntaxKind::TRAILING_CLOSURE)
            .and_then(|closure| closure.parent())
            .is_some_and(|outer| {
                callee_name(&outer).as_deref() == Some("describe") && at_test_file_top_level(&outer)
            }),
        _ => false,
    }
}

fn callee_name(call: &SyntaxNode) -> Option<String> {
    let callee = call.first_child()?;
    (callee.kind() == SyntaxKind::NAME_REF).then(|| callee.text().to_string())
}

/// `L0002`. An `else` whose whole body is one `if`: `else if` says the same
/// one level shallower.
fn collapsible_else_if(file: &SourceFile, else_branch: &SyntaxNode) -> Option<Diagnostic> {
    let block = else_branch
        .children()
        .find(|c| c.kind() == SyntaxKind::BLOCK)?;
    let mut statements = block.children();
    let (Some(only), None) = (statements.next(), statements.next()) else {
        return None;
    };
    if only.kind() != SyntaxKind::IF_EXPR {
        return None;
    }
    let else_keyword = first_token(else_branch)?;
    let if_keyword = first_token(&only)?;
    Some(
        Diagnostic::in_file(
            Code::CollapsibleElseIf,
            file,
            "this `else` holds only an `if`",
        )
        .at(token_span(&else_keyword), "this `else`")
        .with_label(token_span(&if_keyword), "holds only this `if`"),
    )
}

/// `L0003`. `Ok(value) -> Ok(value)` is what the arm `Ok(value)` means on
/// its own.
fn pass_through_arm(file: &SourceFile, arm: &SyntaxNode) -> Option<Diagnostic> {
    let arrow = arm
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::ARROW)?;
    let (pattern, body) = arm.first_child().zip(arm.last_child())?;
    let passes_through = pattern != body
        && stands_for_its_value(&pattern)
        && significant_text(&pattern) == significant_text(&body);
    if !passes_through {
        return None;
    }
    let mut diagnostic = Diagnostic::in_file(
        Code::PassThroughArm,
        file,
        "this arm returns exactly what it matched",
    )
    .at(significant_span(arm), "returns what it matched");
    let head = text_within(
        arm,
        Span::new(significant_span(arm).start, token_span(&arrow).start),
    );
    let value = text_within(&body, significant_span(&body));
    if let (Some(head), Some(value)) = (quotable(head.trim_end()), quotable(&value)) {
        diagnostic = diagnostic.with_repair(format!(
            "Delete `-> {value}` and leave `{head}` on its own as the arm."
        ));
    }
    Some(diagnostic)
}

/// Whether a pattern alone can be an arm's value, as the type checker
/// accepts it (E0056 otherwise): names, literals and constructors of them.
/// `(0, b) -> (0, b)` stays: `(0, b)` alone does not compile.
fn stands_for_its_value(pattern: &SyntaxNode) -> bool {
    match pattern.kind() {
        SyntaxKind::IDENT_PAT | SyntaxKind::LITERAL_PAT => true,
        SyntaxKind::CONSTRUCTOR_PAT => pattern
            .children()
            .filter(|child| mesh_parser::ast::pat::Pattern::cast(child.clone()).is_some())
            .all(|child| stands_for_its_value(&child)),
        _ => false,
    }
}

/// `L0004`. `flag == true` is `flag`, and `flag == false` is `not flag`.
///
/// The edit is machine-applicable: an operand of `==` binds tighter than
/// `==`, and `not` (a prefix operator) tighter than every infix operator,
/// so either stands where the comparison stood. An operand that is itself
/// an infix expression is parenthesized under `not`. A comment outside the
/// kept operand would be lost, so a comparison holding one gets no edit.
fn bool_comparison(file: &SourceFile, binary: &SyntaxNode) -> Option<Diagnostic> {
    let operator = binary
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| matches!(token.kind(), SyntaxKind::EQ_EQ | SyntaxKind::NOT_EQ))?;
    let mut operands = binary.children();
    let (left, right) = (operands.next()?, operands.next()?);
    let (value, literal) = match (bool_literal(&left), bool_literal(&right)) {
        (_, Some(literal)) => (left, literal),
        (Some(literal), None) => (right, literal),
        (None, None) => return None,
    };
    let negated = (operator.kind() == SyntaxKind::EQ_EQ) != literal;
    let value_span = significant_span(&value);
    let value_text = text_within(&value, value_span);
    let replacement = match (negated, binds_as_one(&value)) {
        (false, _) => value_text,
        (true, true) => format!("not {value_text}"),
        (true, false) => format!("not ({value_text})"),
    };
    let span = significant_span(binary);
    let mut diagnostic = Diagnostic::in_file(
        Code::BoolComparison,
        file,
        format!("comparison with the Bool literal `{literal}`"),
    );
    diagnostic = match quotable(&replacement) {
        Some(quoted) => diagnostic.at(span, format!("the same as `{quoted}`")),
        None if negated => diagnostic.at(span, "the same as `not` the value"),
        None => diagnostic.at(span, "the same as the value alone"),
    };
    let keeps_comments = binary
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| is_comment(token.kind()))
        .all(|token| within(token_span(&token), value_span));
    if keeps_comments {
        if let Some(quoted) = quotable(&replacement) {
            diagnostic = diagnostic.with_repair(format!("Replace the comparison with `{quoted}`."));
        }
        diagnostic = diagnostic.with_edit(span, replacement, Applicability::MachineApplicable);
    }
    Some(diagnostic)
}

fn bool_literal(node: &SyntaxNode) -> Option<bool> {
    if node.kind() != SyntaxKind::LITERAL {
        return None;
    }
    match first_token(node)?.kind() {
        SyntaxKind::TRUE_KW => Some(true),
        SyntaxKind::FALSE_KW => Some(false),
        _ => None,
    }
}

/// Whether `not` applied to the expression's text applies to all of it:
/// names, literals, calls, field and index access, `?`, and parentheses.
fn binds_as_one(expression: &SyntaxNode) -> bool {
    matches!(
        expression.kind(),
        SyntaxKind::NAME_REF
            | SyntaxKind::LITERAL
            | SyntaxKind::CALL_EXPR
            | SyntaxKind::FIELD_ACCESS
            | SyntaxKind::INDEX_EXPR
            | SyntaxKind::TRY_EXPR
            | SyntaxKind::TUPLE_EXPR
    )
}

fn is_comment(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT | SyntaxKind::MODULE_DOC_COMMENT
    )
}

fn parent_kind(node: &SyntaxNode) -> Option<SyntaxKind> {
    node.parent().map(|parent| parent.kind())
}

fn significant_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
}

fn first_token(node: &SyntaxNode) -> Option<SyntaxToken> {
    significant_tokens(node).next()
}

fn token_span(token: &SyntaxToken) -> Span {
    let range = token.text_range();
    Span::new(range.start().into(), range.end().into())
}

/// From the node's first token to its last, leaving out whitespace and
/// comments at either end.
fn significant_span(node: &SyntaxNode) -> Span {
    let mut tokens = significant_tokens(node).map(|token| token_span(&token));
    let first = tokens.next().unwrap_or_else(|| {
        let range = node.text_range();
        Span::new(range.start().into(), range.start().into())
    });
    let last = tokens.last().unwrap_or(first);
    Span::new(first.start, last.end)
}

fn within(inner: Span, outer: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// The text of the node's tokens inside `span`.
fn text_within(node: &SyntaxNode, span: Span) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| within(token_span(token), span))
        .map(|token| token.text().to_owned())
        .collect()
}

/// `text` when it reads well inside backticks: one short line.
fn quotable(text: &str) -> Option<&str> {
    (!text.contains('\n') && text.chars().count() <= QUOTE_LIMIT).then_some(text)
}

/// The node's tokens without whitespace, newlines or comments.
fn significant_text(node: &SyntaxNode) -> Vec<String> {
    significant_tokens(node)
        .map(|token| token.text().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<Diagnostic> {
        let parse = mesh_parser::parse(source);
        lint(&parse, &SourceFile::new("main.mpl", source))
    }

    /// `(code, line)` for every finding.
    fn findings(source: &str) -> Vec<(&'static str, u32)> {
        run(source)
            .iter()
            .map(|d| (d.code().id(), d.line().expect("a finding has a line")))
            .collect()
    }

    /// `source` with `diagnostic`'s edits applied.
    fn apply(source: &str, diagnostic: &Diagnostic) -> String {
        let mut text = source.to_owned();
        let mut edits = diagnostic.edits().to_vec();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.span().start));
        for edit in edits {
            let span = edit.span();
            text.replace_range(span.start as usize..span.end as usize, edit.replacement());
        }
        text
    }

    fn text_at(source: &str, span: Span) -> &str {
        &source[span.start as usize..span.end as usize]
    }

    #[test]
    fn deep_nesting_reports_the_first_level_past_the_limit_once() {
        let source = "\
fn main() do
  case a() do
    Ok(b) -> case b do
      Ok(c) -> for d in c do
        if d > 0 do
          case d do
            1 -> if true do 1 else 2 end
            _ -> 0
          end
        end
      end
      Err(_) -> []
    end
    Err(_) -> []
  end
end
";
        assert_eq!(findings(source), vec![("L0001", 6)]);
        let found = run(source);
        assert_eq!(
            found[0].message(),
            "`case` is nested 5 levels deep (at most 4)"
        );
    }

    #[test]
    fn four_levels_pass() {
        let source = "\
fn main() do
  for x in xs do
    case x do
      Some(y) -> if y > 0 do
        List.map(y, fn z -> z end)
      end
      None -> []
    end
  end
end
";
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn closures_and_loops_count_but_else_if_chains_do_not() {
        let deep = "\
fn main() do
  List.each(xs) do |x|
    while true do
      receive do
        m -> List.map(m, fn y -> case y do
          _ -> 0
        end end)
      end
    end
  end
end
";
        assert_eq!(findings(deep), vec![("L0001", 5)]);

        let chain = "\
fn main() do
  if a do
    1
  else if b do
    2
  else if c do
    if d do
      if e do
        if f do
          3
        end
      end
    end
  else
    4
  end
end
";
        assert_eq!(findings(chain), vec![]);
    }

    #[test]
    fn every_definition_starts_over() {
        let source = "\
service Store do
  fn init() -> Int do
    0
  end

  call Get() :: Int do |state|
    if a do
      if b do
        if c do
          if d do
            (state, state)
          end
        end
      end
    end
  end
end

actor worker() do
  receive do
    m -> if a do
      if b do
        if c do
          m
        end
      end
    end
  end
end

describe(\"group\") do
  setup() do
    if a do
      if b do
        if c do
          if d do
            1
          end
        end
      end
    end
  end

  test(\"inner\") do
    if a do
      if b do
        if c do
          if d do
            assert(true)
          end
        end
      end
    end
  end
end
";
        assert_eq!(findings(source), vec![]);
    }

    /// A `test` call that is not a statement of a test file (here, bound by
    /// a `let` in a function) is an ordinary call with a trailing closure.
    #[test]
    fn a_nested_test_call_does_not_start_over() {
        let source = "\
fn f(a, b, c) do
  let t = test(\"x\") do
    if a do
      if b do
        if c do
          if a do
            1
          end
        end
      end
    end
  end
  t
end
";
        assert_eq!(findings(source), vec![("L0001", 6)]);
    }

    #[test]
    fn collapsible_else_if_needs_a_lone_if() {
        let source = "\
fn f(a, b) do
  if a do
    1
  else
    if b do
      2
    else
      3
    end
  end
end

fn g(a, b) do
  if a do
    1
  else
    log(b)
    if b do
      2
    else
      3
    end
  end
end
";
        assert_eq!(findings(source), vec![("L0002", 4)]);
    }

    #[test]
    fn pass_through_arm_matches_tokens_not_layout() {
        let source = "\
fn f(r) do
  case r do
    Ok(value) -> Ok(value)
    Err(Some( e )) when e > 0 ->
      Err(Some(e))
    Err(None) -> Err(None)
    other -> other
  end
end

fn g(r) do
  case r do
    Ok(value)
    Err(e) -> Err(String.length(e))
  end
end

fn h(p) do
  case p do
    (0, b) -> (0, b)
    [a] -> [a]
    Some((x, y)) -> Some((x, y))
    _ -> p
  end
end
";
        assert_eq!(
            findings(source),
            vec![("L0003", 3), ("L0003", 4), ("L0003", 6), ("L0003", 7),]
        );
    }

    #[test]
    fn a_test_file_using_assert_receive_parses() {
        // It was a parse error: "expected a newline or `;` after the statement".
        let source = "test(\"receive\") do\n  send(self(), 42)\n  assert_receive 42, 500\nend\n";
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn a_describe_with_bare_setup_parses() {
        // `setup do` was a parse error: "expected a newline or `;` after the
        // statement".
        let source = "describe(\"g\") do\n  setup do\n    let x = 1\n  end\n  test(\"t\") do\n    assert(x == 1)\n  end\nend\n";
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn bool_comparison_flags_literals_only() {
        let source = "\
fn f(flag, other) do
  let a = flag == true
  let b = false != flag
  let c = flag == other
  a and b and c
end
";
        assert_eq!(findings(source), vec![("L0004", 2), ("L0004", 3)]);
    }

    #[test]
    fn a_parse_with_errors_gets_no_findings() {
        let source = "fn main(flag) do\n  let = 1\n  flag == true\nend\n";
        assert!(!mesh_parser::parse(source).ok());
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn findings_are_catalog_warnings_in_their_file() {
        let source = "fn f(flag) do\n  flag == true\nend\n";
        let found = run(source);
        assert_eq!(found.len(), 1);
        let diagnostic = &found[0];
        assert_eq!(diagnostic.code(), Code::BoolComparison);
        assert_eq!(diagnostic.severity(), mesh_diagnostics::Severity::Warning);
        assert_eq!(
            diagnostic.file().map(|file| file.path()),
            Some(std::path::Path::new("main.mpl"))
        );
    }

    #[test]
    fn deep_nesting_points_at_the_keyword_and_labels_each_level() {
        let source = "\
fn main(n) do
  if n > 0 do
    while n > 1 do
      case n do
        _ -> List.each([n]) do |m|
          for x in [m] do
            x
          end
        end
      end
    end
  end
end
";
        let found = run(source);
        assert_eq!(found.len(), 1);
        let diagnostic = &found[0];
        assert_eq!(diagnostic.code(), Code::DeepNesting);
        assert_eq!(text_at(source, diagnostic.span().unwrap()), "for");
        assert_eq!(diagnostic.line(), Some(6));
        assert_eq!(diagnostic.primary_label(), Some("level 5"));
        let levels: Vec<_> = diagnostic
            .labels()
            .iter()
            .map(|label| (text_at(source, label.span()), label.message()))
            .collect();
        assert_eq!(
            levels,
            [
                ("if", "level 1"),
                ("while", "level 2"),
                ("case", "level 3"),
                ("do", "level 4"),
            ]
        );
        assert!(diagnostic
            .repair()
            .starts_with("Move this `for` into a function"));
        assert!(diagnostic.edits().is_empty());
    }

    #[test]
    fn collapsible_else_if_points_at_the_else_and_its_if() {
        let source = "fn f(a, b) do\n  if a do\n    1\n  else\n    if b do\n      2\n    else\n      3\n    end\n  end\nend\n";
        let found = run(source);
        let diagnostic = &found[0];
        assert_eq!(diagnostic.code(), Code::CollapsibleElseIf);
        assert_eq!(diagnostic.message(), "this `else` holds only an `if`");
        assert_eq!(text_at(source, diagnostic.span().unwrap()), "else");
        assert_eq!(diagnostic.line(), Some(4));
        let label = &diagnostic.labels()[0];
        assert_eq!(text_at(source, label.span()), "if");
        assert_eq!(label.message(), "holds only this `if`");
        assert_eq!(diagnostic.repair(), Code::CollapsibleElseIf.info().repair);
    }

    #[test]
    fn pass_through_arm_spans_the_arm_and_names_what_to_delete() {
        let source = "fn f(r) do\n  case r do\n    Ok(value) -> Ok(value)\n    Err(e) when e > 0 ->\n      Err(e)\n    Err(e) -> Err(0)\n  end\nend\n";
        let found = run(source);
        let arms: Vec<_> = found
            .iter()
            .map(|d| (text_at(source, d.span().unwrap()), d.repair()))
            .collect();
        assert_eq!(
            arms,
            [
                (
                    "Ok(value) -> Ok(value)",
                    "Delete `-> Ok(value)` and leave `Ok(value)` on its own as the arm."
                ),
                (
                    "Err(e) when e > 0 ->\n      Err(e)",
                    "Delete `-> Err(e)` and leave `Err(e) when e > 0` on its own as the arm."
                ),
            ]
        );
    }

    /// `(comparison, what replaces it)`, each compared inside a function.
    const BOOL_COMPARISONS: &[(&str, &str)] = &[
        ("flag == true", "flag"),
        ("true == flag", "flag"),
        ("flag != false", "flag"),
        ("flag == false", "not flag"),
        ("false == flag", "not flag"),
        ("flag != true", "not flag"),
        ("ready(flag) == false", "not ready(flag)"),
        ("flag.done != true", "not flag.done"),
        ("(flag and other) == false", "not (flag and other)"),
        ("other < flag == false", "not (other < flag)"),
        ("other < flag == true", "other < flag"),
        ("(flag) == true", "(flag)"),
        ("true == false", "not true"),
    ];

    #[test]
    fn bool_comparison_spans_the_comparison_and_edits_it_safely() {
        for (comparison, replacement) in BOOL_COMPARISONS {
            let source = format!("fn f(flag, other) do\n  {comparison}\nend\n");
            let found = run(&source);
            assert_eq!(found.len(), 1, "{comparison}");
            let diagnostic = &found[0];
            assert_eq!(text_at(&source, diagnostic.span().unwrap()), *comparison);
            assert_eq!(
                diagnostic.primary_label(),
                Some(format!("the same as `{replacement}`").as_str())
            );
            assert_eq!(
                diagnostic.repair(),
                format!("Replace the comparison with `{replacement}`.")
            );
            let [edit] = diagnostic.edits() else {
                panic!("{comparison}: {:?}", diagnostic.edits());
            };
            assert_eq!(edit.applicability(), Applicability::MachineApplicable);
            assert_eq!(edit.span(), diagnostic.span().unwrap());
            assert_eq!(edit.replacement(), *replacement);
        }
    }

    /// The node at `span` that is an expression of `kind`, if one is.
    fn node_at(source: &str, span: Span, kind: SyntaxKind) -> Option<SyntaxNode> {
        mesh_parser::parse(source)
            .syntax()
            .descendants()
            .find(|node| node.kind() == kind && significant_span(node) == span)
    }

    /// Wherever the comparison stands, its edit parses as the operand it
    /// kept, or as `not` applied to all of that operand: the comparison's
    /// meaning.
    #[test]
    fn bool_comparison_edits_keep_their_meaning_in_context() {
        let contexts = [
            "let x = other and flag == false or other",
            "let x = flag == false == other",
            "let x = other == (flag != true)",
            "let x = -flag == false",
            "let x = not flag == false",
            "let x = flag == false |> other",
            "let x = [flag == false, other]",
            "if f(flag) == false do\n    1\n  end",
            "let x = other < flag == false and other",
            "let x = other + flag == true",
            "let x = true != other.done and flag",
        ];
        for context in contexts {
            let source = format!("fn f(flag, other) do\n  {context}\nend\n");
            let comparisons: Vec<_> = run(&source)
                .into_iter()
                .filter(|d| d.code() == Code::BoolComparison)
                .collect();
            assert!(!comparisons.is_empty(), "{context}");
            for diagnostic in comparisons {
                let span = diagnostic.span().unwrap();
                let binary = node_at(&source, span, SyntaxKind::BINARY_EXPR).unwrap();
                let value = binary
                    .children()
                    .find(|operand| bool_literal(operand).is_none())
                    .unwrap_or_else(|| binary.first_child().unwrap());
                let value_text = significant_text(&value);

                let fixed = apply(&source, &diagnostic);
                assert!(
                    mesh_parser::parse(&fixed).ok(),
                    "{context} became {fixed:?}"
                );
                let replacement = diagnostic.edits()[0].replacement();
                let edited = Span::new(span.start, span.start + replacement.len() as u32);
                let kept = match replacement.strip_prefix("not ") {
                    None => node_at(&fixed, edited, value.kind()),
                    Some(_) => node_at(&fixed, edited, SyntaxKind::UNARY_EXPR)
                        .and_then(|not| not.first_child())
                        .map(|operand| match operand.kind() {
                            SyntaxKind::TUPLE_EXPR if value.kind() != SyntaxKind::TUPLE_EXPR => {
                                operand.first_child().unwrap()
                            }
                            _ => operand,
                        }),
                };
                assert!(
                    kept.is_some_and(|kept| kept.kind() == value.kind()
                        && significant_text(&kept) == value_text),
                    "{context}: `{replacement}` does not keep the comparison's meaning in {fixed:?}"
                );
            }
        }
    }

    #[test]
    fn a_comment_inside_a_bool_comparison_blocks_its_edit() {
        let source = "fn f(flag) do\n  let x = flag == # always\n    true\n  x\nend\n";
        let found = run(source);
        assert_eq!(found.len(), 1);
        assert!(found[0].edits().is_empty());
        assert_eq!(found[0].repair(), Code::BoolComparison.info().repair);

        let source =
            "fn f(flag) do\n  let x = ready(\n    flag, # the input\n  ) == true\n  x\nend\n";
        let found = run(source);
        assert_eq!(
            found[0].edits()[0].replacement(),
            "ready(\n    flag, # the input\n  )"
        );
        assert_eq!(
            found[0].primary_label(),
            Some("the same as the value alone")
        );
    }

    #[test]
    fn the_bool_comparison_example_is_fixed_by_its_edit() {
        let example = Code::BoolComparison.info().example.unwrap();
        let found = run(example.bad);
        assert_eq!(found.len(), 1);
        assert_eq!(apply(example.bad, &found[0]), example.fixed);
    }
}
