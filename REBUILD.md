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

### Completed (continued)
- ✅ Variable declarations with type inference (`var a = 1`, `var b int = 2`, `var c int`)
- ✅ Constant declarations with type inference (`const X = 1`, `const Y int = 2`)
- ✅ Constants inside functions (not just top-level)

### Not Yet Implemented

**Priority 1 - Critical:**
- Import / package system
- Standard library (Result, Option as prelude)
- Slice type (`[]T`) + composite literals
- Map type (`map[K]V`) + composite literals
- Goroutines (`go func()`)
- Channel type (`chan T`, `<-chan T`, `chan<- T`)
- Channel operations (`ch <- v`, `<-ch`, `close(ch)`)

**Priority 2 - Important:**
- Interface types (define contracts, Go stdlib interop)
- Pointer type (`*T`, `&x`, `*p`)
- `break`, `continue`
- Range-based for loops (`for i, v := range collection`)
- `defer`
- Anonymous functions / closures
- `select` statement
- `make` / `new` built-ins

**Priority 3 - Nice to Have:**
- Unary operators (`-x` negation)
- Bitwise operators (`&`, `|`, `^`, `<<`, `>>`, `&^`)
- Compound assignments (`+=`, `-=`, `*=`, etc.)
- Increment/decrement (`++`, `--`)
- Slice expressions (`arr[1:3]`)
- Type assertions (`x.(Type)`)
- Blank identifier in assignments (`_ = foo()`)
- Full numeric types (`int8`-`64`, `uint8`-`64`, `float32`, `byte`, `rune`, `uintptr`, `complex64`/`128`)

**Soppo-specific (after Priority 1):**
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

## Variable Declaration Rules (Go-style)

Following Go's rules for `:=` vs `=`:

| Syntax | Meaning | Scope |
|--------|---------|-------|
| `x := 1` | Declare AND assign (short declaration) | Inside functions only |
| `x = 1` | Assign only (must be declared) | Anywhere after declaration |
| `var x = 1` | Declare with type inference | Anywhere |
| `var x int = 1` | Declare with explicit type | Anywhere |
| `var x int` | Declare without value (zero value) | Anywhere |
| `const X = 1` | Constant with inference | Top-level |
| `const X int = 1` | Constant with explicit type | Top-level |

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
  common/         - Shared test utilities
  pass.rs         - Tests that should compile successfully
  fail.rs         - Tests that should produce errors
  fixtures/
    pass/         - .sop files that should compile
    fail/         - .sop files that should error
  snapshots/      - Insta snapshot files
```

## Testing

```bash
cargo test --lib             # Unit tests
cargo test --test pass       # Passing compilation tests
cargo test --test fail       # Error case tests
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
