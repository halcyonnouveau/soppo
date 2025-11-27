<img src="./docs/assets/soppo.png" alt="soppo" style="max-width: 100%;">

# Soppo

A language that compiles to Go, adding ergonomic and type safety features that Go lacks. Soppo uses Go syntax wherever possible - if you know Go, you know most of Soppo.

See [docs/DESIGN.md](docs/DESIGN.md) for language design.

## Quick Look

```go
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

func calcArea(s Shape) float64 {
	var area float64

	// Pattern matching with destructuring
	match s {
	case Shape.Circle{radius: r}:
		area = 3.14159 * r * r
	case Shape.Rectangle{width: w, height: h}:
		area = w * h
	}

	return area
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
```

## License

BSD 3-Clause. See [LICENSE](LICENSE).
