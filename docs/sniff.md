# Sniff

Sniff checks your Soppo code for common issues and suggests improvements. It runs after type-checking and has access to full type information.

```bash
sop sniff                        # Lint all project files
sop sniff src/**/*.sop           # Lint specific files/globs
sop sniff --ignore try_operator  # Ignore specific rules
```

Ignore rules globally in `sop.mod`:

```toml
[sniff]
ignore = ["try_operator"]
```

Ignore warnings with `//sniff:ignore [rule]` on the line before:

```go
//sniff:ignore try_operator
if err != nil { 
	return err 
}
```

## Rules

### `try_operator`

Detects `if err != nil { ... }` patterns that could use the `?` operator.

```go
result, err := doSomething()
if err != nil {
	return "", err
}
```

Consider rewriting as:

```go
result := doSomething() ?
```

See the [Error Handling section](guide.md#error-handling--operator) in the language guide.

### `shadow`

Warns when a variable declaration shadows a variable from an outer scope.

```go
x := 1
if condition {
	x := 2  // warning: `x` shadows a variable from an outer scope
	fmt.Println(x)
}
```

Shadowing can lead to subtle bugs where you accidentally use the wrong variable. Consider using a different name:

```go
x := 1
if condition {
	innerX := 2
	fmt.Println(innerX)
}
```

Error types are allowed to shadow (reusing `err` is idiomatic). Import shadowing is a compile error, not a lint warning.

### `unreachable`

Warns when code can never be executed because it follows a terminating statement like `return`, `break`, or `continue`.

```go
func example() {
	return
	fmt.Println("hello")  // warning: unreachable code
}
```

This also detects unreachable code after an `if` where both branches terminate:

```go
if condition {
	return
} else {
	return
}
fmt.Println("hello")  // warning: unreachable code
```
