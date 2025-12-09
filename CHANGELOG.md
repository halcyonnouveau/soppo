# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
