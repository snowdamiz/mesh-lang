//! Pattern matrix to decision tree compiler.
//!
//! Implements Maranget's algorithm for compiling pattern matrices into
//! efficient decision trees. The algorithm works by:
//!
//! 1. Representing match arms as a pattern matrix (rows = arms, columns = positions)
//! 2. Selecting the column with the most constructor diversity
//! 3. Specializing the matrix for each constructor
//! 4. Recursing on specialized sub-matrices
//! 5. Producing Leaf nodes when all patterns are wildcards/variables

use crate::mir::{MirExpr, MirLiteral, MirMatchArm, MirModule, MirPattern, MirType};
use crate::pattern::{AccessPath, ConstructorTag, DecisionTree};

/// Compile a single match expression into a decision tree.
///
/// Takes the scrutinee type, a list of match arms, and source location
/// information for generating Fail nodes.
pub fn compile_match(
    _scrutinee_ty: &MirType,
    _arms: &[MirMatchArm],
    _file: &str,
    _line: u32,
) -> DecisionTree {
    todo!("compile_match: pattern compilation not yet implemented")
}

/// Walk all `MirExpr::Match` nodes in a module and compile them to
/// `MirExpr::CompiledMatch` with decision trees.
pub fn compile_patterns(module: &mut MirModule) {
    for func in &mut module.functions {
        compile_expr_patterns(&mut func.body);
    }
}

/// Recursively compile match expressions within an expression tree.
fn compile_expr_patterns(_expr: &mut MirExpr) {
    todo!("compile_expr_patterns: pattern compilation not yet implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirLiteral, MirMatchArm, MirPattern, MirType};
    use crate::pattern::{AccessPath, DecisionTree};

    // ── Helper functions ─────────────────────────────────────────────

    fn make_arm(pattern: MirPattern, guard: Option<MirExpr>, body: MirExpr) -> MirMatchArm {
        MirMatchArm {
            pattern,
            guard,
            body,
        }
    }

    fn int_body(n: i64) -> MirExpr {
        MirExpr::IntLit(n, MirType::Int)
    }

    fn string_body(s: &str) -> MirExpr {
        MirExpr::StringLit(s.to_string(), MirType::String)
    }

    fn var_expr(name: &str, ty: MirType) -> MirExpr {
        MirExpr::Var(name.to_string(), ty)
    }

    // ── Test 1: Single wildcard arm ──────────────────────────────────

    #[test]
    fn test_wildcard_arm() {
        // match x { _ -> 1 }
        // Expected: Leaf { arm_index: 0, bindings: [] }
        let arms = vec![make_arm(MirPattern::Wildcard, None, int_body(1))];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match tree {
            DecisionTree::Leaf {
                arm_index,
                bindings,
            } => {
                assert_eq!(arm_index, 0);
                assert!(bindings.is_empty());
            }
            other => panic!("Expected Leaf, got {:?}", other),
        }
    }

    // ── Test 2: Variable binding ─────────────────────────────────────

    #[test]
    fn test_variable_binding() {
        // match x { y -> y }
        // Expected: Leaf { arm_index: 0, bindings: [(y, Int, Root)] }
        let arms = vec![make_arm(
            MirPattern::Var("y".to_string(), MirType::Int),
            None,
            var_expr("y", MirType::Int),
        )];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match tree {
            DecisionTree::Leaf {
                arm_index,
                bindings,
            } => {
                assert_eq!(arm_index, 0);
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].0, "y");
                assert_eq!(bindings[0].1, MirType::Int);
                assert_eq!(bindings[0].2, AccessPath::Root);
            }
            other => panic!("Expected Leaf, got {:?}", other),
        }
    }

    // ── Test 3: Integer literal tests ────────────────────────────────

    #[test]
    fn test_integer_literals() {
        // match x { 1 -> "one", 2 -> "two", _ -> "other" }
        // Expected: Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(1), Leaf(2)))
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::Int(1)),
                None,
                string_body("one"),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::Int(2)),
                None,
                string_body("two"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("other")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        // Should be Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(1), Leaf(2)))
        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                success,
                failure,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert!(matches!(value, MirLiteral::Int(1)));
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
                // failure should be another Test for literal 2
                match failure.as_ref() {
                    DecisionTree::Test {
                        scrutinee_path: path2,
                        value: val2,
                        success: s2,
                        failure: f2,
                    } => {
                        assert_eq!(*path2, AccessPath::Root);
                        assert!(matches!(val2, MirLiteral::Int(2)));
                        assert!(matches!(
                            s2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 1,
                                ..
                            }
                        ));
                        assert!(matches!(
                            f2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 2,
                                ..
                            }
                        ));
                    }
                    other => panic!("Expected nested Test, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 4: Boolean literal tests ────────────────────────────────

    #[test]
    fn test_boolean_literals() {
        // match flag { true -> 1, false -> 0 }
        // Expected: Test(Root, Bool(true), Leaf(0), Leaf(1))
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::Bool(true)),
                None,
                int_body(1),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::Bool(false)),
                None,
                int_body(0),
            ),
        ];

        let tree = compile_match(&MirType::Bool, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                success,
                failure,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert!(matches!(value, MirLiteral::Bool(true)));
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
                assert!(matches!(
                    failure.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 1,
                        ..
                    }
                ));
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 5: Constructor switch ───────────────────────────────────

    #[test]
    fn test_constructor_switch() {
        // match shape { Circle(r) -> r, Rectangle(w, h) -> w * h }
        // Expected: Switch(Root, [(Circle/0, Leaf(0, [(r, Float, VariantField(Root, "Circle", 0))])),
        //                         (Rectangle/1, Leaf(1, [...]))])
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Shape".to_string(),
                    variant: "Circle".to_string(),
                    fields: vec![MirPattern::Var("r".to_string(), MirType::Float)],
                    bindings: vec![("r".to_string(), MirType::Float)],
                },
                None,
                var_expr("r", MirType::Float),
            ),
            make_arm(
                MirPattern::Constructor {
                    type_name: "Shape".to_string(),
                    variant: "Rectangle".to_string(),
                    fields: vec![
                        MirPattern::Var("w".to_string(), MirType::Float),
                        MirPattern::Var("h".to_string(), MirType::Float),
                    ],
                    bindings: vec![
                        ("w".to_string(), MirType::Float),
                        ("h".to_string(), MirType::Float),
                    ],
                },
                None,
                var_expr("w", MirType::Float),
            ),
        ];

        let tree = compile_match(&MirType::SumType("Shape".to_string()), &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                default,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert_eq!(cases.len(), 2);

                // First case: Circle
                assert_eq!(cases[0].0.variant_name, "Circle");
                assert_eq!(cases[0].0.tag, 0);
                match &cases[0].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "r");
                        assert_eq!(
                            bindings[0].2,
                            AccessPath::VariantField(
                                Box::new(AccessPath::Root),
                                "Circle".to_string(),
                                0
                            )
                        );
                    }
                    other => panic!("Expected Leaf for Circle, got {:?}", other),
                }

                // Second case: Rectangle
                assert_eq!(cases[1].0.variant_name, "Rectangle");
                assert_eq!(cases[1].0.tag, 1);
                match &cases[1].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 1);
                        assert_eq!(bindings.len(), 2);
                        assert_eq!(bindings[0].0, "w");
                        assert_eq!(bindings[1].0, "h");
                    }
                    other => panic!("Expected Leaf for Rectangle, got {:?}", other),
                }

                assert!(default.is_none());
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 6: Nested patterns (tuple + constructor) ────────────────

    #[test]
    fn test_nested_tuple_constructor() {
        // match pair { (Some(x), _) -> x, (None, y) -> y }
        // Expected: Switch on TupleField(Root, 0) for Some/None tags
        let arms = vec![
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Constructor {
                        type_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        fields: vec![MirPattern::Var("x".to_string(), MirType::Int)],
                        bindings: vec![("x".to_string(), MirType::Int)],
                    },
                    MirPattern::Wildcard,
                ]),
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Constructor {
                        type_name: "Option".to_string(),
                        variant: "None".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                    MirPattern::Var("y".to_string(), MirType::Int),
                ]),
                None,
                var_expr("y", MirType::Int),
            ),
        ];

        let scrutinee_ty = MirType::Tuple(vec![
            MirType::SumType("Option".to_string()),
            MirType::Int,
        ]);
        let tree = compile_match(&scrutinee_ty, &arms, "test.snow", 1);

        // The tree should switch on TupleField(Root, 0) for constructor tag
        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                ..
            } => {
                assert_eq!(
                    *scrutinee_path,
                    AccessPath::TupleField(Box::new(AccessPath::Root), 0)
                );
                assert!(cases.len() >= 2);

                // Some case should bind x from inside the variant
                let some_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Some")
                    .expect("Should have Some case");
                match &some_case.1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        // Should bind x from VariantField(TupleField(Root, 0), "Some", 0)
                        let x_binding = bindings.iter().find(|(name, _, _)| name == "x");
                        assert!(x_binding.is_some(), "Should bind x");
                    }
                    other => panic!("Expected Leaf for Some case, got {:?}", other),
                }

                // None case should bind y from TupleField(Root, 1)
                let none_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "None")
                    .expect("Should have None case");
                match &none_case.1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 1);
                        let y_binding = bindings.iter().find(|(name, _, _)| name == "y");
                        assert!(y_binding.is_some(), "Should bind y");
                    }
                    other => panic!("Expected Leaf for None case, got {:?}", other),
                }
            }
            other => panic!("Expected Switch on tuple field, got {:?}", other),
        }
    }

    // ── Test 7: Or-patterns (duplicate arms) ─────────────────────────

    #[test]
    fn test_or_pattern_duplicates_arms() {
        // match x { 1 | 2 -> "small", _ -> "big" }
        // Expected: Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(0_dup), Leaf(1)))
        // Both literal matches should point to arm_index 0
        let arms = vec![
            make_arm(
                MirPattern::Or(vec![
                    MirPattern::Literal(MirLiteral::Int(1)),
                    MirPattern::Literal(MirLiteral::Int(2)),
                ]),
                None,
                string_body("small"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("big")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        // Should be Test(Root, 1, Leaf(0), Test(Root, 2, Leaf(0), Leaf(1)))
        match &tree {
            DecisionTree::Test {
                value, success, failure, ..
            } => {
                assert!(matches!(value, MirLiteral::Int(1)));
                match success.as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf(0) for first or-alt, got {:?}", other),
                }
                match failure.as_ref() {
                    DecisionTree::Test {
                        value: v2,
                        success: s2,
                        failure: f2,
                        ..
                    } => {
                        assert!(matches!(v2, MirLiteral::Int(2)));
                        match s2.as_ref() {
                            DecisionTree::Leaf { arm_index, .. } => {
                                assert_eq!(*arm_index, 0, "Or-pattern should share arm_index 0")
                            }
                            other => {
                                panic!("Expected Leaf(0) for second or-alt, got {:?}", other)
                            }
                        }
                        match f2.as_ref() {
                            DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 1),
                            other => panic!("Expected Leaf(1) for default, got {:?}", other),
                        }
                    }
                    other => panic!("Expected nested Test for second literal, got {:?}", other),
                }
            }
            other => panic!("Expected Test at root, got {:?}", other),
        }
    }

    // ── Test 8: Guard expression ─────────────────────────────────────

    #[test]
    fn test_guard_expression() {
        // match x { n when n > 0 -> "positive", _ -> "non-positive" }
        // Expected: Guard(guard_expr, Leaf(0, [(n, Int, Root)]), Leaf(1))
        let guard = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard),
                string_body("positive"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("non-positive")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Guard {
                success, failure, ..
            } => {
                match success.as_ref() {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "n");
                        assert_eq!(bindings[0].2, AccessPath::Root);
                    }
                    other => panic!("Expected Leaf for guard success, got {:?}", other),
                }
                match failure.as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => {
                        assert_eq!(*arm_index, 1);
                    }
                    other => panic!("Expected Leaf for guard failure, got {:?}", other),
                }
            }
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 9: Guard with Fail fallback ─────────────────────────────

    #[test]
    fn test_guard_with_fail_fallback() {
        // match x { n when n > 0 -> "positive" }
        // Only guarded arm, no default => Fail node on guard failure
        let guard = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![make_arm(
            MirPattern::Var("n".to_string(), MirType::Int),
            Some(guard),
            string_body("positive"),
        )];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Guard {
                success, failure, ..
            } => {
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
                match failure.as_ref() {
                    DecisionTree::Fail { message, .. } => {
                        assert!(message.contains("non-exhaustive"));
                    }
                    other => panic!("Expected Fail for guard failure, got {:?}", other),
                }
            }
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 10: Constructor with wildcard default ────────────────────

    #[test]
    fn test_constructor_with_wildcard_default() {
        // match opt { Some(x) -> x, _ -> 0 }
        // Expected: Switch(Root, [(Some, Leaf(0, [x]))], default=Leaf(1))
        let arms = vec![
            make_arm(
                MirPattern::Constructor {
                    type_name: "Option".to_string(),
                    variant: "Some".to_string(),
                    fields: vec![MirPattern::Var("x".to_string(), MirType::Int)],
                    bindings: vec![("x".to_string(), MirType::Int)],
                },
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(
            &MirType::SumType("Option".to_string()),
            &arms,
            "test.snow",
            1,
        );

        match &tree {
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                default,
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                assert_eq!(cases.len(), 1);
                assert_eq!(cases[0].0.variant_name, "Some");
                match &cases[0].1 {
                    DecisionTree::Leaf {
                        arm_index,
                        bindings,
                    } => {
                        assert_eq!(*arm_index, 0);
                        assert_eq!(bindings.len(), 1);
                        assert_eq!(bindings[0].0, "x");
                    }
                    other => panic!("Expected Leaf for Some, got {:?}", other),
                }
                assert!(default.is_some());
                match default.as_ref().unwrap().as_ref() {
                    DecisionTree::Leaf { arm_index, .. } => {
                        assert_eq!(*arm_index, 1);
                    }
                    other => panic!("Expected Leaf for default, got {:?}", other),
                }
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }

    // ── Test 11: Tuple pattern ───────────────────────────────────────

    #[test]
    fn test_tuple_pattern() {
        // match pair { (1, y) -> y, (x, 2) -> x, _ -> 0 }
        // Expected: Tests on TupleField(Root, 0) and TupleField(Root, 1)
        let arms = vec![
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Literal(MirLiteral::Int(1)),
                    MirPattern::Var("y".to_string(), MirType::Int),
                ]),
                None,
                var_expr("y", MirType::Int),
            ),
            make_arm(
                MirPattern::Tuple(vec![
                    MirPattern::Var("x".to_string(), MirType::Int),
                    MirPattern::Literal(MirLiteral::Int(2)),
                ]),
                None,
                var_expr("x", MirType::Int),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let scrutinee_ty = MirType::Tuple(vec![MirType::Int, MirType::Int]);
        let tree = compile_match(&scrutinee_ty, &arms, "test.snow", 1);

        // Should test tuple fields, not Root
        fn contains_tuple_field_test(tree: &DecisionTree) -> bool {
            match tree {
                DecisionTree::Test {
                    scrutinee_path,
                    failure,
                    success,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _))
                        || contains_tuple_field_test(success)
                        || contains_tuple_field_test(failure)
                }
                DecisionTree::Switch {
                    scrutinee_path,
                    cases,
                    default,
                    ..
                } => {
                    matches!(scrutinee_path, AccessPath::TupleField(_, _))
                        || cases.iter().any(|(_, t)| contains_tuple_field_test(t))
                        || default
                            .as_ref()
                            .map_or(false, |d| contains_tuple_field_test(d))
                }
                DecisionTree::Guard {
                    success, failure, ..
                } => contains_tuple_field_test(success) || contains_tuple_field_test(failure),
                _ => false,
            }
        }

        assert!(
            contains_tuple_field_test(&tree),
            "Decision tree should test tuple fields, got {:?}",
            tree
        );
    }

    // ── Test 12: String literal test ─────────────────────────────────

    #[test]
    fn test_string_literals() {
        // match s { "hello" -> 1, "world" -> 2, _ -> 0 }
        let arms = vec![
            make_arm(
                MirPattern::Literal(MirLiteral::String("hello".to_string())),
                None,
                int_body(1),
            ),
            make_arm(
                MirPattern::Literal(MirLiteral::String("world".to_string())),
                None,
                int_body(2),
            ),
            make_arm(MirPattern::Wildcard, None, int_body(0)),
        ];

        let tree = compile_match(&MirType::String, &arms, "test.snow", 1);

        match &tree {
            DecisionTree::Test {
                scrutinee_path,
                value,
                ..
            } => {
                assert_eq!(*scrutinee_path, AccessPath::Root);
                match value {
                    MirLiteral::String(s) => assert_eq!(s, "hello"),
                    other => panic!("Expected String literal, got {:?}", other),
                }
            }
            other => panic!("Expected Test, got {:?}", other),
        }
    }

    // ── Test 13: Multiple guards ─────────────────────────────────────

    #[test]
    fn test_multiple_guards() {
        // match x { n when n > 0 -> "pos", n when n < 0 -> "neg", _ -> "zero" }
        // Each guard should chain: Guard -> Guard -> Leaf
        let guard_pos = MirExpr::BinOp {
            op: crate::mir::BinOp::Gt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };
        let guard_neg = MirExpr::BinOp {
            op: crate::mir::BinOp::Lt,
            lhs: Box::new(var_expr("n", MirType::Int)),
            rhs: Box::new(int_body(0)),
            ty: MirType::Bool,
        };

        let arms = vec![
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard_pos),
                string_body("pos"),
            ),
            make_arm(
                MirPattern::Var("n".to_string(), MirType::Int),
                Some(guard_neg),
                string_body("neg"),
            ),
            make_arm(MirPattern::Wildcard, None, string_body("zero")),
        ];

        let tree = compile_match(&MirType::Int, &arms, "test.snow", 1);

        // Should be Guard(pos_guard, Leaf(0), Guard(neg_guard, Leaf(1), Leaf(2)))
        match &tree {
            DecisionTree::Guard {
                success,
                failure: first_failure,
                ..
            } => {
                assert!(matches!(
                    success.as_ref(),
                    DecisionTree::Leaf {
                        arm_index: 0,
                        ..
                    }
                ));
                match first_failure.as_ref() {
                    DecisionTree::Guard {
                        success: s2,
                        failure: f2,
                        ..
                    } => {
                        assert!(matches!(
                            s2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 1,
                                ..
                            }
                        ));
                        assert!(matches!(
                            f2.as_ref(),
                            DecisionTree::Leaf {
                                arm_index: 2,
                                ..
                            }
                        ));
                    }
                    other => panic!("Expected nested Guard, got {:?}", other),
                }
            }
            other => panic!("Expected Guard, got {:?}", other),
        }
    }

    // ── Test 14: Or-pattern with constructors ────────────────────────

    #[test]
    fn test_or_pattern_constructors() {
        // match color { Red | Blue -> "cool", Green -> "warm" }
        // Or-patterns on constructors should expand and both lead to arm_index 0
        let arms = vec![
            make_arm(
                MirPattern::Or(vec![
                    MirPattern::Constructor {
                        type_name: "Color".to_string(),
                        variant: "Red".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                    MirPattern::Constructor {
                        type_name: "Color".to_string(),
                        variant: "Blue".to_string(),
                        fields: vec![],
                        bindings: vec![],
                    },
                ]),
                None,
                string_body("cool"),
            ),
            make_arm(
                MirPattern::Constructor {
                    type_name: "Color".to_string(),
                    variant: "Green".to_string(),
                    fields: vec![],
                    bindings: vec![],
                },
                None,
                string_body("warm"),
            ),
        ];

        let tree = compile_match(
            &MirType::SumType("Color".to_string()),
            &arms,
            "test.snow",
            1,
        );

        match &tree {
            DecisionTree::Switch { cases, .. } => {
                // Red and Blue should both have arm_index 0, Green has arm_index 1
                let red_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Red")
                    .expect("Should have Red case");
                match &red_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf for Red, got {:?}", other),
                }

                let blue_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Blue")
                    .expect("Should have Blue case");
                match &blue_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 0),
                    other => panic!("Expected Leaf for Blue, got {:?}", other),
                }

                let green_case = cases
                    .iter()
                    .find(|(tag, _)| tag.variant_name == "Green")
                    .expect("Should have Green case");
                match &green_case.1 {
                    DecisionTree::Leaf { arm_index, .. } => assert_eq!(*arm_index, 1),
                    other => panic!("Expected Leaf for Green, got {:?}", other),
                }
            }
            other => panic!("Expected Switch, got {:?}", other),
        }
    }
}
