# Soppo Language Design

A language that compiles to Go, adding enums and pattern matching. Syntax follows Go as closely as possible.

## Core Principles

- **Go syntax compatibility** - If Go has syntax for something, use it
- Transpile to idiomatic Go code
- Focus on type safety via enums and exhaustive pattern matching

## Enums

```go
// Simple enums (no data)
type Color enum {
    Red
    Green
    Blue
}

// Single value variants (Go struct field syntax)
type Result enum {
    Ok int
    Err string
}

// Struct variants
type Shape enum {
    Circle struct { radius float64 }
    Rectangle struct { width float64; height float64 }
}

// Generic enums
type Option[T any] enum {
    Some T
    None
}
```

Transpiles to interface + concrete types:

```go
type Color interface { isColor() }

type Red struct{}
func (Red) isColor() {}

type Green struct{}
func (Green) isColor() {}
```

## Pattern Matching

Match is a statement (like Go's switch), not an expression. Uses qualified names (`Type.Variant`) and parentheses for data extraction:

```go
// Match on enums - assign within each arm
var action string
match color {
case Color.Red:
    action = "stop"
case Color.Green:
    action = "go"
case Color.Yellow:
    action = "slow"
}

// Extract data from variants
var result int
match opt {
case Option.Some(value):
    result = value * 2
case Option.None:
    result = 0
}

// Struct variant destructuring (planned)
var area float64
match shape {
case Shape.Circle{radius: r}:
    area = 3.14 * r * r
case Shape.Rectangle{width: w, height: h}:
    area = w * h
}

// Match with side effects only (no assignment needed)
match event {
case Event.Click(x, y):
    handleClick(x, y)
case Event.KeyPress(key):
    handleKey(key)
}
```

Transpiles to Go type switches.

## Functions

Go syntax - no colons in parameter lists:

```go
// Regular function
func add(x int, y int) int {
    return x + y
}

// Generic function
func identity[T any](x T) T {
    return x
}

// Method with receiver
func (c Counter) increment() Counter {
    c.value = c.value + 1
    return c
}
```

## Structs

Standard Go syntax:

```go
type User struct {
    name string
    age int
}
```

## Error Handling

The `?` operator handles Go's `(T, error)` return pattern with less boilerplate.

### Basic Usage

```go
// Auto-return error if not nil
func process() (Config, error) {
    port := parsePort(config) ?
    return Config{port: port}, nil
}
```

If `parsePort` returns a non-nil error, the function immediately returns with that error.

### With Error Wrapping

```go
func process() (Config, error) {
    port := parsePort(config) ? {
        return Config{}, fmt.Errorf("parse failed: %v", err)
    }
    return Config{port: port}, nil
}
```

The block after `?` has an implicit `err` variable containing the error.

### Transpilation

```go
// Soppo
r := SomeFunction() ?

// Becomes Go
r, _err := SomeFunction()
if _err != nil {
    return _zero, _err  // returns zero values + error
}
```

With custom handling:
```go
// Soppo
r := SomeFunction() ? {
    return Config{}, fmt.Errorf("failed: %v", err)
}

// Becomes Go
r, err := SomeFunction()
if err != nil {
    return Config{}, fmt.Errorf("failed: %v", err)
}
```

## Nil Safety

Go allows dereferencing nil pointers, which crashes at runtime:

```go
// Go - compiles fine, crashes at runtime
var user *User = nil
fmt.Println(user.Name)  // panic: nil pointer dereference
```

Soppo uses flow-sensitive nil tracking to catch this at compile time:

```go
// Soppo
user := findUser(1)       // *User, may be nil
fmt.Println(user.Name)    // ERROR: user may be nil

if user != nil {
    fmt.Println(user.Name)  // OK: compiler knows non-nil here
}
```

The compiler tracks which variables have been nil-checked and only allows dereferencing after a check. No new syntax - just smarter type checking.

## Exhaustiveness Checking

All enum variants must be handled:

```go
// ERROR: non-exhaustive match, missing: Yellow
match color {
case Color.Red:
    doRed()
case Color.Green:
    doGreen()
}
```

## Go Interop

Go packages work with standard `import`:

```go
import "fmt"
fmt.Println("hello")
```

## Build Commands

```
soppo run main.sop   # Transpile and run
soppo build          # Transpile all .sop files
soppo check          # Type check only
```
