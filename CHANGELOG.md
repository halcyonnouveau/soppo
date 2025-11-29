# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
