# Editor Support

Soppo provides editor integration through the Language Server Protocol (LSP) and Tree-sitter for syntax highlighting. Currently, only Zed is officially supported.

> [!NOTE]
> Additional editor integrations will not be added until Soppo reaches a stable release.

## Zed

1. Clone the Soppo repository
2. In Zed, open the command palette and run `zed: install dev extension`
3. Select the `editors/zed` directory from the cloned repository

The extension provides syntax highlighting via Tree-sitter and will automatically download the LSP binary when you first open a `.sop` file.

See [Zed's documentation](https://zed.dev/docs/extensions/installing-extensions) for more details on dev extensions.

## Other Editors

If you want to configure another editor:

- **Syntax highlighting:** The Tree-sitter grammar is located in `editors/tree-sitter`
- **LSP:** The binary can be downloaded from [GitHub Releases](https://github.com/halcyonnouveau/soppo/releases)
