//! Mesh linter: code the compiler accepts but a reviewer should not.
//!
//! Every rule reads the lossless CST from `mesh-parser`, so linting needs no
//! type information and works on any source that parses.

use mesh_parser::{ParseError, SyntaxKind, SyntaxNode, SyntaxToken};

/// Control-flow nesting a function body may use before `deep-nesting` fires.
pub const MAX_NESTING: usize = 4;

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    /// Rule name, such as `deep-nesting`.
    pub rule: &'static str,
    pub message: String,
    /// Byte offset of the code the finding points at.
    pub offset: u32,
}

/// Lint `source`, or return its first parse error.
pub fn lint(source: &str) -> Result<Vec<Lint>, ParseError> {
    let parse = mesh_parser::parse(source);
    if let Some(error) = parse.errors().first() {
        return Err(error.clone());
    }
    let root = parse.syntax();
    let mut lints = Vec::new();
    deep_nesting(&root, 0, &mut lints);
    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::ELSE_BRANCH => collapsible_else_if(&node, &mut lints),
            SyntaxKind::MATCH_ARM => pass_through_arm(&node, &mut lints),
            SyntaxKind::BINARY_EXPR => bool_comparison(&node, &mut lints),
            _ => {}
        }
    }
    lints.sort_by_key(|lint| lint.offset);
    Ok(lints)
}

/// `if`, `case`, loops, `receive` and closures each nest one level deeper,
/// counted from the enclosing definition. Only the outermost construct past
/// [`MAX_NESTING`] is reported.
fn deep_nesting(node: &SyntaxNode, depth: usize, lints: &mut Vec<Lint>) {
    for child in node.children() {
        let depth = match child.kind() {
            SyntaxKind::FN_DEF
            | SyntaxKind::ACTOR_DEF
            | SyntaxKind::CALL_HANDLER
            | SyntaxKind::CAST_HANDLER
            | SyntaxKind::TERMINATE_CLAUSE => 0,
            SyntaxKind::TRAILING_CLOSURE if is_test_block(&child) => 0,
            // `else if` continues its chain rather than nesting inside it.
            SyntaxKind::IF_EXPR if parent_kind(&child) == Some(SyntaxKind::ELSE_BRANCH) => depth,
            SyntaxKind::IF_EXPR
            | SyntaxKind::CASE_EXPR
            | SyntaxKind::FOR_IN_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::RECEIVE_EXPR
            | SyntaxKind::CLOSURE_EXPR
            | SyntaxKind::TRAILING_CLOSURE => depth + 1,
            _ => depth,
        };
        if depth > MAX_NESTING {
            let keyword = first_token(&child).map_or(String::new(), |t| t.text().to_owned());
            lints.push(Lint {
                rule: "deep-nesting",
                message: format!(
                    "`{keyword}` is nested {depth} levels deep (at most {MAX_NESTING}); \
                     extract a function, return early, or use `?`"
                ),
                offset: child.text_range().start().into(),
            });
        } else {
            deep_nesting(&child, depth, lints);
        }
    }
}

/// The body of a top-level `test`, `describe`, `setup` or `teardown` block
/// (or one inside a `describe`) is a definition of its own, like a function's.
fn is_test_block(closure: &SyntaxNode) -> bool {
    let Some(call) = closure.parent() else {
        return false;
    };
    matches!(
        callee_name(&call).as_deref(),
        Some("test" | "describe" | "setup" | "teardown")
    ) && at_test_file_top_level(&call)
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

/// `else` whose whole body is one `if`: `else if` says the same one level
/// shallower.
fn collapsible_else_if(else_branch: &SyntaxNode, lints: &mut Vec<Lint>) {
    let Some(block) = else_branch
        .children()
        .find(|c| c.kind() == SyntaxKind::BLOCK)
    else {
        return;
    };
    let mut statements = block.children();
    if let (Some(only), None) = (statements.next(), statements.next()) {
        if only.kind() == SyntaxKind::IF_EXPR {
            lints.push(Lint {
                rule: "collapsible-else-if",
                message: "this `else` holds only an `if`; write `else if`".to_owned(),
                offset: else_branch.text_range().start().into(),
            });
        }
    }
}

/// `Ok(value) -> Ok(value)` is what the arm `Ok(value)` means on its own.
fn pass_through_arm(arm: &SyntaxNode, lints: &mut Vec<Lint>) {
    let has_arrow = arm
        .children_with_tokens()
        .any(|element| element.kind() == SyntaxKind::ARROW);
    let (Some(pattern), Some(body)) = (arm.first_child(), arm.last_child()) else {
        return;
    };
    if has_arrow
        && pattern != body
        && stands_for_its_value(&pattern)
        && significant_text(&pattern) == significant_text(&body)
    {
        lints.push(Lint {
            rule: "pass-through-arm",
            message: "this arm returns exactly what it matched; write the pattern alone".to_owned(),
            offset: arm.text_range().start().into(),
        });
    }
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

/// `flag == true` is `flag`, and `flag == false` is `not flag`.
fn bool_comparison(binary: &SyntaxNode, lints: &mut Vec<Lint>) {
    let Some(operator) = binary
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| matches!(token.kind(), SyntaxKind::EQ_EQ | SyntaxKind::NOT_EQ))
    else {
        return;
    };
    let compares_bool_literal = binary.children().any(|operand| {
        operand.kind() == SyntaxKind::LITERAL
            && first_token(&operand)
                .is_some_and(|t| matches!(t.kind(), SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW))
    });
    if compares_bool_literal {
        lints.push(Lint {
            rule: "bool-comparison",
            message: "comparing with a Bool literal; use the value itself or `not`".to_owned(),
            offset: operator.text_range().start().into(),
        });
    }
}

fn parent_kind(node: &SyntaxNode) -> Option<SyntaxKind> {
    node.parent().map(|parent| parent.kind())
}

fn first_token(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| !token.kind().is_trivia())
}

/// The node's tokens without whitespace, newlines or comments.
fn significant_text(node: &SyntaxNode) -> Vec<String> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(rule, line)` for every finding.
    fn findings(source: &str) -> Vec<(&'static str, usize)> {
        lint(source)
            .expect("parses")
            .into_iter()
            .map(|lint| {
                let line = source[..lint.offset as usize].matches('\n').count() + 1;
                (lint.rule, line)
            })
            .collect()
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
        assert_eq!(findings(source), vec![("deep-nesting", 6)]);
        let message = &lint(source).unwrap()[0].message;
        assert!(
            message.starts_with("`case` is nested 5 levels deep (at most 4)"),
            "{message}"
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
        assert_eq!(findings(deep), vec![("deep-nesting", 5)]);

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
        assert_eq!(findings(source), vec![("collapsible-else-if", 4)]);
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
            vec![
                ("pass-through-arm", 3),
                ("pass-through-arm", 4),
                ("pass-through-arm", 6),
                ("pass-through-arm", 7),
            ]
        );
    }

    #[test]
    fn a_test_file_using_assert_receive_parses() {
        // It was a parse error: "expected a newline or `;` after the statement".
        let source = "test(\"receive\") do\n  send(self(), 42)\n  assert_receive 42, 500\nend\n";
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
        assert_eq!(
            findings(source),
            vec![("bool-comparison", 2), ("bool-comparison", 3)]
        );
    }

    #[test]
    fn parse_errors_are_returned() {
        assert!(lint("fn main() do\n  let = 1\nend\n").is_err());
    }
}
