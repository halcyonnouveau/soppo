use proptest::prelude::*;
use soppo::syntax::{
    BinOp, Block, Decl, EnumVariant, Expr, ExprKind, Field, File, FuncDecl, Generic, Ident,
    IntFormat, Literal, Param, Pattern, PatternKind, Span, Stmt, StmtKind, TypeAnnotation,
    TypeDecl, TypeKind, UnaryOp,
};

/// Generate a valid identifier name (starts with letter, contains alphanumeric)
fn arb_ident_name() -> impl Strategy<Value = String> {
    "[a-z][a-zA-Z0-9]{0,10}".prop_map(|s| s.to_string())
}

/// Generate a valid type name (starts with uppercase)
fn arb_type_name() -> impl Strategy<Value = String> {
    "[A-Z][a-zA-Z0-9]{0,10}".prop_map(|s| s.to_string())
}

fn arb_ident() -> impl Strategy<Value = Ident> {
    arb_ident_name().prop_map(Ident::dummy)
}

fn arb_binop() -> impl Strategy<Value = BinOp> {
    prop_oneof![
        Just(BinOp::Add),
        Just(BinOp::Sub),
        Just(BinOp::Mul),
        Just(BinOp::Div),
        Just(BinOp::Mod),
        Just(BinOp::Eq),
        Just(BinOp::Ne),
        Just(BinOp::Lt),
        Just(BinOp::Le),
        Just(BinOp::Gt),
        Just(BinOp::Ge),
        Just(BinOp::And),
        Just(BinOp::Or),
        Just(BinOp::BitAnd),
        Just(BinOp::BitOr),
        Just(BinOp::BitXor),
        Just(BinOp::Shl),
        Just(BinOp::Shr),
    ]
}

fn arb_unaryop() -> impl Strategy<Value = UnaryOp> {
    prop_oneof![
        Just(UnaryOp::Neg),
        Just(UnaryOp::Not),
        Just(UnaryOp::Deref),
        Just(UnaryOp::Ref),
        Just(UnaryOp::Recv),
    ]
}

fn arb_literal() -> impl Strategy<Value = Literal> {
    prop_oneof![
        any::<i64>().prop_map(|n| Literal::Integer(n, IntFormat::Decimal)),
        "[a-zA-Z0-9 ]{0,20}".prop_map(|s| Literal::String(s.to_string())),
        any::<bool>().prop_map(Literal::Bool),
    ]
}

fn arb_type_annotation() -> impl Strategy<Value = TypeAnnotation> {
    // Simple types only - no recursion to avoid explosion
    prop_oneof![
        Just("int"),
        Just("int64"),
        Just("float64"),
        Just("string"),
        Just("bool"),
        Just("byte"),
        Just("rune"),
    ]
    .prop_map(|name| TypeAnnotation {
        name: name.to_string(),
        args: vec![],
        span: Span::dummy(),
        nullable: false,
    })
}

/// Generate a simple expression (no recursion)
fn arb_simple_expr() -> impl Strategy<Value = Expr> {
    prop_oneof![
        any::<i64>()
            .prop_map(|n| Expr::new(ExprKind::Integer(n, IntFormat::Decimal), Span::dummy())),
        any::<f64>()
            .prop_filter("finite floats only", |f| f.is_finite())
            .prop_map(|n| Expr::new(ExprKind::Float(n), Span::dummy())),
        "[a-zA-Z0-9 ]{0,20}"
            .prop_map(|s| Expr::new(ExprKind::String(s.to_string()), Span::dummy())),
        any::<bool>().prop_map(|b| Expr::new(ExprKind::Bool(b), Span::dummy())),
        Just(Expr::new(ExprKind::Nil, Span::dummy())),
        arb_ident_name().prop_map(|s| Expr::new(ExprKind::Ident(s), Span::dummy())),
    ]
}

/// Generate expressions with limited depth
fn arb_expr() -> impl Strategy<Value = Expr> {
    arb_simple_expr().prop_recursive(
        3,  // depth
        64, // max nodes
        10, // items per collection
        |inner| {
            prop_oneof![
                // Binary expression
                (arb_binop(), inner.clone(), inner.clone()).prop_map(|(op, left, right)| {
                    Expr::new(
                        ExprKind::Binary {
                            op,
                            left: Box::new(left),
                            right: Box::new(right),
                        },
                        Span::dummy(),
                    )
                }),
                // Unary expression
                (arb_unaryop(), inner.clone()).prop_map(|(op, operand)| {
                    Expr::new(
                        ExprKind::Unary {
                            op,
                            operand: Box::new(operand),
                        },
                        Span::dummy(),
                    )
                }),
                // Field access
                (inner.clone(), arb_ident_name()).prop_map(|(expr, field)| {
                    Expr::new(
                        ExprKind::Field {
                            expr: Box::new(expr),
                            field,
                            span: Span::dummy(),
                        },
                        Span::dummy(),
                    )
                }),
                // Index expression
                (inner.clone(), inner.clone()).prop_map(|(expr, index)| {
                    Expr::new(
                        ExprKind::Index {
                            expr: Box::new(expr),
                            index: Box::new(index),
                        },
                        Span::dummy(),
                    )
                }),
            ]
        },
    )
}

fn arb_pattern() -> impl Strategy<Value = Pattern> {
    prop_oneof![
        // Default pattern
        Just(Pattern {
            kind: PatternKind::Default,
            span: Span::dummy(),
        }),
        // Variant pattern
        arb_type_name().prop_map(|name| Pattern {
            kind: PatternKind::Variant {
                name,
                type_args: Vec::new(),
            },
            span: Span::dummy(),
        }),
        // Literal pattern
        arb_literal().prop_map(|lit| Pattern {
            kind: PatternKind::Literal(lit),
            span: Span::dummy(),
        }),
        // Destructor pattern
        (arb_type_name(), arb_ident()).prop_map(|(name, binding)| Pattern {
            kind: PatternKind::Destructor {
                name,
                type_args: Vec::new(),
                binding,
            },
            span: Span::dummy(),
        }),
    ]
}

fn arb_field() -> impl Strategy<Value = Field> {
    (arb_ident(), arb_type_annotation()).prop_map(|(ident, ty)| Field {
        ident,
        ty,
        tag: None,
        attributes: vec![],
    })
}

fn arb_param() -> impl Strategy<Value = Param> {
    (arb_ident(), arb_type_annotation()).prop_map(|(ident, ty)| Param { ident, ty })
}

fn arb_generic() -> impl Strategy<Value = Generic> {
    (arb_ident(), prop_oneof![Just("any"), Just("comparable")]).prop_map(|(ident, constraint)| {
        Generic {
            ident,
            constraint: constraint.to_string(),
        }
    })
}

fn arb_enum_variant() -> impl Strategy<Value = EnumVariant> {
    prop_oneof![
        // Unit variant
        arb_ident().prop_map(|ident| EnumVariant::Unit {
            ident,
            attributes: vec![],
        }),
        // Single value variant
        (arb_ident(), arb_type_annotation()).prop_map(|(ident, ty)| EnumVariant::Single {
            ident,
            ty,
            attributes: vec![],
        }),
        // Struct variant
        (arb_ident(), prop::collection::vec(arb_field(), 1..4)).prop_map(|(ident, fields)| {
            EnumVariant::Struct {
                ident,
                fields,
                attributes: vec![],
            }
        }),
    ]
}

fn arb_block() -> impl Strategy<Value = Block> {
    prop::collection::vec(arb_simple_stmt(), 0..5).prop_map(|stmts| Block {
        stmts,
        span: Span::dummy(),
    })
}

/// Generate simple statements (no deep nesting)
fn arb_simple_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        // Declaration
        (arb_ident(), arb_simple_expr()).prop_map(|(ident, value)| Stmt {
            kind: StmtKind::Decl { ident, value },
            span: Span::dummy()
        }),
        // Return
        prop::collection::vec(arb_simple_expr(), 0..3).prop_map(|values| Stmt {
            kind: StmtKind::Return { values },
            span: Span::dummy()
        }),
        // Expression statement
        arb_simple_expr().prop_map(|expr| Stmt {
            kind: StmtKind::Expr(expr),
            span: Span::dummy()
        }),
        // Break
        Just(Stmt {
            kind: StmtKind::Break,
            span: Span::dummy()
        }),
        // Continue
        Just(Stmt {
            kind: StmtKind::Continue,
            span: Span::dummy()
        }),
    ]
}

fn arb_type_kind() -> impl Strategy<Value = TypeKind> {
    prop_oneof![
        // Alias
        arb_type_annotation().prop_map(|target| TypeKind::Alias { target }),
        // Enum with variants
        prop::collection::vec(arb_enum_variant(), 1..5)
            .prop_map(|variants| TypeKind::Enum { variants }),
        // Struct
        prop::collection::vec(arb_field(), 0..5).prop_map(|fields| TypeKind::Struct { fields }),
    ]
}

fn arb_type_decl() -> impl Strategy<Value = TypeDecl> {
    (
        arb_ident(),
        prop::collection::vec(arb_generic(), 0..2),
        arb_type_kind(),
    )
        .prop_map(|(ident, generics, kind)| TypeDecl {
            ident,
            generics,
            kind,
            span: Span::dummy(),
            doc_comment: None,
            attributes: vec![],
        })
}

/// Generate an unnamed return param (empty ident name)
fn arb_return_param() -> impl Strategy<Value = Param> {
    arb_type_annotation().prop_map(|ty| Param {
        ident: Ident::new("", ty.span),
        ty,
    })
}

fn arb_func_decl() -> impl Strategy<Value = FuncDecl> {
    (
        arb_ident(),
        prop::collection::vec(arb_param(), 0..4),
        prop::collection::vec(arb_return_param(), 0..2),
        arb_block(),
    )
        .prop_map(|(ident, params, returns, body)| FuncDecl {
            receiver: None,
            ident,
            generics: vec![],
            params,
            returns,
            body,
            span: Span::dummy(),
            doc_comment: None,
            attributes: vec![],
        })
}

fn arb_decl() -> impl Strategy<Value = Decl> {
    prop_oneof![
        arb_type_decl().prop_map(Decl::Type),
        arb_func_decl().prop_map(Decl::Func),
    ]
}

/// Generate a complete file with package and declarations
fn arb_file() -> impl Strategy<Value = File> {
    (arb_ident_name(), prop::collection::vec(arb_decl(), 1..5)).prop_map(|(name, decls)| File {
        package: Ident {
            name,
            span: Span::dummy(),
        },
        imports: vec![],
        decls,
        comments: vec![],
    })
}

proptest! {
    /// Binary operators should be consistent - generating an expression shouldn't panic
    #[test]
    fn expr_generation_doesnt_panic(expr in arb_expr()) {
        // If we get here, generation succeeded
        let _ = format!("{:?}", expr);
    }

    /// Pattern generation shouldn't panic
    #[test]
    fn pattern_generation_doesnt_panic(pattern in arb_pattern()) {
        let _ = format!("{:?}", pattern);
    }

    /// Type declarations should be well-formed
    #[test]
    fn type_decl_generation_doesnt_panic(decl in arb_type_decl()) {
        let _ = format!("{:?}", decl);
    }

    /// Function declarations should be well-formed
    #[test]
    fn func_decl_generation_doesnt_panic(func in arb_func_decl()) {
        let _ = format!("{:?}", func);
    }

    /// Complete files should be well-formed
    #[test]
    fn file_generation_doesnt_panic(file in arb_file()) {
        let _ = format!("{:?}", file);
    }
}

mod compiler_properties {
    use std::io::Write;
    use std::process::Command;

    use soppo::build::compile;
    use soppo::types::Infer;

    use super::*;

    /// If type inference completes without error on an expression,
    /// it should produce a consistent type when run again
    #[test]
    fn type_inference_is_deterministic() {
        let expr = Expr::new(ExprKind::Integer(42, IntFormat::Decimal), Span::dummy());

        let mut infer1 = Infer::new().unwrap();
        let mut infer2 = Infer::new().unwrap();

        let ty1 = infer1.infer_expr_inner(&expr);
        let ty2 = infer2.infer_expr_inner(&expr);

        assert_eq!(format!("{:?}", ty1), format!("{:?}", ty2));
    }

    proptest! {
        /// Compiling valid-looking source shouldn't panic (errors are fine, panics are not)
        #[test]
        fn compile_doesnt_panic(
            pkg in arb_ident_name(),
            func_name in arb_ident_name(),
        ) {
            let source = format!(
                "package {}\n\nfunc {}() {{\n}}\n",
                pkg, func_name
            );
            // Errors are expected - we just want to make sure it doesn't panic
            let _ = compile(&source, "test.sop");
        }

        /// If Soppo compilation succeeds, the generated Go code should also compile
        #[test]
        fn successful_compile_produces_valid_go(
            pkg_name in arb_ident_name(),
            func_name in arb_ident_name(),
            ret_val in any::<i64>(),
        ) {
            // Generate simple but valid Soppo source (not main package to avoid needing main func)
            let source = format!(
                "package {}\n\nfunc {}() int {{\n    return {}\n}}\n",
                pkg_name, func_name, ret_val
            );

            // Try to compile with Soppo
            if let Ok(go_code) = compile(&source, "test.sop") {
                // Write to temp file and run go tool compile (type checks without linking)
                let dir = tempfile::tempdir().unwrap();
                let go_file = dir.path().join("test.go");
                let mut f = std::fs::File::create(&go_file).unwrap();
                write!(f, "{}", go_code).unwrap();

                let output = Command::new("go")
                    .arg("tool")
                    .arg("compile")
                    .arg("-p")
                    .arg(&pkg_name)
                    .arg(&go_file)
                    .current_dir(dir.path())
                    .output()
                    .expect("failed to run go tool compile");

                prop_assert!(
                    output.status.success(),
                    "go compile failed on Soppo output:\n--- Source ---\n{}\n--- Generated Go ---\n{}\n--- Error ---\n{}",
                    source,
                    go_code,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}
