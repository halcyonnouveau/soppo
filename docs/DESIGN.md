# Soppo Language Design

This document describes the design of Soppo - the principles that guide development and the features it provides. It's intended for contributors and anyone wanting to understand why Soppo works the way it does.

## Principles

- Reuse Go syntax where it exists
- Compile to idiomatic Go
- Catch errors at compile time, not runtime
- Do the hard work to make correctness easy
- Every feature should complement Go's design
- Always provide useful error messages

## Architecture

```
Source (.sop) -> Parser -> AST -> Type Checking/Inference -> Codegen -> Output (.go)
                                            ^
                                   External .go packages
```

Go imports are resolved by parsing Go source files with tree-sitter to extract type signatures.

## Enums

Go lacks sum types, forcing developers to use interfaces or constants with no compile-time exhaustiveness checking. Soppo enums provide tagged unions that the compiler can verify.

```go
// Unit variants
type Colour enum {
	Red
	Green
	Blue
}

// Data variants
type Result enum {
	Ok int
	Err string
}

// Struct variants
type Shape enum {
    Circle struct { radius float64 }
    Rectangle struct {
    	width float64
    	height float64
    }
}

// Generic enums
type Option[T any] enum {
	Some T
	None
}
```

## Pattern Matching

Go's `switch` lacks destructuring and exhaustiveness checking. `match` replaces `switch` in Soppo, making it safe and ergonomic to work with enums and structured data.

```go
match colour {
case Colour.Red:
    action = "stop"
case Colour.Green:
    action = "go"
}

// Data extraction
match opt {
case Option.Some(value):
    result = value * 2
case Option.None:
    result = 0
}

// Struct destructuring
match shape {
case Shape.Circle{radius: r}:
    area = 3.14 * r * r
case Shape.Rectangle{width: w, ...}:
    area = width * 1
}

// Multiple patterns (bindings must match)
case "a", "b", "c":
case Colour.Red, Colour.Blue:
case Shape.Circle{radius: r}, Shape.Ellipse{radius: r, ...}:

// Expression-less match (like if/else chain)
match {
case x > 0:
    pos()
case x < 0:
    neg()
case x == 0 || y == 0:
    zero()
}
```

All variants must be handled (exhaustiveness checking).

## Error Handling (`?` operator)

Go's `if err != nil { return err }` is verbose and repetitive. The `?` operator reduces boilerplate while keeping error handling explicit and visible.

```go
// Propagate error directly
port := parsePort(config) ?

// Custom handling, no variable needed
port := parsePort(config) ? {
    return fmt.Errorf("parse failed")
}

// Custom handling with named error (avoids shadowing)
port := parsePort(config) ? parseErr {
    return fmt.Errorf("parse failed: %v", parseErr)
}

// Works with error-only returns too
deleteFile(path) ?
```

Works with `error` and `(T, error)` returns. On non-nil error, returns early with zero values + error.

**Note**: There is a space before the `?` (not like Rust).

## Nil Safety

Nil pointer dereferences are a common source of runtime panics in Go. Soppo tracks nil state through control flow and catches unsafe access at compile time.

```go
user := findUser(1)         // *User, may be nil
fmt.Println(user.Name)      // ERROR: user may be nil

if user != nil {
    fmt.Println(user.Name)  // OK: user is proven non-nil here
}

// Early return also works
if user == nil {
    return
}
fmt.Println(user.Name)      // OK: user is non-nil after the guard
```

Some expressions are automatically non-nil:
- `&expr` (address-of) - always points to a valid value
- `new(T)` - allocates and returns a valid pointer

### Escape Hatch: `.(!nil)`

When you know a pointer is non-nil from external context (e.g., API guarantees), use `.(!nil)` to assert it:

```go
// External API guarantees non-nil return in production
user := getUser().(!nil)
fmt.Println(user.Name)  // OK
```

This generates no runtime code - it's purely a compile-time assertion.

**Note**: Flow-sensitive nil tracking is complex, and we don't expect to catch every case. The goal is to catch common mistakes, not provide formal guarantees. Interface nil is a known limitation.

## Named Arguments

Positional arguments can be unclear when multiple parameters share the same type. Named arguments make call sites self-documenting without requiring wrapper structs.

```go
func createUser(name string, age int, active bool) User

createUser("alice", 30, true)                       // positional
createUser(name: "alice", age: 30, active: true)    // named
createUser("alice", age: 30, active: true)          // mixed
```

Variadic parameters are always positional at the end:

```go
func log(level string, msgs ...string)
log(level: "info", "msg1", "msg2") 
```

## String Interpolation

`fmt.Sprintf` with format verbs is error-prone - mismatched types or argument counts are only caught at runtime. Interpolation is safer and more readable.

```go
name := "alice"
age := 30
msg := "hello {name}, you are {age}"
// Codegens to: fmt.Sprintf("hello %v, you are %v", name, age)
```

Variables only, no expressions. Use `{{` to escape literal braces.

## Imports

```go
import "fmt"
import "github.com/user/project/helpers"

import (
    "fmt"
    "net/http"
    "github.com/user/project/util/helpers"
    myHelpers "github.com/user/project/util/helpers"
)
```

Soppo uses Go-style module-qualified import paths for all imports. Local Soppo packages are detected automatically:

- If an import path starts with your module path (from `go.mod`) AND
- The corresponding local directory contains `.sop` files

Then it's treated as a Soppo import and the generated Go import path is adjusted to point to the `gen/` output directory.

For example, with module `github.com/user/project`:
- `import "github.com/user/project/helpers"` → Soppo import if `helpers/` has `.sop` files
- `import "github.com/user/project/helpers"` → Go import if `helpers/` only has `.go` files
- `import "fmt"` → Go import (external package)

Like Go, Soppo imports are package-based (directory-based). Each directory with `.sop` files forms a package.

**Note**: You cannot import external Soppo projects directly. To use an external Soppo library, import its generated Go code as a regular Go import.

Types from Go packages are extracted via tree-sitter parsing.
