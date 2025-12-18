# Soppo Language Design

This document describes the design of Soppo - the principles that guide development and the features it provides. It's intended for contributors and anyone wanting to understand why Soppo works the way it does.

## Principles

- Reuse Go syntax where it exists
- Compile to idiomatic Go
- Catch errors at compile time, not runtime
- Do the hard work to make correctness easy
- Every feature should complement Go's design
- Always provide useful error messages

## Compiler Architecture

```
Source (.sop) -> Parser -> AST -> Type Checking/Inference -> Codegen -> Output (.go)
                                            ^
                                   External .go packages
```

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

> [!WARNING]
> Using match as the final statement in a returning function is semantically valid in Soppo, but the generated Go code won't compile. Go doesn't recognise exhaustive switch statements, it requires a return after the switch even when all cases return. Assign to a variable instead:
>
> ```go
> // Semantically correct, but generated Go won't compile
> func describe(c Colour) string {
>     match c {
>     case Colour.Red:
>         return "red"
>     case Colour.Green:
>         return "green"
>     case Colour.Blue:
>         return "blue"
>     }
> }
>
> // Use assignment instead
> func describe(c Colour) string {
>     var result string
>     match c {
>     case Colour.Red:
>         result = "red"
>     case Colour.Green:
>         result = "green"
>     case Colour.Blue:
>         result = "blue"
>     }
>     return result
> }
> ```

When you only care about one variant, full `match` is verbose. Combining `if` with a type assertion provides a concise way to check and destructure a single variant:

```go
// Instead of:
match opt {
case Option.Some(x):
	use(x)
case Option.None:
	// nothing
}

// Write:
if x := opt.(Option.Some) {
	use(x)
}

// With else block:
if x := opt.(Option.Some) {
	use(x)
} else {
	handleNone()
}

// For struct variants, binding gets the full struct:
if c := shape.(Shape.Circle) {
	area = 3.14 * c.radius * c.radius
}

// Unit variants work too (binding is the zero struct):
if _ := colour.(Colour.Red) {
	stop()
}
```

Uses Go's type assertion syntax `.(Type)` but the block only executes if the variant matches. The binding receives the variant's data (inner value for single-value variants, full struct for struct variants).

> [!NOTE]
> Type assertions behave differently for Soppo enums vs Go interfaces. Enum variant assertions (e.g., `opt.(Option.Some)`) return a nilable pointer for the if-init pattern above. Go interface assertions (e.g., `iface.(int)`) return the actual type, matching standard Go semantics.

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
		return nil          // OK: return type is nilable
	}
	return &User{name: "Alice"}
}
```

Nilable types include:
- `?*T` - nilable pointer
- `?[]T` - nilable slice
- `?map[K]V` - nilable map
- `?chan T` - nilable channel
- `?func(...)` - nilable function
- `?Interface` - nilable interface

Non-nilable types with nil zero values require initialisation:

```go
var user *User              // ERROR: non-nilable type requires initialisation
var user *User = nil        // ERROR: cannot assign nil to non-nilable type
var user *User = &User{}    // OK
```

After a nil check, nilable types are automatically narrowed to non-nilable:

```go
result := findUser(1)    // ?*User

if result != nil {
	fmt.Println(result.name)    // OK: result is *User here
	printUser(result)           // OK: can pass to func expecting *User
}

// Early return also works
if result == nil {
	return
}

fmt.Println(result.name)    // OK: result is non-nil after guard
```

Some expressions are guaranteed non-nil:
- `&expr` (address-of) - always points to a valid value
- `new(T)` - allocates and returns a valid pointer
- Nilable results after `?` succeeds
```go
ptr := &User{}              // *User (non-nilable)
newUser := new(User)        // *User (non-nilable)

func getUser(id int) (*User, error) { ... }
func getNames() ([]string, error) { ... }
func getScores() (map[string]int, error) { ... }

func process() error {
    user := getUser(1) ?    // If we get here, error was nil
    fmt.Println(user.name)  // OK: user is non-nil after ?
    return nil
}
```

When you know a value is non-nil from external context (e.g., API guarantees), use `.(!nil)` to assert it:

```go
user := findUser(1).(!nil)
fmt.Println(user.name)  // OK: user is *User
```

This generates no runtime code - it's purely a compile-time assertion.

Types from external Go packages default to nilable since Go has no nilability annotations. Use nil checks or `.(!nil)` when passing to Soppo functions expecting non-nilable types.

> [!NOTE]
> Flow-sensitive nil tracking is complex, and we don't expect to catch every case. The goal is to catch common mistakes, not provide formal guarantees. One notable limitation is Go's interface nil behaviour, an interface is only `== nil` when both its type and value are nil:
>
> ```go
> func getError() error {
>     var p *MyError = nil
>     return p  // interface has (type=*MyError, value=nil)
> }
>
> err := getError()
> if err != nil {
>     // This runs! The interface isn't nil because it has a concrete type
> }
> ```
>
> Soppo supports `?Interface` but currently cannot detect when a non-nil interface wraps a nil concrete value.

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
// Expressions are also supported
msg2 := "total: {len(items) * 2}"
```

> [!TIP]
> Use `{{` and `}}` to escape literal braces.

## Go Interop and Imports

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

Then it's treated as a Soppo import and the generated Go import path is adjusted to point to the compilation output directory.

For example, with module `github.com/user/project`:
- `import "github.com/user/project/helpers"`: Soppo import if `helpers/` has `.sop` files
- `import "github.com/user/project/helpers"`: Go import if `helpers/` only has `.go` files
- `import "fmt"`: Go import (external package)

Like Go, Soppo imports are package-based (directory-based). Each directory with `.sop` files forms a package. Types from Go packages are extracted via tree-sitter parsing.

Soppo-generated Go code includes special markers that preserve type information when re-imported:

```go
//soppo:generated             // File marker
//soppo:nilable user : 0      // Function annotation - param "user" and return 0 are nilable
//soppo:enum                  // Type annotation - struct is an enum variant

type User struct {
	Name  string
	Address *Address //soppo:nilable
}
```

When importing a Go package with `//soppo:generated`:
- Fields with `//soppo:nilable` are treated as nilable
- Fields **without** the marker are non-nilable
- Types marked `//soppo:enum` support pattern matching

When importing regular Go packages (no marker):
- All pointer/slice/map/chan/func/interface types are assumed nilable

This allows Soppo projects to depend on generated Go from other Soppo projects while preserving nil safety guarantees.

> [!IMPORTANT]
> You cannot import external Soppo source directly. To use an external Soppo library, import its generated Go code as a regular Go import.

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
