// Hand-written idiomatic Go equivalents of the soppo-generated ? operator patterns.
// These should be identical — the ? operator is expected to have zero overhead.
package runtime

import (
	"fmt"
	"testing"
)

// --- Idiomatic Go: manual if err != nil ---

func processOrderBaseline(id string) (int, error) {
	port, err := parsePortTry(id)
	if err != nil {
		return 0, err
	}

	user, err := getUserTry(port)
	if err != nil {
		return 0, err
	}

	return user.age, nil
}

func processOrderBaselineWrapped(id string) (int, error) {
	port, err := parsePortTry(id)
	if err != nil {
		return 0, fmt.Errorf("parse failed: %w", err)
	}

	user, err := getUserTry(port)
	if err != nil {
		return 0, fmt.Errorf("get user failed: %w", err)
	}

	return user.age, nil
}

// --- Benchmarks ---

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
