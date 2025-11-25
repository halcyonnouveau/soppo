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

Uses qualified names (`Type.Variant`) and parentheses for data extraction:

```go
// Match expression (returns value)
result := match opt {
case Option.Some(value):
    value * 2
case Option.None:
    0
}

// Match on simple enums
action := match color {
case Color.Red:
    "stop"
case Color.Green:
    "go"
case Color.Yellow:
    "slow"
}

// Struct variant destructuring
area := match shape {
case Shape.Circle{radius: r}:
    3.14 * r * r
case Shape.Rectangle{width: w, height: h}:
    w * h
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

## Error Handling (planned)

### Result Type

```go
type Result[T any, E any] enum {
    Ok T
    Err E
}
```

### The ? Operator

```go
func process() (Config, error) {
    port := parsePort(config)?  // Returns early if err != nil
    return Config{port: port}, nil
}
```

## Exhaustiveness Checking (planned)

All enum variants must be handled:

```go
// ERROR: non-exhaustive match, missing: Yellow
match color {
case Color.Red: ...
case Color.Green: ...
}
```

## Standard Library (planned)

**Prelude** - Auto-imported:
- `Option[T]`, `Some`, `None`
- `Result[T, E]`, `Ok`, `Err`

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
