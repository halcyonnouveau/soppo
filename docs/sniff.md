# Sniff

Sniff checks your Soppo code for common issues and suggests improvements. It runs after type-checking and has access to full type information.

```bash
sop sniff                        # Lint all project files
sop sniff src/**/*.sop           # Lint specific files/globs
sop sniff --disable try_operator # Disable specific rules
```

Disable rules globally in `sop.mod`:

```toml
[sniff]
disable = ["try_operator"]
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
