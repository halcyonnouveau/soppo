//! Typed AST - AST nodes with inferred types attached.
//!
//! This module contains typed versions of the syntax AST nodes.
//! Each expression carries its inferred `Type`, enabling codegen
//! to access type information directly without GlobalCtxt lookups.

use super::ty::Type;
use crate::syntax::{AssignOp, BinOp, Generic, Ident, IntFormat, Literal, Span, UnaryOp};

/// A typed expression with its inferred type
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Type,
    pub span: Span,
}

impl TypedExpr {
    pub fn new(kind: TypedExprKind, ty: Type, span: Span) -> Self {
        Self { kind, ty, span }
    }

    /// Create an error expression (for error recovery)
    pub fn error(span: Span) -> Self {
        Self {
            kind: TypedExprKind::Error,
            ty: Type::error(),
            span,
        }
    }

    /// Check if this is an error expression
    pub fn is_error(&self) -> bool {
        self.ty.is_error()
    }
}

/// Typed expression kinds - mirrors ExprKind but with types resolved
#[derive(Debug, Clone)]
pub enum TypedExprKind {
    // Literals
    Integer(i64, IntFormat),
    Float(f64),
    String(String),
    RawString(String),
    Rune(String),
    StringInterpolation(Vec<TypedStringPart>),
    Bool(bool),
    Nil,

    /// Variable reference
    Ident(String),

    Binary {
        op: BinOp,
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
    },

    /// Function call expression, e.g., `foo[int](1, 2, 3)`
    Call {
        /// The function being called
        func: Box<TypedExpr>,
        /// Type arguments in square brackets, e.g., `[int]` in `make[[]int](5)`
        type_args: Vec<Type>,
        /// Value arguments in parentheses, e.g., `(1, 2, 3)` in `foo(1, 2, 3)`
        args: Vec<TypedCallArg>,
    },

    /// Type conversion: `int(x)`, `[]byte(s)`, `MyType(value)`
    /// Distinct from Call - this converts a value to a target type.
    TypeConversion {
        /// The target type to convert to
        target_ty: Type,
        /// The value being converted
        value: Box<TypedExpr>,
    },

    /// Type instantiation: `Option[int]` for accessing generic type members
    /// Used when accessing variants like `Option[int].None`
    TypeInst {
        /// The instantiated type with resolved type arguments
        ty: Type,
    },

    /// Struct field access: `point.x`, `person.name`
    Field {
        expr: Box<TypedExpr>,
        field: String,
        span: Span,
    },

    /// Package member access: `fmt.Println`, `helpers.Point`
    PackageMember {
        /// The package alias/name
        pkg: String,
        /// The member being accessed (function, type, constant)
        member: String,
    },

    /// Enum variant access: `Option.Some`, `Colour.Red`
    EnumVariant {
        /// The full enum type (with type args if applicable)
        enum_ty: Type,
        /// The variant name
        variant: String,
    },

    Index {
        expr: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },

    Slice {
        expr: Box<TypedExpr>,
        low: Option<Box<TypedExpr>>,
        high: Option<Box<TypedExpr>>,
        cap: Option<Box<TypedExpr>>,
    },

    TypeAssert {
        expr: Box<TypedExpr>,
        /// The target type of the assertion
        target_ty: Type,
        /// True if type narrowing proves this always succeeds
        known_safe: bool,
    },

    NilAssert {
        expr: Box<TypedExpr>,
    },

    ArrayLit {
        /// Element type (resolved)
        elem_ty: Type,
        elements: Vec<TypedExpr>,
    },

    StructLit {
        /// Full resolved struct type including generics
        struct_ty: Type,
        /// Field assignments: (field_name, value) - None for positional
        fields: Vec<(Option<String>, TypedExpr)>,
        /// True if type was implicit in source (e.g., inside slice literal)
        implicit: bool,
        /// Whether written on multiple lines in source
        multiline: bool,
    },

    AnonStructLit {
        /// The anonymous struct type
        struct_ty: Type,
        /// Field assignments
        fields: Vec<(Option<String>, TypedExpr)>,
    },

    MapLit {
        /// Full resolved map type
        map_ty: Type,
        entries: Vec<(TypedExpr, TypedExpr)>,
    },

    Unary {
        op: UnaryOp,
        operand: Box<TypedExpr>,
    },

    FuncLit {
        params: Vec<TypedParam>,
        returns: Vec<TypedParam>,
        body: TypedBlock,
    },

    Block(TypedBlock),
    Paren(Box<TypedExpr>),

    /// Error placeholder (for error recovery)
    Error,
}

/// Part of an interpolated string (typed)
#[derive(Debug, Clone)]
pub enum TypedStringPart {
    Literal(String),
    Expr {
        expr: Box<TypedExpr>,
        format: Option<String>,
    },
}

/// Function call argument (typed)
/// (optional name with span, expression, is_spread)
pub type TypedCallArg = (Option<(String, Span)>, TypedExpr, bool);

/// A typed statement
#[derive(Debug, Clone)]
pub struct TypedStmt {
    pub kind: TypedStmtKind,
    pub span: Span,
}

impl TypedStmt {
    pub fn new(kind: TypedStmtKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Returns true if this statement diverges (never returns normally).
    /// Used for determining if control flow continues past this statement.
    pub fn diverges(&self) -> bool {
        self.kind.diverges()
    }

    /// Create an error statement for when inference fails.
    pub fn error(span: Span) -> Self {
        Self {
            kind: TypedStmtKind::Expr(TypedExpr::error(span)),
            span,
        }
    }
}

/// Typed statement kinds
#[derive(Debug, Clone)]
pub enum TypedStmtKind {
    /// x := value (short declaration)
    Decl {
        ident: Ident,
        /// Resolved variable type
        var_ty: Type,
        value: TypedExpr,
    },

    /// x, y := values (multi-value short declaration)
    MultiDecl {
        idents: Vec<Ident>,
        /// Resolved types for each variable
        var_tys: Vec<Type>,
        values: Vec<TypedExpr>,
    },

    /// var x type = value
    VarDecl {
        ident: Ident,
        var_ty: Type,
        has_explicit_type: bool,
        value: Option<TypedExpr>,
    },

    /// var a, b, c type = values
    MultiVarDecl {
        idents: Vec<Ident>,
        var_ty: Type,
        has_explicit_type: bool,
        values: Vec<TypedExpr>,
    },

    /// const x = value
    ConstDecl {
        ident: Ident,
        const_ty: Type,
        has_explicit_type: bool,
        value: TypedExpr,
    },

    /// const a, b = values
    MultiConstDecl {
        idents: Vec<Ident>,
        const_ty: Type,
        has_explicit_type: bool,
        values: Vec<TypedExpr>,
    },

    /// x = value
    Assign {
        target: TypedExpr,
        value: TypedExpr,
    },

    /// x, y = values
    MultiAssign {
        targets: Vec<TypedExpr>,
        values: Vec<TypedExpr>,
    },

    /// x += value
    CompoundAssign {
        target: TypedExpr,
        op: AssignOp,
        value: TypedExpr,
    },

    /// x++ or x--
    IncDec {
        target: TypedExpr,
        is_inc: bool,
    },

    /// for condition { body }
    For {
        condition: TypedExpr,
        body: TypedBlock,
    },

    /// for init; condition; post { body }
    ForCStyle {
        init: Option<Box<TypedStmt>>,
        condition: Option<TypedExpr>,
        post: Option<Box<TypedStmt>>,
        body: TypedBlock,
    },

    /// for key, value := range collection { body }
    ForRange {
        key: Ident,
        key_ty: Type,
        value: Option<Ident>,
        value_ty: Option<Type>,
        collection: TypedExpr,
        body: TypedBlock,
    },

    /// if condition { then } else { else }
    If {
        init: Option<Box<TypedStmt>>,
        condition: TypedExpr,
        then_block: TypedBlock,
        else_block: Option<TypedBlock>,
    },

    /// return values
    Return {
        values: Vec<TypedExpr>,
    },

    /// match scrutinee { arms }
    Match {
        scrutinee: Option<TypedExpr>,
        scrutinee_ty: Option<Type>,
        arms: Vec<TypedArm>,
    },

    /// ch <- value
    Send {
        channel: TypedExpr,
        value: TypedExpr,
    },

    /// select { cases }
    Select {
        cases: Vec<TypedSelectCase>,
    },

    /// go expr
    Go(TypedExpr),

    /// defer expr
    DeferStmt(TypedExpr),

    Break,
    Continue,

    /// Expression statement
    Expr(TypedExpr),

    /// stmt ? or stmt ? { handler }
    TryStmt {
        stmt: Box<TypedStmt>,
        error_name: Option<String>,
        handler: Option<TypedBlock>,
        try_span: Span,
        /// Number of non-error values to discard (computed, not Cell)
        discard_count: usize,
        /// Types of discarded values
        discard_types: Vec<Type>,
    },

    /// Local type declaration
    LocalTypeDecl(TypedTypeDecl),
}

impl TypedStmtKind {
    /// Returns true if this statement diverges (never returns normally).
    pub fn diverges(&self) -> bool {
        match self {
            // These always diverge
            TypedStmtKind::Return { .. } => true,
            TypedStmtKind::Break => true,
            TypedStmtKind::Continue => true,

            // If/Match diverge only if all branches diverge
            TypedStmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                let then_diverges = then_block.stmts.last().is_some_and(|s| s.diverges());
                let else_diverges = else_block
                    .as_ref()
                    .is_some_and(|b| b.stmts.last().is_some_and(|s| s.diverges()));
                then_diverges && else_diverges
            }
            TypedStmtKind::Match { arms, .. } => {
                // Match diverges if all arms diverge (exhaustiveness checked separately)
                !arms.is_empty()
                    && arms
                        .iter()
                        .all(|arm| arm.body.stmts.last().is_some_and(|s| s.diverges()))
            }

            // Everything else doesn't diverge
            _ => false,
        }
    }
}

/// Typed block
#[derive(Debug, Clone)]
pub struct TypedBlock {
    pub stmts: Vec<TypedStmt>,
    pub span: Span,
}

impl TypedBlock {
    pub fn new(stmts: Vec<TypedStmt>, span: Span) -> Self {
        Self { stmts, span }
    }

    /// Returns true if this block diverges (last statement diverges).
    pub fn diverges(&self) -> bool {
        self.stmts.last().is_some_and(|s| s.diverges())
    }
}

/// Typed function parameter
#[derive(Debug, Clone)]
pub struct TypedParam {
    pub ident: Ident,
    pub ty: Type,
    /// Whether this parameter/return is nullable (for soppo:nilable comment)
    pub nullable: bool,
}

/// Typed match arm
#[derive(Debug, Clone)]
pub struct TypedArm {
    pub patterns: Vec<TypedPattern>,
    pub body: TypedBlock,
    pub span: Span,
}

/// Typed pattern
#[derive(Debug, Clone)]
pub struct TypedPattern {
    pub kind: TypedPatternKind,
    pub span: Span,
    /// The type this pattern matches against
    pub matched_ty: Type,
}

/// Typed pattern kinds
#[derive(Debug, Clone)]
pub enum TypedPatternKind {
    /// default case
    Default,

    /// Enum variant: Colour.Red or Go constant
    Variant {
        /// Full enum type
        enum_ty: Type,
        variant_name: String,
        /// Resolved type arguments
        type_args: Vec<Type>,
        /// True for Soppo enum, false for Go constant
        is_soppo_enum: bool,
    },

    /// Literal: 42, "hello", true
    Literal(Literal),

    /// Destructor: Result.Ok(value)
    Destructor {
        enum_ty: Type,
        variant_name: String,
        type_args: Vec<Type>,
        binding: Ident,
        /// Type of the bound value
        binding_ty: Type,
    },

    /// Struct destructuring: Shape.Circle{radius: r} or Point{x, y}
    StructDestructor {
        /// Full pattern name (e.g., "Shape.Circle" or "Point")
        pattern_name: String,
        /// Base enum/struct type
        struct_ty: Type,
        type_args: Vec<Type>,
        fields: Vec<(String, TypedFieldPattern)>,
        rest: bool,
    },

    /// Guard expression
    Guard(Box<TypedExpr>),
}

/// Typed field pattern
#[derive(Debug, Clone)]
pub enum TypedFieldPattern {
    /// Bind field to variable
    Bind(Ident, Type),
    /// Match field against literal
    Literal(Literal),
}

/// Typed select case
#[derive(Debug, Clone)]
pub struct TypedSelectCase {
    pub kind: TypedSelectCaseKind,
    pub body: TypedBlock,
    pub span: Span,
}

/// Typed select case kinds
#[derive(Debug, Clone)]
pub enum TypedSelectCaseKind {
    /// <-ch (receive and discard)
    Recv {
        channel: TypedExpr,
        /// Type of received value
        recv_ty: Type,
    },
    /// v := <-ch
    RecvDecl {
        ident: Ident,
        channel: TypedExpr,
        recv_ty: Type,
    },
    /// v, ok := <-ch
    RecvDeclOk {
        ident: Ident,
        ok_ident: Ident,
        channel: TypedExpr,
        recv_ty: Type,
    },
    /// ch <- value
    Send {
        channel: TypedExpr,
        value: TypedExpr,
    },
    /// default:
    Default,
}

/// Typed top-level declaration
#[derive(Debug, Clone)]
pub enum TypedDecl {
    Const(TypedConstDecl),
    ConstBlock(Vec<TypedConstDecl>),
    Var(TypedVarDecl),
    Type(TypedTypeDecl),
    Func(TypedFuncDecl),
}

/// Typed constant declaration
#[derive(Debug, Clone)]
pub struct TypedConstDecl {
    pub ident: Ident,
    pub const_ty: Type,
    /// True if type was explicitly annotated in source
    pub has_explicit_type: bool,
    pub value: TypedExpr,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Typed variable declaration
#[derive(Debug, Clone)]
pub struct TypedVarDecl {
    pub ident: Ident,
    pub var_ty: Type,
    pub has_explicit_type: bool,
    pub value: Option<TypedExpr>,
    pub span: Span,
}

/// Typed type declaration
#[derive(Debug, Clone)]
pub struct TypedTypeDecl {
    pub ident: Ident,
    pub generics: Vec<Generic>,
    pub kind: TypedTypeKind,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Typed type declaration kind
#[derive(Debug, Clone)]
pub enum TypedTypeKind {
    Alias {
        target: Type,
    },
    Definition {
        target: Type,
    },
    Enum {
        /// Resolved variant types
        variants: Vec<TypedEnumVariant>,
    },
    Struct {
        /// Resolved field types
        fields: Vec<(String, Type, Option<String>)>, // (name, type, tag)
    },
    Interface {
        methods: Vec<TypedInterfaceMethod>,
    },
}

/// Typed enum variant
#[derive(Debug, Clone)]
pub enum TypedEnumVariant {
    Unit {
        ident: Ident,
    },
    Single {
        ident: Ident,
        ty: Type,
    },
    Struct {
        ident: Ident,
        fields: Vec<(String, Type)>,
    },
}

/// Typed interface method
#[derive(Debug, Clone)]
pub struct TypedInterfaceMethod {
    pub ident: Ident,
    pub params: Vec<TypedParam>,
    pub returns: Vec<Type>,
}

/// Typed function declaration
#[derive(Debug, Clone)]
pub struct TypedFuncDecl {
    pub receiver: Option<TypedParam>,
    pub ident: Ident,
    pub generics: Vec<Generic>,
    pub params: Vec<TypedParam>,
    pub returns: Vec<TypedParam>,
    pub body: TypedBlock,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Import kind - Go or Soppo module
#[derive(Debug, Clone)]
pub enum TypedImportKind {
    Go,
    Soppo(crate::syntax::ModuleId),
}

/// Typed import
#[derive(Debug, Clone)]
pub struct TypedImport {
    pub alias: Option<String>,
    pub path: String,
    pub span: Span,
    pub kind: TypedImportKind,
}

/// A complete typed source file
#[derive(Debug, Clone)]
pub struct TypedFile {
    pub package: Ident,
    pub imports: Vec<TypedImport>,
    pub decls: Vec<TypedDecl>,
    /// Comments from the source file (for preservation in output)
    pub comments: Vec<crate::syntax::Comment>,
}
