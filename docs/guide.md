# Soppo Language Guide

This guide covers Soppo's features, syntax, and tooling. It's intended for anyone learning or using the language.

## Principles

- Reuse Go syntax where it exists
- Compile to idiomatic Go
- Catch errors at compile time, not runtime
- Do the hard work to make correctness easy
- Every feature should complement Go's design
- Always provide useful error messages

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

// Methods on variants use EnumName.Variant as receiver
func (v Result.Ok) MarshalJSON() ([]byte, error) {
	return json.Marshal(map[string]any{"Ok": v.Value})
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

// Struct matching with literal field values
match point {
case Point{x: 0, y: 0}:
	origin()
case Point{x: 0, y}:
	onYAxis(y)
case Point{x, y: 0}:
	onXAxis(x)
default:
	other(point.x, point.y)
}
```

All variants must be handled (exhaustiveness checking for enums).

When you only care about one variant, full `match` is verbose. Type assertions provide a concise way to check and destructure a single variant:

```go
shape := Shape.Circle{radius: 5.0}

// When the compiler knows the variant, direct assertion works:
circle := shape.(Shape.Circle)
area := 3.14 * circle.radius * circle.radius

// When the variant is unknown, use comma-ok form:
if c, ok := unknownShape.(Shape.Circle); ok {
	area = 3.14 * c.radius * c.radius
}

// The comma-ok form also works in if statements:
if rect, ok := shape.(Shape.Rectangle); ok {
	area = rect.width * rect.height
} else {
	// Not a rectangle
}
```

The compiler tracks which variant a variable holds. If you assign `shape := Shape.Circle{...}`, then `shape.(Shape.Circle)` is allowed without comma-ok. If the variant is unknown (e.g., received as a parameter), you must use the comma-ok form.

## Error Handling (`?` operator)

Go's `if err != nil { return err }` is verbose and repetitive. The `?` operator reduces boilerplate while keeping error handling explicit and visible.

```go
// Propagate error directly
port := parsePort(config) ?

// Custom handling
port := parsePort(config) ? {
	return fmt.Errorf("parse failed")
}

// Custom handling with named error
port := parsePort(config) ? err {
	deleteFile(path) ? deleteErr {
		return fmt.Errorf("cleanup failed after %v: %v", err, deleteErr)
	}
	return fmt.Errorf("parse failed: %v", err)
}

// Works with error-only returns too
deleteFile(path) ?
```

Works with `error` and `(T, error)` returns. On non-nil error, returns early with zero values + error.

> [!TIP]
> There is a space before the `?` (not like Rust).

## Nil Safety

Nil pointer dereferences are a common source of runtime panics in Go. Soppo uses explicit nilability annotations and flow-sensitive tracking to catch unsafe access at compile time.

Use `?` prefix to mark types that can be nil:

```go
var user *User              // non-nilable - must be initialised
var maybeUser ?*User = nil  // nilable - can hold nil

func findUser(id int) ?*User {
	if id == 0 {
		return nil
	}
	return &User{name: "Alice"}
}
```

Nilable types include `?*T`, `?[]T`, `?map[K]V`, `?chan T`, `?func(...)`, and `?Interface`.

Non-nilable types require initialisation:

```go
var user *User              // ERROR: requires initialisation
var user *User = nil        // ERROR: cannot assign nil
var user *User = &User{}    // OK
```

After a nil check, nilable types are automatically narrowed to non-nilable:

```go
user := findUser(1)  // ?*User

if user == nil {
	return
}

fmt.Println(user.name)  // OK: user is *User after guard
```

Some expressions are guaranteed non-nil: `&expr`, `new(T)`, and values after `?` succeeds.

When you know a value is non-nil, but the compiler can't prove it, use `.(!nil)` to assert. This panics if the value is nil, so prefer nil checks when possible:

```go
user := findUser(1).(!nil)  // panics if nil - use only when certain
fmt.Println(user.name)
```

Assigning a nilable pointer to an interface requires a nil check first:

```go
func getError() error {
	var p ?*MyError = nil
	return p  // ERROR: nilable pointer assigned to interface
}

// Fix: check first or return nil directly
func getError() error {
	var p ?*MyError = maybeGetError()
	if p == nil {
		return nil  // OK: explicit nil
	}
	return p  // OK: p is non-nil here
}
```

This prevents a Go gotcha where a non-nil interface can wrap a nil pointer.

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
msg2 := "total: {len(items) * 2}"

// Format specifiers: {expr:spec}
fmt.Println("Hex: {num:x}, Padded: {num:08d}")
fmt.Println("Price: {price:.2f}")
fmt.Println("Debug: {items:#v}")
```

Format specifiers follow Go's `fmt` verbs (`d`, `x`, `b`, `f`, `e`, `s`, `t`, `v`, etc.). The compiler validates that specifiers match the expression type.

> [!TIP]
> Use `{{` and `}}` to escape literal braces.

## Go Interop and Imports

```go
import "fmt"
import "github.com/user/project/helpers"

import (
	"fmt"
	"net/http"
	myHelpers "github.com/user/project/util/helpers"
)
```

Soppo uses Go-style import paths. Local Soppo packages are detected automatically when the import path starts with your module path (from `go.mod`) and the directory contains `.sop` files. Otherwise, the import is treated as a Go package.

Types from external Go packages are assumed nilable since Go has no nilability annotations. Use nil checks or `.(!nil)` when passing to Soppo functions expecting non-nilable types.

Unlike Go, import names cannot be shadowed by local variables.

> [!NOTE]
> You cannot import external Soppo source directly. To use an external Soppo library, import its generated Go code. Soppo preserves type information (nilability, enums) in generated code via special comments, so cross-project nil safety is maintained.

## Build System

Soppo uses `sop.mod` for project configuration. Create one in your project root (alongside `go.mod`):

```toml
include = ["**/*.sop"]
exclude = ["testdata/**"]
output = "gen"
```

All fields are optional. Without `include`, all `.sop` files are compiled. Without `output`, `.go` files are generated next to their source files.

```bash
sop build                        # Uses sop.mod config
sop build src/**/*.sop           # Compile specific files/globs
sop build --output gen           # Output to directory
sop check                        # Type-check without codegen
```

Without `sop.mod`, you must provide files or glob patterns as arguments.

> [!NOTE]
> A `go.mod` file is still required for module path resolution.

## Testing

Test files use the `_test.sop` suffix and have access to all symbols in their package. Code examples in doc comments (doctests) are extracted and run as Go Example functions.

```go
// Add returns the sum of two integers.
//
// ```sop
// import "fmt"
//
// result := math.Add(1, 2)
// fmt.Println(result)
// // Output:
// // 3
// ```
func Add(a, b int) int {
    return a + b
}
```

Use ` ```sop ` or ` ```soppo ` fences. Imports must be explicit; the documented package is auto-imported. Attributes like `ignore`, `no_run`, and `should_panic` control execution.

```bash
sop test                         # Run all tests
sop test -v                      # Verbose output
sop test -run TestAdd            # Filter by pattern
sop test -- -cover               # Pass flags to go test
```
