// Hand-written idiomatic Go equivalents of the soppo-generated string interpolation.
// These should be identical — interpolation is expected to compile to fmt.Sprintf.
//
// Helper functions use //go:noinline to prevent the compiler from inlining and
// constant-folding them away, which would turn the benchmark into a no-op.
package runtime

import (
	"fmt"
	"testing"
)

//go:noinline
func greetBaseline(name string, age int) string {
	return fmt.Sprintf("hello %s, you are %d", name, age)
}

//go:noinline
func formatHexBaseline(num int) string {
	return fmt.Sprintf("value: %x, padded: %08x", num, num)
}

func BenchmarkInterpSimpleBaseline(b *testing.B) {
	var sink string
	for b.Loop() {
		sink = greetBaseline("alice", 30)
	}
	_ = sink
}

func BenchmarkInterpFormatBaseline(b *testing.B) {
	var sink string
	for b.Loop() {
		sink = formatHexBaseline(255)
	}
	_ = sink
}
