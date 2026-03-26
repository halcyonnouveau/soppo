// Hand-written idiomatic Go equivalents of the soppo-generated enum patterns.
//
// Helper functions use //go:noinline to prevent the compiler from inlining and
// constant-folding them away, which would turn the benchmark into a no-op.
package runtime

import "testing"

type ShapeKind int

const (
	ShapeKindCircle ShapeKind = iota
	ShapeKindRectangle
	ShapeKindTriangle
)

type ShapeBaseline struct {
	kind               ShapeKind
	radius             float64
	width, height      float64
	triBase, triHeight float64
}

//go:noinline
func areaBaseline(s ShapeBaseline) float64 {
	switch s.kind {
	case ShapeKindCircle:
		return 3.14159 * s.radius * s.radius
	case ShapeKindRectangle:
		return s.width * s.height
	case ShapeKindTriangle:
		return 0.5 * s.triBase * s.triHeight
	}
	return 0
}

//go:noinline
func unwrapOrBaseline(val int, err string, defaultVal int) int {
	if err != "" {
		return defaultVal
	}
	return val
}

//go:noinline
func divideBaseline(a, b int) (int, bool) {
	if b == 0 {
		return 0, false
	}
	return a / b, true
}

func BenchmarkEnumMatchBaseline(b *testing.B) {
	shapes := []ShapeBaseline{
		{kind: ShapeKindCircle, radius: 5.0},
		{kind: ShapeKindRectangle, width: 3.0, height: 4.0},
		{kind: ShapeKindTriangle, triBase: 6.0, triHeight: 3.0},
	}

	var sink float64
	for b.Loop() {
		for _, s := range shapes {
			sink = areaBaseline(s)
		}
	}
	_ = sink
}

func BenchmarkResultUnwrapBaseline(b *testing.B) {
	var sink int
	for b.Loop() {
		sink = unwrapOrBaseline(42, "", 0)
		sink = unwrapOrBaseline(0, "failed", 99)
	}
	_ = sink
}

func BenchmarkOptionDivideBaseline(b *testing.B) {
	var sink0 int
	var sink1 bool
	for b.Loop() {
		sink0, sink1 = divideBaseline(10, 2)
		sink0, sink1 = divideBaseline(10, 0)
	}
	_ = sink0
	_ = sink1
}
