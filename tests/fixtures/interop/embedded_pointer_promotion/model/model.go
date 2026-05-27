// Package model exercises promotion of embedded pointer fields through a chain
// of embeddings (Child embeds *Mid, Mid embeds *Base).
package model

type Base struct {
	id int
}

type Mid struct {
	tag string
	*Base
}

type Child struct {
	label string
	*Mid
}

func NewChild() *Child {
	return &Child{Mid: &Mid{Base: &Base{}}}
}

// AcceptBase requires a *Base, so the promoted child.Base field must resolve to
// a pointer rather than a value.
func AcceptBase(b *Base) string {
	if b == nil {
		return "nil"
	}
	return "base"
}
