<img src="https://raw.githubusercontent.com/halcyonnouveau/soppo/refs/heads/main/docs/assets/soppo.png" alt="soppo" style="max-width: 100%;">

# Soppo

A language that compiles to Go, adding ergonomic and type safety features that Go lacks. Soppo uses Go syntax wherever possible - if you know Go, you know most of Soppo.

**[Take the tour](https://tour.soppolang.dev)** · [Visit the playground](https://play.soppolang.dev) · [See the website](https://soppolang.dev)

## Why Soppo?

- **Enums & pattern matching:** Tagged unions with exhaustive matching
- **Nil safety:** Compile-time nil checks
- **Error handling:** `?` propagation with custom handling blocks
- **Rust-inspired diagnostics:** Helpful compiler error messages
- **Go interop:** Use any Go library directly
- **Batteries included:** LSP, formatter, linter, test runner with doctests

See [docs/guide.md](docs/guide.md) for the language guide.

## Installation

Install via [SOPMOD](https://github.com/halcyonnouveau/sopmod), the version manager for Soppo, written in Soppo:

```sh
curl -fsSL https://soppolang.dev/install.sh | sh
sopmod install sop latest
```

Or install manually with Cargo: `cargo install soppo`

## License

BSD 3-Clause. See [LICENSE](LICENSE).
