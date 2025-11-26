use crate::source::Span;

/// A complete source file
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub package: String,
    pub imports: Vec<Import>,
    pub decls: Vec<Decl>,
}

/// Import declaration
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: String,
    pub span: Span,
}

/// Top-level declaration
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Const(ConstDecl),
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
    Alias { target: Type },
    Enum { variants: Vec<EnumVariant> },
    Struct { fields: Vec<Field> },
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
    pub span: Span,
}

/// Type annotation (before type checking)
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: String,
    pub args: Vec<Type>,
    pub span: Span,
}

/// Function declaration
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub receiver: Option<Param>,
    pub name: String,
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
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
    // var x = value, var x type, or var x type = value
    VarDecl {
        name: String,
        ty: Option<Type>, //infer from value if not provided
        value: Option<Expr>,
    },
    // const x = value or const x type = value (inside functions)
    ConstDecl {
        name: String,
        ty: Option<Type>, // infer from value if not provided
        value: Expr,
    },
    // x = value or x.y = value (assignment)
    Assign {
        target: Expr,
        value: Expr,
    },
    For {
        condition: Expr,
        body: Block,
    },
    If {
        condition: Expr,
        then_block: Block,
        else_block: Option<Block>,
    },
    Return {
        value: Option<Expr>,
    },
    Match {
        scrutinee: Expr,
        arms: Vec<Arm>,
    },
    Expr(Expr),
}

/// Expression
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Ident(String),
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        func: Box<Expr>,
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    Field {
        expr: Box<Expr>,
        field: String,
    },
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    ArrayLit {
        ty: Option<Type>, // For [5]int{...} syntax
        elements: Vec<Expr>,
    },
    StructLit {
        ty: Type,                    // The struct type name
        fields: Vec<(String, Expr)>, // field_name: value pairs
    },
    Block(Block),
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
}

/// Match arm
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Block,
    pub span: Span,
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
    /// Unit variant with no data: Color.Red
    Variant(String),
    /// Literal value: 42, "hello", true
    Literal(Literal),
    /// Variant with data extraction: Result.Ok(value)
    Destructor { name: String, binding: String },
    /// Struct variant destructuring: Shape.Circle{radius: r, ...}
    StructDestructor {
        name: String,                  // e.g., "Shape.Circle"
        fields: Vec<(String, String)>, // (field_name, binding_name) pairs
        rest: bool,                    // true if `...` was used to ignore remaining fields
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    String(String),
    Bool(bool),
}
