// Package runtime provides runtime support for Soppo attributes.
//
// Attributes are registered at init() time by generated code and can be
// queried by libraries at runtime.
package runtime

import "sync"

// registry stores attributes by target and field name.
// Structure: registry[target][field] = []any
var registry = map[string]map[string][]any{}
var registryMu sync.RWMutex

// RegisterAttr registers an attribute for a target and field.
//
// Target naming convention:
//   - Struct fields: "pkg.StructName" + "FieldName"
//   - Functions: "pkg.FuncName" + ""
//   - Methods: "pkg.TypeName.MethodName" + ""
//   - Types: "pkg.TypeName" + ""
//   - Enum variants: "pkg.EnumName" + "VariantName"
func RegisterAttr(target, field string, attr any) {
	registryMu.Lock()
	defer registryMu.Unlock()

	if registry[target] == nil {
		registry[target] = map[string][]any{}
	}
	registry[target][field] = append(registry[target][field], attr)
}

// GetAttr returns the first attribute of type T for a target and field.
// Returns the zero value and false if no matching attribute is found.
func GetAttr[T any](target, field string) (T, bool) {
	registryMu.RLock()
	defer registryMu.RUnlock()

	if fields, ok := registry[target]; ok {
		for _, attr := range fields[field] {
			if v, ok := attr.(T); ok {
				return v, true
			}
		}
	}
	var zero T
	return zero, false
}

// GetAttrs returns all attributes of type T for a target and field.
func GetAttrs[T any](target, field string) []T {
	registryMu.RLock()
	defer registryMu.RUnlock()

	var result []T
	if fields, ok := registry[target]; ok {
		for _, attr := range fields[field] {
			if v, ok := attr.(T); ok {
				result = append(result, v)
			}
		}
	}
	return result
}

// HasAttr checks if a target and field has any attribute of type T.
func HasAttr[T any](target, field string) bool {
	_, ok := GetAttr[T](target, field)
	return ok
}

// AllTargets returns all registered targets.
func AllTargets() []string {
	registryMu.RLock()
	defer registryMu.RUnlock()

	targets := make([]string, 0, len(registry))
	for target := range registry {
		targets = append(targets, target)
	}
	return targets
}

// AllFields returns all registered fields for a target.
func AllFields(target string) []string {
	registryMu.RLock()
	defer registryMu.RUnlock()

	if fields, ok := registry[target]; ok {
		result := make([]string, 0, len(fields))
		for field := range fields {
			result = append(result, field)
		}
		return result
	}
	return nil
}

// EnumVariant holds metadata about an enum variant.
// Registered as an attribute with target=EnumName, field=VariantName.
type EnumVariant struct {
	WrapperType any // Zero-value instance of the variant wrapper struct
}

// GetEnumVariants returns all registered variants for an enum type.
// Looks up EnumVariant attributes registered for the enum.
func GetEnumVariants(enumTarget string) []struct {
	Name      string
	ValueType any
} {
	registryMu.RLock()
	defer registryMu.RUnlock()

	var result []struct {
		Name      string
		ValueType any
	}

	if fields, ok := registry[enumTarget]; ok {
		for fieldName, attrs := range fields {
			for _, attr := range attrs {
				if ev, ok := attr.(EnumVariant); ok {
					result = append(result, struct {
						Name      string
						ValueType any
					}{
						Name:      fieldName,
						ValueType: ev.WrapperType,
					})
				}
			}
		}
	}
	return result
}
