// Hand-written idiomatic Go equivalent of a function using multiple soppo features
// together: enums, pattern matching, error handling, nil checks, and string interpolation.
//
// Helper functions use //go:noinline to prevent the compiler from inlining and
// constant-folding them away, which would turn the benchmark into a no-op.
package runtime

import (
	"errors"
	"fmt"
	"testing"
)

type Role int

const (
	RoleAdmin Role = iota
	RoleUser
	RoleGuest
)

type Account struct {
	name  string
	email string
	role  Role
}

var (
	errNotFound    = errors.New("not found")
	errUnauthorised = errors.New("unauthorised")
)

//go:noinline
func fetchAccount(id int) (*Account, error) {
	if id <= 0 {
		return nil, errNotFound
	}
	switch id {
	case 1:
		return &Account{name: "Alice", email: "alice@example.com", role: RoleAdmin}, nil
	case 2:
		return &Account{name: "Bob", email: "bob@example.com", role: RoleUser}, nil
	default:
		return &Account{name: "Guest", email: "", role: RoleGuest}, nil
	}
}

//go:noinline
func fetchSupervisor(account *Account) *Account {
	if account.role == RoleAdmin {
		return nil
	}
	return &Account{name: "Alice", email: "alice@example.com", role: RoleAdmin}
}

//go:noinline
func processAccountBaseline(id int) (string, error) {
	account, err := fetchAccount(id)
	if err != nil {
		return "", fmt.Errorf("fetch failed for id %d: %w", id, err)
	}

	var summary string
	switch account.role {
	case RoleAdmin:
		summary = fmt.Sprintf("admin %s <%s>", account.name, account.email)
	case RoleUser:
		sup := fetchSupervisor(account)
		if sup == nil {
			return "", errUnauthorised
		}
		summary = fmt.Sprintf("user %s, supervised by %s", account.name, sup.name)
	case RoleGuest:
		if account.email == "" {
			summary = fmt.Sprintf("guest %s (no email)", account.name)
		} else {
			summary = fmt.Sprintf("guest %s <%s>", account.name, account.email)
		}
	}

	return summary, nil
}

func BenchmarkIntegrationBaseline(b *testing.B) {
	var sinkS string
	var sinkE error
	for b.Loop() {
		sinkS, sinkE = processAccountBaseline(1)
		sinkS, sinkE = processAccountBaseline(2)
		sinkS, sinkE = processAccountBaseline(3)
	}
	_ = sinkS
	_ = sinkE
}

func BenchmarkIntegrationErrorBaseline(b *testing.B) {
	var sinkS string
	var sinkE error
	for b.Loop() {
		sinkS, sinkE = processAccountBaseline(0)
	}
	_ = sinkS
	_ = sinkE
}
