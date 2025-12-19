use std::cell::Cell;
use std::fmt;

use super::lexer::Comment;
use super::source::Span;

/// A function call argument: (optional name with span, expression, is_spread)
/// - name: Some((name, span)) for named arguments like `foo: expr`, None for positional
/// - expr: The argument expression
/// - spread: true if this argument uses spread syntax `expr...`
pub type CallArg = (Option<(String, Span)>, Expr, bool);

/// An identifier with its source location
#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }

    /// Create a dummy ident for testing or generated code
    pub fn dummy(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            span: Span::dummy(),
        }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl AsRef<str> for Ident {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

impl PartialEq<str> for Ident {
    fn eq(&self, other: &str) -> bool {
        self.name == other
    }
}

impl PartialEq<&str> for Ident {
    fn eq(&self, other: &&str) -> bool {
        self.name == *other
    }
}

impl PartialEq<String> for Ident {
    fn eq(&self, other: &String) -> bool {
        self.name == *other
    }
}

/// A complete source file
#[derive(Debug, Clone)]
pub struct File {
    pub package: Ident,
    pub imports: Vec<Import>,
    pub decls: Vec<Decl>,
    pub comments: Vec<Comment>,
}

/// Import declaration
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub alias: Option<String>, // Optional alias: myPkg "path/to/pkg"
    pub path: String,
    pub span: Span,
}

/// Top-level declaration
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Const(ConstDecl),
    ConstBlock(Vec<ConstDecl>), // Grouped const block for iota support
    Var(VarDecl),
    Type(TypeDecl),
    Func(FuncDecl),
}

impl Decl {
    /// Get the span of this declaration
    pub fn span(&self) -> &Span {
        match self {
            Decl::Const(c) => &c.span,
            Decl::ConstBlock(cs) => &cs.first().expect("const block should not be empty").span,
            Decl::Var(v) => &v.span,
            Decl::Type(t) => &t.span,
            Decl::Func(f) => &f.span,
        }
    }
}

/// Constant declaration
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub ident: Ident,
    pub ty: Option<TypeAnnotation>, // Optional - infer from value if not provided
    pub value: Expr,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Package-level variable declaration
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub ident: Ident,
    pub ty: Option<TypeAnnotation>, // Optional - infer from value if not provided
    pub value: Option<Expr>,        // Optional - zero value if not provided
    pub span: Span,
}

/// Type declaration (enum, struct, alias)
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub ident: Ident,
    pub generics: Vec<Generic>,
    pub kind: TypeKind,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Kind of type declaration
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// Type alias: type X = Y (X is exactly Y)
    Alias {
        target: TypeAnnotation,
    },
    /// Type definition: type X Y (X is a new distinct type based on Y)
    Definition {
        target: TypeAnnotation,
    },
    Enum {
        variants: Vec<EnumVariant>,
    },
    Struct {
        fields: Vec<Field>,
    },
    Interface {
        methods: Vec<InterfaceMethod>,
    },
}

/// Interface method signature
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethod {
    pub ident: Ident,
    pub params: Vec<Param>,
    pub returns: Vec<TypeAnnotation>,
}

/// Generic type parameter
#[derive(Debug, Clone, PartialEq)]
pub struct Generic {
    pub ident: Ident,
    pub constraint: String, // e.g., "any", "comparable"
}

/// Enum variant
#[derive(Debug, Clone, PartialEq)]
pub enum EnumVariant {
    /// Unit variant: Red
    Unit { ident: Ident },
    /// Single value variant: Text string
    Single { ident: Ident, ty: TypeAnnotation },
    /// Struct variant: Circle struct { Radius float64 }
    Struct { ident: Ident, fields: Vec<Field> },
}

/// Struct field
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub ident: Ident,
    pub ty: TypeAnnotation,
    pub tag: Option<String>,
}

/// Type annotation (before type checking)
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAnnotation {
    pub name: String,
    pub args: Vec<TypeAnnotation>,
    pub span: Span,
    pub nullable: bool, // true for ?*T, ?[]T, ?Interface
}

/// Function declaration
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub receiver: Option<Param>,
    pub ident: Ident,
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    pub returns: Vec<Param>, // Empty = no return; Param.ident.name is "" for unnamed returns
    pub body: Block,
    pub span: Span,
    pub doc_comment: Option<String>,
}

/// Function parameter
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub ident: Ident,
    pub ty: TypeAnnotation,
}

/// Block of statements
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// Statement
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    // x := value (short declaration with inference)
    Decl {
        ident: Ident,
        value: Expr,
    },
    // x, y := value or x, y := expr1, expr2 (multi-value short declaration)
    MultiDecl {
        ident: Vec<Ident>,
        values: Vec<Expr>, // 1 value (multi-return) or N values (one per name)
    },
    // var x = value, var x type, or var x type = value
    VarDecl {
        ident: Ident,
        ty: Option<TypeAnnotation>, //infer from value if not provided
        value: Option<Expr>,
    },
    // var a, b, c type or var a, b = expr1, expr2
    MultiVarDecl {
        ident: Vec<Ident>,
        ty: Option<TypeAnnotation>, // shared type for all vars (var a, b, c int)
        values: Vec<Expr>,          // empty = zero value, or one value per name
    },
    // const x = value or const x type = value (inside functions)
    ConstDecl {
        ident: Ident,
        ty: Option<TypeAnnotation>, // infer from value if not provided
        value: Expr,
    },
    // const a, b = expr1, expr2 or const a, b type = expr1, expr2
    MultiConstDecl {
        idents: Vec<Ident>,
        ty: Option<TypeAnnotation>, // shared type for all consts
        values: Vec<Expr>,          // one value per name (consts must have values)
    },
    // x = value or x.y = value (assignment)
    Assign {
        target: Expr,
        value: Expr,
    },
    // x, y = value or x, y = expr1, expr2 (multi-value assignment)
    MultiAssign {
        targets: Vec<Expr>,
        values: Vec<Expr>, // 1 value (multi-return) or N values (one per target)
    },
    // x += value, x -= value, etc. (compound assignment)
    CompoundAssign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    // x++ or x-- (increment/decrement)
    IncDec {
        target: Expr,
        is_inc: bool, // true for ++, false for --
    },
    For {
        condition: Expr,
        body: Block,
    },
    // C-style for loop: for init; condition; post { body }
    ForCStyle {
        init: Option<Box<Stmt>>, // init statement (e.g., i := 0)
        condition: Option<Expr>, // condition (e.g., i < 10), None = infinite loop
        post: Option<Box<Stmt>>, // post statement (e.g., i++)
        body: Block,
    },
    // for key := range collection or for key, value := range collection
    ForRange {
        key: Ident,           // First variable (index/key)
        value: Option<Ident>, // Second variable (value) - None for single-var form
        collection: Expr,     // The collection being ranged over
        body: Block,
    },
    If {
        init: Option<Box<Stmt>>, // Optional init statement: if x := expr; condition { }
        condition: Expr,
        then_block: Block,
        else_block: Option<Block>,
    },
    Return {
        values: Vec<Expr>, // Empty = no return, one = single, many = multi-value
    },
    Match {
        scrutinee: Option<Expr>, // None for expression-less match
        arms: Vec<Arm>,
    },
    // ch <- value (channel send)
    Send {
        channel: Expr,
        value: Expr,
    },
    // select { case ... }
    Select {
        cases: Vec<SelectCase>,
    },
    // go expr (goroutine)
    Go(Expr),
    // defer expr (deferred call)
    DeferStmt(Expr),
    // break
    Break,
    // continue
    Continue,
    Expr(Expr),
    // stmt ? or stmt ? { handler } or stmt ? errName { handler }
    TryStmt {
        stmt: Box<Stmt>,
        error_name: Option<String>,
        handler: Option<Block>,
        try_span: Span,
        // Number of non-error return values to discard (set by type checker)
        // For `f() ?` where f returns `(T, error)`, this is 1
        // For `f() ?` where f returns just `error`, this is 0
        discard_count: Cell<usize>,
    },
    // type Name struct { ... } or type Name = ExistingType (local type declaration)
    LocalTypeDecl(TypeDecl),
}

/// Expression
#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Inferred type arguments for generic enum variants (set during type inference)
    pub inferred_type_args: std::cell::RefCell<Option<Vec<String>>>,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self {
            kind,
            span,
            inferred_type_args: std::cell::RefCell::new(None),
        }
    }
}

impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.span == other.span
    }
}

/// Part of an interpolated string
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// Literal text segment
    Literal(String),
    /// Interpolated expression: {expr}
    Expr(Box<Expr>),
}

/// Format of an integer literal for preserving source representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntFormat {
    Decimal,
    Octal,  // 0o755
    Hex,    // 0xFF
    Binary, // 0b1010
}

/// Integer literal with its original format
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntLit {
    pub value: i64,
    pub format: IntFormat,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Integer(i64, IntFormat),
    Float(f64),
    String(String),
    /// Raw string literal (backtick string): `hello\nworld` - no escape processing
    RawString(String),
    /// Rune literal: 'a', '\n', etc. - stores the raw character content
    Rune(String),
    /// Interpolated string: "Hello, {name}!"
    StringInterpolation(Vec<StringPart>),
    Bool(bool),
    Nil,
    Ident(String),
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        func: Box<Expr>,
        type_args: Vec<TypeAnnotation>,
        args: Vec<CallArg>,
    },
    Field {
        expr: Box<Expr>,
        field: String,
        field_span: Span,
    },
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    /// Slice expression: arr[low:high] or arr[low:high:cap]
    Slice {
        expr: Box<Expr>,
        low: Option<Box<Expr>>,
        high: Option<Box<Expr>>,
        cap: Option<Box<Expr>>, // For 3-index slice arr[low:high:cap]
    },
    /// Type assertion: x.(Type)
    TypeAssert {
        expr: Box<Expr>,
        ty: TypeAnnotation,
        /// Set by type inference if narrowing proves this assertion always succeeds
        known_match: Cell<bool>,
    },
    /// Nil assertion: x.(!nil) - asserts pointer is non-nil
    NilAssert {
        expr: Box<Expr>,
    },
    ArrayLit {
        ty: Option<TypeAnnotation>, // For [5]int{...} syntax
        elements: Vec<Expr>,
    },
    StructLit {
        ty: Option<TypeAnnotation>, // The struct type name (None for implicit like `{Name: "x"}`)
        fields: Vec<(String, Expr)>, // field_name: value pairs
    },
    /// Anonymous struct literal: struct { X int; Y int }{X: 1, Y: 2}
    AnonStructLit {
        field_defs: Vec<Field>,      // The inline field definitions
        fields: Vec<(String, Expr)>, // field_name: value pairs
    },
    MapLit {
        ty: TypeAnnotation,         // The map type: map[K]V
        entries: Vec<(Expr, Expr)>, // key: value pairs
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    /// Anonymous function: func(params) returns { body }
    FuncLit {
        params: Vec<Param>,
        returns: Vec<Param>,
        body: Block,
    },
    Block(Block),
    /// Parenthesised expression - preserves explicit grouping from source
    Paren(Box<Expr>),
}

/// Unary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,   // -x
    Not,   // !x
    Deref, // *p
    Ref,   // &x
    Recv,  // <-ch (channel receive)
}

/// Binary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    // Bitwise operators
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// Compound assignment operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Add,    // +=
    Sub,    // -=
    Mul,    // *=
    Div,    // /=
    Mod,    // %=
    BitAnd, // &=
    BitOr,  // |=
    BitXor, // ^=
    Shl,    // <<=
    Shr,    // >>=
}

/// Match arm
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub patterns: Vec<Pattern>, // Multiple patterns: case a, b, c:
    pub body: Block,
    pub span: Span,
}

/// Select case for select statement
#[derive(Debug, Clone, PartialEq)]
pub struct SelectCase {
    pub kind: SelectCaseKind,
    pub body: Block,
    pub span: Span,
}

/// Kind of select case
#[derive(Debug, Clone, PartialEq)]
pub enum SelectCaseKind {
    /// <-ch (receive and discard)
    Recv { channel: Expr },
    /// v := <-ch (receive with declaration)
    RecvDecl { ident: Ident, channel: Expr },
    /// v, ok := <-ch (receive with ok check)
    RecvDeclOk {
        ident: Ident,
        ok_ident: Ident,
        channel: Expr,
    },
    /// ch <- value (send)
    Send { channel: Expr, value: Expr },
    /// default:
    Default,
}

/// Pattern
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
    /// Type args inferred from context (e.g., scrutinee type in match)
    pub inferred_type_args: std::cell::RefCell<Option<Vec<String>>>,
}

#[derive(Debug, Clone)]
pub enum PatternKind {
    /// Catch-all case: default
    Default,
    /// Unit variant with no data: Colour.Red or Go constant like tar.TypeDir
    /// The Cell<bool> indicates if this is a soppo enum (true) or Go constant (false)
    /// Parser defaults to true; type inference sets to false for Go constants
    Variant {
        name: String,
        type_args: Vec<TypeAnnotation>,
        is_soppo_enum: Cell<bool>,
    },
    /// Literal value: 42, "hello", true
    Literal(Literal),
    /// Variant with data extraction: Result.Ok(value)
    Destructor {
        name: String,
        type_args: Vec<TypeAnnotation>,
        binding: Ident,
    },
    /// Struct destructuring: Shape.Circle{radius: r, ...} or Point{x: 0, y}
    StructDestructor {
        name: String, // e.g., "Shape.Circle" or "Point"
        type_args: Vec<TypeAnnotation>,
        fields: Vec<(String, FieldPattern)>, // (field_name, pattern) pairs
        rest: bool,                          // true if `...` was used to ignore remaining fields
    },
    /// Guard expression for expression-less match: case x > 0:
    Guard(Box<Expr>),
}

impl PartialEq for PatternKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PatternKind::Default, PatternKind::Default) => true,
            (
                PatternKind::Variant {
                    name: n1,
                    type_args: t1,
                    ..
                },
                PatternKind::Variant {
                    name: n2,
                    type_args: t2,
                    ..
                },
            ) => n1 == n2 && t1 == t2,
            (PatternKind::Literal(a), PatternKind::Literal(b)) => a == b,
            (
                PatternKind::Destructor {
                    name: n1,
                    type_args: t1,
                    binding: b1,
                },
                PatternKind::Destructor {
                    name: n2,
                    type_args: t2,
                    binding: b2,
                },
            ) => n1 == n2 && t1 == t2 && b1 == b2,
            (
                PatternKind::StructDestructor {
                    name: n1,
                    type_args: t1,
                    fields: f1,
                    rest: r1,
                },
                PatternKind::StructDestructor {
                    name: n2,
                    type_args: t2,
                    fields: f2,
                    rest: r2,
                },
            ) => n1 == n2 && t1 == t2 && f1 == f2 && r1 == r2,
            (PatternKind::Guard(a), PatternKind::Guard(b)) => a == b,
            _ => false,
        }
    }
}

/// Field pattern in struct destructuring
#[derive(Debug, Clone, PartialEq)]
pub enum FieldPattern {
    /// Bind field value to a variable: `field: binding` or just `field`
    Bind(Ident),
    /// Match field against a literal: `field: 42`
    Literal(Literal),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64, IntFormat),
    String(String),
    Bool(bool),
    Nil,
}
