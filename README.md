# Terraform Link Docs (Zed)

Makes Terraform resource types, data sources, and module `source` strings
**clickable** in Zed, linking straight to their documentation on
`registry.terraform.io`.

A port of the VSCode extension
[`vscode-terraform-link-docs`](https://github.com/tdharris/vscode-terraform-link-docs).

## Architecture

Zed has no editor API for decorating arbitrary text with links. The only
mechanism that produces clickable links inside a document is the LSP request
`textDocument/documentLink`. So this project is two pieces:

1. **`terraform-link-docs-lsp`** (`server/`) — a small standalone language server
   that implements `textDocument/documentLink` for Terraform/HCL files and
   returns links to `registry.terraform.io`. All the real logic lives here.
2. **The Zed extension** (this repo root, WASM) — a thin wrapper that registers
   that language server against the `Terraform` and `HCL` languages and tells Zed
   how to launch the binary.

**Requires the official [Terraform](https://github.com/zed-extensions/terraform)
extension** to be installed in Zed — it provides the Terraform/HCL language
definitions, grammar, and `terraform-ls`. This extension attaches an additional
language server to those languages and only contributes document links; the two
run side by side.

## Local dev / test

1. Install the server on your `PATH`:
   ```sh
   cargo install --path server
   ```
   This drops `terraform-link-docs-lsp` into `~/.cargo/bin`.
2. Ensure the official **Terraform** extension is installed in Zed.
3. In Zed: **Extensions → Install Dev Extension** → select this repo root.
4. Open a `.tf` file. Resource types, data sources, and module `source` strings
   render as clickable links — click (or Cmd/Ctrl-click) to open the docs.

### Troubleshooting

- If links don't appear, check the logs by launching Zed with
  `zed --foreground`.
- Confirm the `lsp_document_links` editor setting is enabled (it is on by
  default).
- If the extension reports it can't find the server, make sure
  `terraform-link-docs-lsp` is on your `PATH` (re-run `cargo install --path server`).

## Scope

v1 is hardcoded to `registry.terraform.io` with no configuration. Later
iterations will add the VSCode extension's settings (community providers,
alternate documentation registries, custom URL templates).
