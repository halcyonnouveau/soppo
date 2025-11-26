# Soppo Compiler Progress

## Architecture

```
Source Code → Lexer → Parser → AST → Type Inference → Codegen → Go Code
```

- **Lexer** (`lexer.rs`): Uses `logos`, tracks spans with byte offsets
- **Parser** (`parser.rs`): Recursive descent, newline-aware (Go-style)
- **AST** (`ast.rs`): `Expr/Stmt/Pattern { kind, span }` pattern
- **Type System** (`ty.rs`): `Type::Con/Fun/Var` ADT, Hindley-Milner inference
- **Codegen** (`codegen.rs`): Pure AST traversal, no regex

## Current Status

### Completed
- ✅ Lexer with full span tracking
- ✅ Parser for all expressions, statements, declarations
- ✅ Newline-based parsing (Go-style, no semicolons)
- ✅ Type inference (Hindley-Milner with unification)
- ✅ Enum codegen (interface + variant types + constructors)
- ✅ Match statements → Go type switches
- ✅ Generic type constraints (`[T any, E any]`)
- ✅ Keyword validation (Go + Soppo reserved words)
- ✅ Pattern matching with qualified names (`Result.Ok(value)`)
- ✅ Snapshot testing with `insta`
- ✅ Struct variant destructuring in match (`case Shape.Circle{radius: r, ...}:`)
- ✅ Exhaustiveness checking for match expressions
- ✅ Namespaced enum variant types (`Color_Red`, `Shape_Circle`) to avoid collisions
- ✅ Generic type inference (instantiation, type argument syntax `func[T](args)`)

### Not Yet Implemented
- Standard library (Result, Option as prelude)
- `.d.sop` definition files for Go interop
- `?` operator for error propagation

### Future `...` Support
Currently `...` is only used in struct destructuring patterns to ignore remaining fields. Go uses `...` in several other contexts that we should eventually support:
- **Variadic function parameters**: `func sum(nums ...int) int`
- **Slice spreading in function calls**: `sum(slice...)`
- **Array length inference**: `arr := [...]int{1, 2, 3}`

## Key Design Decisions

1. **Newline tracking**: Parser treats newlines as separators (like Go's automatic semicolon insertion)
2. **Enum syntax**: `type Result enum { Ok T, Err E }` with `Result.Ok(value)` in patterns
3. **Generic constraints required**: `[T any]` not `[T]` (matches Go 1.18+)
4. **Type checking mandatory**: No codegen without successful type check
5. **Snapshot tests for codegen**: Unit tests for internal logic

## File Structure

```
src/
  lib.rs          - Public API
  main.rs         - CLI entry point
  lexer.rs        - Token definitions + lexer
  parser.rs       - Recursive descent parser
  ast.rs          - AST node definitions
  ty.rs           - Type representation
  infer.rs        - Type inference engine
  codegen.rs      - Go code generation
  module.rs       - GlobalState, module tracking
  source.rs       - Span, FileId, Symbol
  error.rs        - Error types with miette
tests/
  e2e.rs          - End-to-end tests with snapshots
  snapshots/      - Insta snapshot files
```

## Testing

```bash
cargo test --lib      # Unit tests
cargo test --test e2e # E2E with snapshots
cargo insta review    # Review snapshot changes
```

## Example

Soppo:
```
type Result[T any, E any] enum {
    Ok T
    Err E
}

func unwrapOr(r Result, defaultVal int) int {
    var result int
    match r {
    case Result.Ok(value):
        result = value
    case Result.Err(msg):
        result = defaultVal
    }
    return result
}
```

Generated Go:
```go
type Result[T any, E any] interface {
    isResult()
}

type Ok[T any, E any] struct {
    Value T
}
func (Ok[T, E]) isResult() {}

type Err[T any, E any] struct {
    Value E
}
func (Err[T, E]) isResult() {}

func ResultOk[T any, E any](value T) Result[T, E] {
    return Ok[T, E]{Value: value}
}

func ResultErr[T any, E any](value E) Result[T, E] {
    return Err[T, E]{Value: value}
}
```
