use std::cell::Cell;

use super::lexer::Comment;
use super::source::Span;

/// A complete source file
#[derive(Debug, Clone)]
pub struct File {
    pub package: String,
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
    Type(TypeDecl),
    Func(FuncDecl),
}

/// Constant declaration
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<Type>, // Optional - infer from value if not provided
    pub value: Expr,
    pub span: Span,
}

/// Type declaration (enum, struct, alias)
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    pub generics: Vec<Generic>,
    pub kind: TypeKind,
    pub span: Span,
}

/// Kind of type declaration
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// Type alias: type X = Y (X is exactly Y)
    Alias {
        target: Type,
    },
    /// Type definition: type X Y (X is a new distinct type based on Y)
    Definition {
        target: Type,
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
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Vec<Type>,
    pub span: Span,
}

/// Generic type parameter
#[derive(Debug, Clone, PartialEq)]
pub struct Generic {
    pub name: String,
    pub constraint: String, // e.g., "any", "comparable"
    pub span: Span,
}

/// Enum variant
#[derive(Debug, Clone, PartialEq)]
pub enum EnumVariant {
    /// Unit variant: Red
    Unit { name: String, span: Span },
    /// Single value variant: Text string
    Single { name: String, ty: Type, span: Span },
    /// Struct variant: Circle struct { Radius float64 }
    Struct {
        name: String,
        fields: Vec<Field>,
        span: Span,
    },
}

/// Struct field
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub tag: Option<String>,
    pub span: Span,
}

/// Type annotation (before type checking)
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: String,
    pub args: Vec<Type>,
    pub span: Span,
    pub nullable: bool, // true for ?*T, ?[]T, ?Interface
}

/// Function declaration
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub receiver: Option<Param>,
    pub name: String,
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    pub return_types: Vec<Type>, // Empty = no return, one = single, many = multi-value
    pub body: Block,
    pub span: Span,
}

/// Function parameter
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
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
        name: String,
        value: Expr,
    },
    // x, y := value or x, y := expr1, expr2 (multi-value short declaration)
    MultiDecl {
        names: Vec<String>,
        values: Vec<Expr>, // 1 value (multi-return) or N values (one per name)
    },
    // var x = value, var x type, or var x type = value
    VarDecl {
        name: String,
        ty: Option<Type>, //infer from value if not provided
        value: Option<Expr>,
    },
    // var a, b, c type or var a, b = expr1, expr2
    MultiVarDecl {
        names: Vec<String>,
        ty: Option<Type>,  // shared type for all vars (var a, b, c int)
        values: Vec<Expr>, // empty = zero value, or one value per name
    },
    // const x = value or const x type = value (inside functions)
    ConstDecl {
        name: String,
        ty: Option<Type>, // infer from value if not provided
        value: Expr,
    },
    // const a, b = expr1, expr2 or const a, b type = expr1, expr2
    MultiConstDecl {
        names: Vec<String>,
        ty: Option<Type>,  // shared type for all consts
        values: Vec<Expr>, // one value per name (consts must have values)
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
    // for key := range collection or for key, value := range collection
    ForRange {
        key: String,           // First variable (index/key)
        value: Option<String>, // Second variable (value) - None for single-var form
        collection: Expr,      // The collection being ranged over
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
        /// Number of non-error return values to discard (set by type checker)
        /// For `f() ?` where f returns `(T, error)`, this is 1
        /// For `f() ?` where f returns just `error`, this is 0
        discard_count: Cell<usize>,
    },
    // type Name struct { ... } or type Name = ExistingType (local type declaration)
    LocalTypeDecl(TypeDecl),
}

/// Expression
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// Part of an interpolated string
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// Literal text segment
    Literal(String),
    /// Interpolated expression: {expr}
    Expr(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Integer(i64),
    Float(f64),
    String(String),
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
        type_args: Vec<Type>,
        args: Vec<(Option<(String, Span)>, Expr)>, // (name with span, value) - name is Some for named args
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
    // Slice expression: arr[low:high] or arr[low:high:cap]
    Slice {
        expr: Box<Expr>,
        low: Option<Box<Expr>>,
        high: Option<Box<Expr>>,
        cap: Option<Box<Expr>>, // For 3-index slice arr[low:high:cap]
    },
    // Type assertion: x.(Type)
    TypeAssert {
        expr: Box<Expr>,
        ty: Type,
    },
    // Nil assertion: x.(!nil) - asserts pointer is non-nil
    NilAssert {
        expr: Box<Expr>,
    },
    ArrayLit {
        ty: Option<Type>, // For [5]int{...} syntax
        elements: Vec<Expr>,
    },
    StructLit {
        ty: Type,                    // The struct type name
        fields: Vec<(String, Expr)>, // field_name: value pairs
    },
    // Anonymous struct literal: struct { X int; Y int }{X: 1, Y: 2}
    AnonStructLit {
        field_defs: Vec<Field>,      // The inline field definitions
        fields: Vec<(String, Expr)>, // field_name: value pairs
    },
    MapLit {
        ty: Type,                   // The map type: map[K]V
        entries: Vec<(Expr, Expr)>, // key: value pairs
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    // Anonymous function: func(params) returnTypes { body }
    FuncLit {
        params: Vec<Param>,
        return_types: Vec<Type>,
        body: Block,
    },
    Block(Block),
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
    RecvDecl { name: String, channel: Expr },
    /// v, ok := <-ch (receive with ok check)
    RecvDeclOk {
        name: String,
        ok_name: String,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// Catch-all case: default
    Default,
    /// Unit variant with no data: Colour.Red
    Variant(String),
    /// Literal value: 42, "hello", true
    Literal(Literal),
    /// Variant with data extraction: Result.Ok(value)
    Destructor { name: String, binding: String },
    /// Struct destructuring: Shape.Circle{radius: r, ...} or Point{x: 0, y}
    StructDestructor {
        name: String,                        // e.g., "Shape.Circle" or "Point"
        fields: Vec<(String, FieldPattern)>, // (field_name, pattern) pairs
        rest: bool,                          // true if `...` was used to ignore remaining fields
    },
    /// Guard expression for expression-less match: case x > 0:
    Guard(Box<Expr>),
}

/// Field pattern in struct destructuring
#[derive(Debug, Clone, PartialEq)]
pub enum FieldPattern {
    /// Bind field value to a variable: `field: binding` or just `field`
    Bind(String),
    /// Match field against a literal: `field: 42`
    Literal(Literal),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    String(String),
    Bool(bool),
}
