<img src="./docs/soppo.png" alt="soppo" style="max-width: 100%;">

# Soppo

A language that compiles to Go, adding type expressiveness and better developer ergonomics.

Soppo brings enums, pattern matching, and safer error handling to Go while staying close to Go's syntax and idioms. It compiles to idiomatic Go code.

## Features

- **Enums with variants** - Define sum types that the compiler understands
- **Exhaustive pattern matching** - The compiler catches missing cases
- **No nil pointers** - Use `Option[T]` instead
- **Error handling with `?`** - Propagate errors cleanly
- **Named arguments** - Python-style parameter naming
- **Go interop** - Use existing Go packages seamlessly

## License

Licensed under the BSD 3-Clause License. See [LICENSE](LICENSE) for details.
