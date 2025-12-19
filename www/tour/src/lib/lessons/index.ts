export interface Lesson {
  slug: string;
  title: string;
  content: string;
  code: string;
}

export const lessons: Lesson[] = [
  {
    slug: "welcome",
    title: "Welcome",
    content: `Soppo is a language that compiles to Go, adding features like enums, pattern matching, nil safety, and ergonomic error handling.

This tour will walk you through Soppo's features with interactive examples. Edit the code on the right and see the output update automatically.

**Try it:** Change the name in the code and watch the output change.`,
    code: `package main

import "fmt"

func main() {
	name := "World"
	fmt.Println("Hello, {name}!")
}
`,
  },
  {
    slug: "strings",
    title: "Strings",
    content: `Soppo adds string interpolation to Go. Instead of \`fmt.Sprintf\`, you can embed expressions directly in strings using curly braces.

Any valid expression works inside the braces, including arithmetic, function calls, and field access.

Use double braces to include a literal brace in your string.

**Try it:** Add another interpolated value to the output.`,
    code: `package main

import "fmt"

func main() {
	name := "Soppo"
	version := 0.6

	fmt.Println("{name} v{version}")
	fmt.Println("2 + 2 = {2 + 2}")
	fmt.Println("Length: {len(name)}")

	// Escaped braces
	fmt.Println("Use {{braces}} for interpolation")
}
`,
  },
  {
    slug: "enums",
    title: "Enums",
    content: `Soppo adds enums (sum types) to Go. An enum defines a type with a fixed set of possible values, called variants.

Simple enums have unit variants with no associated data. The compiler ensures you can only use valid variants.

**Try it:** Add a new color variant and use it.`,
    code: `package main

import "fmt"

type Colour enum {
	Red
	Yellow
	Green
}

func main() {
	light := Colour.Red

	fmt.Println("Light is:", light)
	fmt.Println("Is red?", light == Colour.Red)
}
`,
  },
  {
    slug: "enum-data",
    title: "Enum Data",
    content: `Enum variants can carry associated data. Each variant can hold different types and amounts of data.

This is what makes enums true sum types: a value is one variant OR another, and each variant can have its own shape.

**Try it:** Add a Triangle variant with a base and height.`,
    code: `package main

import "fmt"

type Shape enum {
	Circle struct {
		radius float64
	}
	Rectangle struct {
		width  float64
		height float64
	}
}

func main() {
	c := Shape.Circle{radius: 5.0}
	r := Shape.Rectangle{width: 3.0, height: 4.0}

	fmt.Println("Circle:", c)
	fmt.Println("Rectangle:", r)
}
`,
  },
  {
    slug: "match",
    title: "Match",
    content: `The match expression lets you branch on enum variants and destructure their data in one step.

Each case binds the variant's fields to variables you can use in that branch. Note that match replaces switch in Soppo; there is no switch statement.

**Try it:** Add a case for the Triangle variant if you created one earlier.`,
    code: `package main

import "fmt"

type Shape enum {
	Circle struct { radius float64 }
	Rectangle struct { width, height float64 }
}

func area(s Shape) float64 {
	match s {
	case Shape.Circle{radius: r}:
		return 3.14159 * r * r
	case Shape.Rectangle{width: w, height: h}:
		return w * h
	}
}

func main() {
	c := Shape.Circle{radius: 5.0}
	r := Shape.Rectangle{width: 3.0, height: 4.0}

	fmt.Println("Circle area:", area(c))
	fmt.Println("Rectangle area:", area(r))
}
`,
  },
  {
    slug: "exhaustive",
    title: "Exhaustive",
    content: `Match expressions must be exhaustive: every possible variant must be handled. The compiler will reject code that misses a case.

This catches bugs at compile time. When you add a new variant, the compiler tells you everywhere that needs updating.

Use \`default\` to handle any remaining cases when you don't need to match every variant explicitly.

**Try it:** Remove one of the cases and see the compiler error.`,
    code: `package main

import "fmt"

type Status enum {
	Pending
	Running
	Done
	Failed
}

func describe(s Status) string {
	match s {
	case Status.Pending:
		return "Waiting to start"
	case Status.Running:
		return "In progress"
	case Status.Done:
		return "Completed"
	case Status.Failed:
		return "Error occurred"
	}
}

func main() {
	fmt.Println(describe(Status.Pending))
	fmt.Println(describe(Status.Done))
}
`,
  },
  {
    slug: "nil-safety",
    title: "Nil Safety",
    content: `In Soppo, pointers are non-nil by default. A \`*T\` cannot be nil. To allow nil, use \`?*T\` (a nilable pointer).

This prevents nil pointer dereferences at compile time. The compiler tracks which pointers might be nil and requires you to handle that case.

**Try it:** Change the return type to \`*User\` (removing the \`?\`) and see what happens.`,
    code: `package main

import "fmt"

type User struct {
	name string
}

func findUser(id int) ?*User {
	if id == 1 {
		return &User{name: "Alice"}
	}
	return nil
}

func main() {
	user := findUser(1)
	fmt.Println("Found:", user)

	nobody := findUser(0)
	fmt.Println("Not found:", nobody)
}
`,
  },
  {
    slug: "narrowing",
    title: "Narrowing",
    content: `After a nil check, Soppo automatically narrows nilable types to non-nilable. The compiler tracks control flow to know when a value can't be nil.

This works with \`if\` checks and early returns. Once the compiler proves a value is non-nil, you can use it without further checks.

**Try it:** Remove the nil check and see the compiler error.`,
    code: `package main

import "fmt"

type User struct {
	name string
}

func findUser(id int) ?*User {
	if id == 1 {
		return &User{name: "Alice"}
	}
	return nil
}

func main() {
	user := findUser(1)

	// user is ?*User here
	if user == nil {
		fmt.Println("Not found")
		return
	}

	// user is *User here (narrowed)
	fmt.Println("Hello,", user.name)
}
`,
  },
  {
    slug: "errors",
    title: "Errors",
    content: `The \`?\` operator simplifies Go's \`if err != nil { return err }\` pattern. Place \`?\` after a call that returns an error to propagate it automatically.

If the error is non-nil, the function returns early with zero values and the error. Note the space before \`?\`.

**Try it:** Change \`parsePort("8080")\` to \`parsePort("")\` to trigger an error.`,
    code: `package main

import (
	"errors"
	"fmt"
)

func parsePort(s string) (int, error) {
	if s == "" {
		return 0, errors.New("empty port")
	}
	return 8080, nil
}

func connect() (string, error) {
	port := parsePort("8080") ?
	return "Connected on port {port}", nil
}

func main() {
	result, err := connect()
	if err != nil {
		fmt.Println("Error:", err)
		return
	}
	fmt.Println(result)
}
`,
  },
  {
    slug: "error-handling",
    title: "Error Handling",
    content: `You can add a block after \`?\` to customize error handling. The block receives the error and must return.

Name the error with \`? err { ... }\` to access it in the block. This lets you wrap errors with context or handle them differently.

**Try it:** Modify the error message in the custom handler.`,
    code: `package main

import (
	"errors"
	"fmt"
)

func fetchData(id int) (string, error) {
	if id <= 0 {
		return "", errors.New("invalid id")
	}
	return "data-{id}", nil
}

func process(id int) (string, error) {
	// Custom error handling with named error
	data := fetchData(id) ? err {
		return "", fmt.Errorf("fetch failed for id %d: %v", id, err)
	}

	return "Processed: {data}", nil
}

func main() {
	result, err := process(0)
	if err != nil {
		fmt.Println("Error:", err)
		return
	}
	fmt.Println(result)
}
`,
  },
  {
    slug: "named-args",
    title: "Named Args",
    content: `Soppo supports named arguments at call sites. This makes code clearer when functions have multiple parameters of the same type.

You can mix positional and named arguments. Named arguments use the parameter name followed by a colon.

**Try it:** Reorder the named arguments - they don't have to match parameter order.`,
    code: `package main

import "fmt"

func createUser(name string, age int, active bool) {
	fmt.Println("Name:", name)
	fmt.Println("Age:", age)
	fmt.Println("Active:", active)
}

func main() {
	// Positional
	createUser("Alice", 30, true)

	fmt.Println("---")

	// Named
	createUser(name: "Bob", age: 25, active: false)

	fmt.Println("---")

	// Mixed (positional first, then named)
	createUser("Charlie", age: 35, active: true)
}
`,
  },
  {
    slug: "generic-enums",
    title: "Generic Enums",
    content: `Enums can be generic, taking type parameters. This lets you create reusable patterns like \`Option[T]\` for optional values or \`Result[T, E]\` for error handling.

Generic enums combine the power of sum types with Go's generics. The type parameter is specified in square brackets.

**Try it:** Create a \`Result[T, E]\` enum with \`Ok\` and \`Err\` variants.`,
    code: `package main

import "fmt"

type Option[T any] enum {
	Some struct {
	  Value T
	}
	None
}

func divide(a, b int) Option[int] {
	if b == 0 {
		return Option.None
	}
	return Option.Some{Value: a / b}
}

func main() {
	result := divide(10, 2)
	match result {
	case Option.Some{Value: v}:
		fmt.Println("Result:", v)
	case Option.None:
		fmt.Println("Cannot divide by zero")
	}

	result2 := divide(10, 0)
	match result2 {
	case Option.Some{Value: v}:
		fmt.Println("Result:", v)
	case Option.None:
		fmt.Println("Cannot divide by zero")
	}
}
`,
  },
  {
    slug: "type-assert",
    title: "Type Assert",
    content: `When you only need to handle one variant, use type assertion syntax instead of a full match. The \`.(Variant)\` syntax extracts the variant's data directly.

The compiler tracks which variant a variable holds. When the variant is known at compile time, you can use direct assertion. When it's unknown (e.g., passed as a parameter), use the comma-ok form.

**Try it:** Move the shape creation into a function and pass it to main to see when comma-ok is required.`,
    code: `package main

import "fmt"

type Shape enum {
	Circle struct { radius float64 }
	Rectangle struct { width, height float64 }
}

func main() {
	shape := Shape.Circle{radius: 5.0}

	// Direct assertion - compiler knows it's Circle
	circle := shape.(Shape.Circle)
	fmt.Println("Radius:", circle.radius)

	// For unknown variants, use comma-ok form
	if rect, ok := shape.(Shape.Rectangle); ok {
		fmt.Println("Width:", rect.width)
	} else {
		fmt.Println("Not a rectangle")
	}
}
`,
  },
  {
    slug: "nil-assert",
    title: "Nil Assert",
    content: `The \`.(!nil)\` assertion converts a nilable pointer to a non-nil pointer, panicking if the value is nil. Use this when you're certain a value isn't nil but the compiler can't prove it.

This is similar to force-unwrapping in Swift or \`unwrap()\` in Rust. Use sparingly and prefer nil checks when possible.

**Try it:** Change \`findUser(1)\` to \`findUser(0)\` and see the panic.`,
    code: `package main

import "fmt"

type User struct {
	name string
}

func findUser(id int) ?*User {
	if id == 1 {
		return &User{name: "Alice"}
	}
	return nil
}

func main() {
	// We know user 1 exists
	user := findUser(1).(!nil)
	fmt.Println("Hello,", user.name)

	// This would panic:
	// nobody := findUser(0).(!nil)
}
`,
  },
];

export function getLesson(slug: string): Lesson | undefined {
  return lessons.find((l) => l.slug === slug);
}
