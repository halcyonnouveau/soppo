# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.2](https://github.com/halcyonnouveau/soppo/compare/v0.11.1...v0.11.2) - 2026-08-15

### Fixed

- fix deps

## [0.11.1](https://github.com/halcyonnouveau/soppo/compare/v0.11.0...v0.11.1) - 2026-08-10

### Fixed

- emit valid zero values for enum/interface/generic returns from `?`
- resolve embedded pointer fields from imported Go structs

### Other

- clippy fix

## [0.11.0](https://github.com/halcyonnouveau/soppo/compare/v0.10.1...v0.11.0) - 2026-04-27

### Added

- feat; add var block
- feat; add runtime benchmarks

### Fixed

- fix; minor optimisation on string interpolation

## [0.10.1](https://github.com/halcyonnouveau/soppo/compare/v0.10.0...v0.10.1) - 2026-01-28

### Added

- generate grouped imports

### Fixed

- handle CRLF line endings for Windows support
- multi-var assignment for nilable types

## [0.10.0](https://github.com/halcyonnouveau/soppo/compare/v0.9.0...v0.10.0) - 2025-12-27

### Added

- wrap some go commands
- attr for enums
- *(lsp)* show const fields
- add const struct fields
- *(attrs)* add enum values and type checking for attribute arguments
- *(tour)* add attributes
- add attribute system for compile-time validated metadata
- check const reassignment

### Fixed

- use go list for packages instead of modules
- soppo import root dir
- *(formatter)* attr formatting for struct fields/enum variants
- *(sniff)* sniff entire project
- multi-file packages
- external const inference
- tuple assign
- emit var for const bindings with runtime values

## [0.9.0](https://github.com/halcyonnouveau/soppo/compare/v0.8.1...v0.9.0) - 2025-12-22

### Added

- *(sniff)* try op lint check short circuit if
- add version marker to generated Go files
- *(lsp)* add sniff configuration support
- *(sniff)* add unreachable code rule
- *(sniff)* add shadow rule
- *(playground)* compress shareable base64 code
- helpful error when using switch instead of match
- missing return detection
- add sniff command to lint code

### Fixed

- improve error messages for module resolution failures
- auto-discover project for external Go module resolution
- qualified return type
- newlines in function calls

## [0.8.1](https://github.com/halcyonnouveau/soppo/compare/v0.8.0...v0.8.1) - 2025-12-21

### Added

- detect unused error variables in try blocks

### Fixed

- suppress cascading unused variable error for try captures

### Refactor

- *(tour)* reorder lessons by impact and consolidate content

## [0.8.0](https://github.com/halcyonnouveau/soppo/compare/v0.7.0...v0.8.0) - 2025-12-21

### Added

- improve interface satisfaction checking with detailed errors
- add format specifiers to string interpolation
- improve formatter and parser error messages
- compiler return multiple errors
- add positional struct literal support

### Fixed

- *(lsp)* show all diagnostics in workspace mode
- nilable test
- doctest attributes
- *(lsp)* hover/goto for receiver methods and Go param names
- doctests do what theyre supposed to
- error message for parse error
- codegen output tabs instead of spaces

### Refactor

- split Deref out of Unary AST
- split Field AST into Field, PackageMember, EnumVariant
- split Call AST into Call, TypeConversion, and TypeInst
- move to typed AST to produce a `TypedFile` directly, then zonk

## [0.7.0](https://github.com/halcyonnouveau/soppo/compare/v0.6.0...v0.7.0) - 2025-12-19

### Added

- *(lsp)* add rename
- *(lsp)* add Go doc comments and package name hover/goto
- add tour and other fixes

### Changed

- *(lsp)* rename binary to sopls

### Fixed

- *(lsp)* function hovering
- *(editor)* tree-sitter grammar

## [0.6.0](https://github.com/halcyonnouveau/soppo/compare/v0.5.0...v0.6.0) - 2025-12-18

### Added

- add top level var decl

### Fixed

- formatter issues
- resolve cross-module imports in sop check

## [0.5.0](https://github.com/halcyonnouveau/soppo/compare/v0.4.1...v0.5.0) - 2025-12-18

### Added

- add unused import/variable detection and improve Go semantics
- bye bye import shadowing
- support method calls on types from imported Soppo modules
- nil narrowing in match and match divergence analysis
- add raw string literal support and stdlib return type tests
- infer types for Go package variables from composite literals
- support `return f()` for multi-value returns
- narrow error companions after early return check
- support Go type aliases with numeric underlying types
- allow `?` in non-error-returning functions
- add spread operator
- better integer support
- support grouped named return parameters
- add sop.mod version checking
- add formatter
- add test runner and doctests
- add sop.mod config file and glob pattern support
- add benchmarks
- add fuzzing and proptest infrastructure

### Fixed

- correctly distinguish soppo enums from Go constants in match
- variadic spread
- field indexing and nested composite literal support
- attach module info to Go package variable types
- external modules
- allow `?` with handler in non-error-returning functions
- nil in match
- ansi escape codes
- off-by-one error in block comment parsing
- top level comment gen

## [0.4.1](https://github.com/halcyonnouveau/soppo/compare/v0.4.0...v0.4.1) - 2025-12-09

### Fixed

- named argument slot filling
- validate type arguments are real types

## [0.4.0](https://github.com/halcyonnouveau/soppo/compare/v0.3.1...v0.4.0) - 2025-11-30

### Fixed

- *(lsp)* better span precision

### Refactor

- *(lsp)* ident for better spans

## [0.3.1](https://github.com/halcyonnouveau/soppo/compare/v0.3.0...v0.3.1) - 2025-11-30

### Fixed

- dockerfile

## [0.3.0](https://github.com/halcyonnouveau/soppo/compare/v0.2.0...v0.3.0) - 2025-11-30

### Added

- *(lsp)* add completions
- *(lsp)* add hover support to show type information
- *(tree-sitter)* add string interpolation support
- *(lsp)* add tests and fix diagnostic span extraction
- add tree-sitter grammar and zed extension
- add generic constraint validation
- add || operator nil narrowing and fix type assertion semantics
- generic function instantiation and compound nil narrowing

### Fixed

- *(lsp)* workspace diagnostics
- correct error span for string interpolation expressions
- zed extension config
- add non_exhaustive to error enum

### Other

- update
- workspace support foundations for LSP
- add zed lsp

### Refactor

- unify container type helpers and nil safety checks

## [0.2.0](https://github.com/halcyonnouveau/soppo/compare/v0.1.1...v0.2.0) - 2025-11-29

### Added

- catch nil in func args, slices, maps, and channel sends
- support explicit type args for generic unit enum variants
- add playground and bug fixes
- interface satisfaction, local types, and anonymous structs
- more go features
- support function types and passing function references as arguments
- dynamic interface type lookup for Go packages
- add support for methods on enum variants
- add struct field tags

### Fixed

- go type extraction and ? operator multi-return handling

### Other

- fix indents

## [0.1.1](https://github.com/halcyonnouveau/soppo/compare/v0.1.0...v0.1.1) - 2025-11-29

### Fixed

- track nil state for all nilable types, not just pointers
