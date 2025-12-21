//soppo:generated
package main

/*soppo:enum
Option[T] {
    Some {
        value T
    }
    None
}
*/
type Option[T any] interface {
	isOption()
}

type Option_Some[T any] struct {
	value T
}
func (Option_Some[T]) isOption() {}

type Option_None[T any] struct {}
func (Option_None[T]) isOption() {}
func (Option_None[T]) String() string { return "None" }

func OptionNone[T any]() Option[T] {
	return Option_None[T]{}
}

func main() {
	x := Option.None[String]
	_ = x
}

