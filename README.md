<img src="https://raw.githubusercontent.com/halcyonnouveau/soppo/refs/heads/main/docs/assets/soppo.png" alt="soppo" style="max-width: 100%;">

# Soppo

A language that compiles to Go, adding ergonomic and type safety features that Go lacks. Soppo uses Go syntax wherever possible - if you know Go, you know most of Soppo.

**[Try it in the playground](https://play.soppolang.dev)**

See [docs/DESIGN.md](docs/design.md) for language design.

Soppo is not production ready.

## Installation

The recommended way to install Soppo is through [SOPMOD](https://github.com/halcyonnouveau/sopmod), an installer and version manager for Soppo.

```sh
curl -fsSL https://soppolang.dev/install.sh | sh
sopmod install sop latest
```

You can also install Sop manually by installing [Go](https://go.dev/doc/install/) and downloading a binary from [GitHub Releases](https://github.com/halcyonnouveau/soppo/releases), or with Cargo: `cargo install soppo`.

> **Note:** SOPMOD itself is written entirely in Soppo. Check out [its source code](https://github.com/halcyonnouveau/sopmod) for a real-world example.

## Features

- **Enums:** Tagged unions with struct variants
- **Pattern matching:** Exhaustive matching with destructuring
- **Error handling:** `?` propagation and `? err { }` custom handling
- **Nil safety:** Compile-time nil checks
- **Named arguments:** `func(name: value)` for clarity
- **String interpolation:** `"value: {var}"` syntax
- **Detailed error messages:** Rust-inspired compiler diagnostics
- **Go interop:** Use any Go library directly
- **Batteries included:** LSP, formatter, test runner with doctests

## Quick Look

````go
// Enums with struct variants
type Shape enum {
	Circle struct {
		radius float64
	}
	Rectangle struct {
		width float64
		height float64
	}
}

// calcArea returns the area of a shape.
//
// Doctests - code examples that run as tests:
// ```sop
// import "fmt"
//
// circle := Shape.Circle{radius: 2.0}
// fmt.Println(calcArea(circle))
// // Output:
// // 12.56636
// ```
func calcArea(s Shape) float64 {
	var area float64

	// Pattern matching with destructuring
	match s {
	case Shape.Circle{radius: r}:
		return 3.14159 * r * r
	case Shape.Rectangle{width: w, height: h}:
		return w * h
	}
}

func printArea() error {
	// Named arguments and error propagation with `?`
	config := loadConfig(path: "app.toml") ?

	shape := Shape.Circle{radius: config.radius}
	area := calcArea(shape)

	// String interpolation
	fmt.Println("area: {area}")
	return nil
}

func main() {
	// Custom error handling with `? err { }`
	printArea() ? err {
		fmt.Println("error: {err}")
		os.Exit(1)
	}
}
````

### Error Messages

The compiler catches errors early with helpful messages:

```
  × Non-exhaustive match
    ╭─[main.sop:11:5]
 10 │         var message string
 11 │ ╭─▶     match colour {
 12 │ │       case Colour.Red:
 13 │ │           message = "Stop"
 14 │ │       case Colour.Yellow:
 15 │ │           message = "Wait"
 16 │ ├─▶     }
    · ╰──── missing variants: Green
 17 │     }
    ╰────
  help: Ensure all enum variants are handled, or add a `default` case
```

## License

BSD 3-Clause. See [LICENSE](LICENSE).
