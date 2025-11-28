# Soppo Compiler Progress

## Completed

- Parser (recursive descent, Go-style newlines)
- Type inference (Hindley-Milner)
- Enums → Go interfaces + variant types
- Pattern matching → Go type switches
- Exhaustiveness checking
- Generics with constraints `[T any]`
- Structs, methods, receivers
- Go interop (import stdlib/external packages, type extraction via tree-sitter)
- Pointers (`*T`, `&x`, `new`)
- Slices, arrays, maps
- Channels, goroutines, defer
- Range loops, break/continue
- Anonymous functions
- Unary operators (`-x`, `!x`)
- Full numeric types (int8-64, uint8-64, float32/64, complex64/128)
- Multi-value returns
- Multiple patterns in match arms (`case 1, 2, 3:`)
- Expression-less match (`match { case x > 0: ... }`)
- Interface definitions
- `select` statement
- `close` builtin
- `soppo:enum` markers in generated code
- Comment preservation (single-line `//` and block `/* */`)
- Bitwise operators (`&`, `|`, `^`, `<<`, `>>`)
- Compound assignments (`+=`, `-=`, etc.)
- Increment/decrement (`++`, `--`)
- Slice expressions (`arr[1:3]`)
- Type assertions (`x.(Type)`)
- Blank identifier (`_ = foo()`)

## Not Yet Implemented

**Core:**
- `?` operator (error propagation)
- Nil safety (flow-sensitive tracking)
- Named arguments
- String interpolation

## Testing

```bash
cargo test              # All tests
cargo insta review      # Review snapshots
```
