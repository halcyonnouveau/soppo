// Hand-written idiomatic Go equivalents of the soppo-generated ? operator patterns.
// These should be identical — the ? operator is expected to have zero overhead.
//
// Helper functions use //go:noinline to prevent the compiler from inlining and
// constant-folding them away, which would turn the benchmark into a no-op.
package runtime

import (
	"errors"
	"fmt"
	"testing"
)

var (
	errEmptyBaseline   = errors.New("empty string")
	errInvalidBaseline = errors.New("invalid id")
)

type UserTryBaseline struct {
	name string
	age  int
}

//go:noinline
func parsePortTryBaseline(s string) (int, error) {
	if s == "" {
		return 0, errEmptyBaseline
	}
	return 8080, nil
}

//go:noinline
func getUserTryBaseline(id int) (*UserTryBaseline, error) {
	if id <= 0 {
		return nil, errInvalidBaseline
	}
	return &UserTryBaseline{name: "Alice", age: 30}, nil
}

//go:noinline
func processOrderBaseline(id string) (int, error) {
	port, err := parsePortTryBaseline(id)
	if err != nil {
		return 0, err
	}

	user, err := getUserTryBaseline(port)
	if err != nil {
		return 0, err
	}

	return user.age, nil
}

//go:noinline
func processOrderBaselineWrapped(id string) (int, error) {
	port, err := parsePortTryBaseline(id)
	if err != nil {
		return 0, fmt.Errorf("parse failed: %w", err)
	}

	user, err := getUserTryBaseline(port)
	if err != nil {
		return 0, fmt.Errorf("get user failed: %w", err)
	}

	return user.age, nil
}

func BenchmarkTryPropagateBaseline(b *testing.B) {
	var sinkI int
	var sinkE error
	for b.Loop() {
		sinkI, sinkE = processOrderBaseline("8080")
	}
	_ = sinkI
	_ = sinkE
}

func BenchmarkTryPropagateErrorBaseline(b *testing.B) {
	var sinkI int
	var sinkE error
	for b.Loop() {
		sinkI, sinkE = processOrderBaseline("")
	}
	_ = sinkI
	_ = sinkE
}

func BenchmarkTryWrappedBaseline(b *testing.B) {
	var sinkI int
	var sinkE error
	for b.Loop() {
		sinkI, sinkE = processOrderBaselineWrapped("8080")
	}
	_ = sinkI
	_ = sinkE
}
