# Soppo Language Design

A language that compiles to Go, adding enums, pattern matching, and safer error/nil handling.

## Principles

- Go syntax compatibility - if Go has syntax for it, use it
- Compile to idiomatic Go
- Type safety via enums, exhaustive matching, nil tracking
- Rust-inspired error messages

## Architecture

```
Source (.sop) -> Parser -> AST -> Type Checking/Inference -> Codegen -> Output (.go)
                                            ^
                                   External .go packages
```

Go imports are resolved by parsing Go source files with tree-sitter to extract type signatures.

## Enums

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

`match` replaces `switch` in Soppo.

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

Works with `error` and `(T, error)` returns. On non-nil error, returns early with zero values + error.

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

**Note**: There is a space before the `?` (not like Rust).

## Named Arguments

```go
func createUser(name string, age int, active bool) User

createUser("alice", 30, true)                       // positional
createUser(name: "alice", age: 30, active: true)    // named
createUser("alice", age: 30, active: true)          // mixed

createUser(name: "alice", 30)  // ERROR - positional must come before named
```

Variadic params are always positional at the end:

```go
func log(level string, msgs ...string)
log(level: "info", "msg1", "msg2")  // ok
```

## String Interpolation

```go
name := "alice"
age := 30
msg := "hello {name}, you are {age}"
// Codegens to: fmt.Sprintf("hello %v, you are %v", name, age)
```

Variables only, no expressions. Use `{{` to escape literal braces.

## Nil Safety

Flow-sensitive nil tracking:

```go
user := findUser(1)         // *User, may be nil
fmt.Println(user.Name)      // ERROR: user may be nil

if user != nil {
    fmt.Println(user.Name)  // OK
}
```

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
