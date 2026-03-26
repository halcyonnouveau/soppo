// Hand-written idiomatic Go equivalents of the soppo-generated nil assertion.
// These should be identical — .(!nil) is expected to compile to a nil check + panic.
//
// Helper functions use //go:noinline to prevent the compiler from inlining and
// constant-folding them away, which would turn the benchmark into a no-op.
package runtime

import "testing"

type NilUser struct {
	name string
	age  int
}

//go:noinline
func maybeUserBaseline(id int) *NilUser {
	if id <= 0 {
		return nil
	}
	return &NilUser{name: "Alice", age: 30}
}

//go:noinline
func assertNonNilBaseline(u *NilUser) *NilUser {
	if u == nil {
		panic("nil pointer dereference")
	}
	return u
}

func BenchmarkNilAssertBaseline(b *testing.B) {
	var sink *NilUser
	for b.Loop() {
		u := maybeUserBaseline(1)
		sink = assertNonNilBaseline(u)
	}
	_ = sink
}
