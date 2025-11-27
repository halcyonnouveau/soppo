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
type Color enum { 
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

`match` replaces `swtich` in Soppo.

```go
match color {
case Color.Red:
    action = "stop"
case Color.Green:
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
case Color.Red, Color.Blue:
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
user := findUser(1)       // *User, may be nil
fmt.Println(user.Name)    // ERROR: user may be nil

if user != nil {
    fmt.Println(user.Name)  // OK
}
```

## Go Interop

Standard Go imports work directly:

```go
import "fmt"
import "strings"
```

Types are extracted from Go source via tree-sitter.
