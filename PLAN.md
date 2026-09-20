# Terraform Link Docs — Zed Extension

Port of the VSCode extension [`vscode-terraform-link-docs`](https://github.com/tdharris/vscode-terraform-link-docs)
to Zed. It makes Terraform resource types, data sources, and module `source`
strings clickable, linking to their documentation on `registry.terraform.io`.

## The core architectural constraint

VSCode does this with a `DocumentLinkProvider` — an editor API that lets an
extension attach clickable links to arbitrary text ranges. **Zed has no such
extension API.** Zed extensions can only provide: languages, grammars, language
servers, debuggers, themes, icon themes, snippets, and MCP servers. There is no
way for extension WASM code to decorate the editor directly.

The **only** mechanism in Zed that produces clickable links inside a document is
the LSP request [`textDocument/documentLink`](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_documentLink).
Zed implements this on the client side (`crates/editor/src/document_links.rs`,
`crates/project/src/lsp_store/document_links.rs`), gated by the editor setting
`lsp_document_links` (default: on). Links are rendered as clickable and open in
the browser / editor.

**Therefore this project is two components:**

1. **`terraform-link-docs-lsp`** — a small standalone language server (native
   Rust binary) that implements `textDocument/documentLink` for Terraform/HCL
   files and returns links to `registry.terraform.io`. This is where all the
   real logic lives (ported from the VSCode extension's `src/terraform/*.ts`).

2. **The Zed extension** (WASM, at repo root) — a thin wrapper whose only job is
   to register that language server against the Terraform/HCL languages and tell
   Zed how to launch the binary.

### Why cross-extension language attachment works

The Terraform, `Terraform Vars`, and HCL languages are **not** built into Zed —
they come from the official [`zed-extensions/terraform`](https://github.com/zed-extensions/terraform)
extension. Our extension does **not** redefine them (that would conflict).
Instead it attaches an additional language server to those language *names*.

Precedent: the [`harper`](https://github.com/zed-extensions/harper) extension
attaches its `harper-ls` server to dozens of languages (C, Go, HTML, …) that it
does not define itself. Zed matches `languages = [...]` by name across all
installed extensions at runtime. Multiple language servers per language are
supported and their `documentLink` results are merged.

**Dependency:** the user must have the official **Terraform** extension
installed (for the language definitions + grammar + terraform-ls). Our server
runs alongside terraform-ls and only contributes document links.

## Repository layout

```
zed-terraform-link-docs/
  extension.toml            # Zed extension manifest (registers the LSP)
  Cargo.toml                # WASM extension crate (cdylib, zed_extension_api)
  src/
    lib.rs                  # implements language_server_command
  server/                   # standalone LSP server (its own independent crate)
    Cargo.toml
    src/
      main.rs               # JSON-RPC/stdio loop, initialize, documentLink
      links.rs              # resource/data + module URL logic (ported)
      providers.rs          # required_providers parsing (ported)
    tests/ or #[cfg(test)]  # unit tests for URL generation
  README.md
  PLAN.md
```

**Important:** do **not** put a `[workspace]` at the repo root that includes
`server/`. The root crate compiles to `wasm32-wasip2`; the server compiles to
the native host target. Keep them as two independent crates so `cargo build` at
root (wasm) never tries to build the native server and vice-versa.

## Component 1 — `server/` (the language server)

### Crate

- `[[bin]]` named `terraform-link-docs-lsp`.
- Dependencies (latest as of writing):
  - `lsp-server = "0.10"` (sync stdio JSON-RPC transport, from rust-analyzer)
  - `lsp-types = "0.97"` (LSP 3.17 types)
  - `serde_json = "1"`
  - `serde = { version = "1", features = ["derive"] }` (if needed)
- Edition 2021. No async runtime needed — `lsp-server` is synchronous.

### LSP behavior

- **initialize** → respond with capabilities:
  - `text_document_sync`: `Full` (so we always have current buffer text).
  - `document_link_provider`: `Some(DocumentLinkOptions { resolve_provider: Some(false), work_done_progress_options: default })`.
  - Capture `root_uri` / `workspace_folders` if present (used as a hint; primary
    file resolution is via each document's own URI).
- **initialized / shutdown / exit**: handle gracefully (standard `lsp-server`
  `Connection::initialize` + main loop; exit cleanly on shutdown).
- **textDocument/didOpen, didChange, didClose**: maintain an in-memory
  `HashMap<Url, String>` of open document text. (`Full` sync → each didChange
  carries the whole text.)
- **textDocument/documentLink**: compute and return `Vec<DocumentLink>` for the
  requested document (see logic below).
- Every other request/notification: no-op / empty result. Never panic; log
  errors to stderr (Zed forwards stderr to its log).

### Positions

LSP positions are 0-based `(line, character)` where `character` is a **UTF-16**
code-unit offset. Terraform identifiers and source strings are ASCII in
practice, so char count == UTF-16 units, but compute offsets per line correctly
(operate on the line string; if you want to be exact, count UTF-16 units of the
substring before the range start). Ranges are computed exactly like the VSCode
port: on the matched line, from `prefix.len()` to `prefix.len() + match.len()`.

### Link logic — port of `src/terraform/*.ts`, hardcoded to `registry.terraform.io`

Scan the document **line by line**. For each line try the module matcher first,
then the resource matcher (mirrors `moduleLineMatcher(line) || resourceLineMatcher(line)`).

#### Resource / data sources (port `resource.ts`)

- Line regex: `^(?P<prefix>\s*(?P<kind>resource|data)\s+")(?P<rtype>[^"]+)"`
  - Link range = `prefix.len()` .. `prefix.len() + kind..rtype` match length
    (i.e. the span covering `resource "` open-quote position through the end of
    the resource type, same as VSCode: prefix length to full match length).
  - **Note:** match parity with VSCode — the linked range starts right after the
    opening `"` and covers the resource type text.
- Split `rtype` with `^(?P<provider>[^_]+)_(?P<name>.*)$`. If no match, skip.
- `type_segment` = `data-sources` if kind==`data` else `resources`.
- **If the provider alias is in the provider map** (from `required_providers`,
  value `namespace/providerName`):
  `https://registry.terraform.io/providers/{namespace}/{providerName}/latest/docs/{type_segment}/{name}`
- **Fallback (provider not in map)** — assume the `hashicorp` namespace (this is
  an intentional improvement over the VSCode legacy `www.terraform.io` fallback,
  and matches the common case for first-party providers):
  `https://registry.terraform.io/providers/hashicorp/{provider}/latest/docs/{type_segment}/{name}`

#### Modules (port `module.ts`)

- Line regex: `^(?P<prefix>\s*source\s+=\s+")(?P<src>[^"]+)"`; range = `prefix.len()` .. end of `src`.
- Dispatch on `src`:
  - contains `http://` or `https://` → link target = `src` as-is.
  - starts with `.` (local path) → `file://` URI of the path joined to the
    document's directory (best-effort; opens the folder/file in Zed if
    supported). Resolve relative to the document's own file path.
  - starts with `git::`, `github.com`, or `git@github.com` → **generic git**:
    parse with
    `^(?:git::(?:ssh://)?(([^@]+@)?))?(?:git::https?://)?(?:git@)?(?P<domain>[^:/]+)(?::|/)?(?P<repoPath>[^?#.]+)(?:\.git)?(?://(?P<module>[^?#]+))?(?:\?ref=(?P<ref>[^#]+))?(?:#.*)?$`
    then `repoPath` = strip everything from first `.git`; URL =
    `https://{domain||github.com}/{repoPath}/tree/{ref||HEAD}/{module}`.
  - starts with `gcs::`, `s3::`, or `hg::` → skip (no link).
  - otherwise → **Terraform registry module**:
    - If the first `/`-segment contains a `.` → private registry:
      `https://{src}`.
    - Else split on `/` (drop empty parts) into
      `namespace/name/provider[/...submodules]`:
      - submodules present → join them with `/`, strip a leading `modules/`,
        URL = `https://registry.terraform.io/modules/{namespace}/{name}/{provider}/latest/submodules/{submodule}`.
      - else if namespace & name & provider all present →
        `https://registry.terraform.io/modules/{namespace}/{name}/{provider}`.
      - else skip.

#### Provider map (port `provider.ts`)

Build `HashMap<alias, "namespace/providerName">` by parsing `required_providers`
blocks. The parser must:

- Find each `required_providers` keyword, then the next `{`, allowing only
  whitespace or `=` between keyword and brace (`required_providers {` and
  `required_providers = {`).
- Balance braces to isolate the block, while being string-aware (skip `{`/`}`
  inside `"`/`'` strings, honoring `\` escapes) and skipping `#` and `//`
  line comments.
- Inside the block, match providers with
  `(?P<alias>[\w-]+)\s*=\s*\{[\s\S]*?source\s*=\s*"(?P<source>[^"]+)"`.

Source files to scan (native fs — the server is a normal process and can read
the workspace): the document's own directory's `versions.tf`, `main.tf`,
`providers.tf`, **plus the current document itself**. Process the current
document **last** using the live buffer text (from the in-memory map) so it
overrides on-disk definitions. For sibling files, read from disk; ignore missing
files. (A per-directory cache like the VSCode version is optional for v1 —
correctness first; recomputing each `documentLink` request is acceptable.)

### Tests

Add `#[cfg(test)]` unit tests covering, at minimum:
- resource in provider map → community URL (e.g. `cloudflare/cloudflare` →
  `.../providers/cloudflare/cloudflare/latest/docs/resources/dns_record`).
- resource not in map → hashicorp fallback URL.
- data source → `data-sources` segment.
- registry module (`terraform-aws-modules/vpc/aws`) and submodule.
- private registry module (`app.terraform.io/...`).
- github / generic-git module sources.
- `required_providers` parsing incl. `=` form and comments.

Use the `examples/*.tf` from the VSCode repo as fixtures
(`/Users/owen/Documents/git/misc/vscode-terraform-link-docs/examples/`) — copy
them into `server/tests/fixtures/` or inline the snippets.

### Build/verify

- `cargo build --release` inside `server/` must succeed.
- `cargo test` inside `server/` must pass.

## Component 2 — root extension (WASM wrapper)

### `extension.toml`

```toml
id = "terraform-link-docs"
name = "Terraform Link Docs"
version = "0.0.1"
schema_version = 1
authors = ["..."]
description = "Clickable links from Terraform resources, data sources, and module sources to their registry.terraform.io documentation."
repository = "https://github.com/laluowen/zed-terraform-link-docs"

[language_servers.terraform-link-docs]
name = "Terraform Link Docs"
languages = ["Terraform", "HCL"]
language_ids = { "Terraform" = "terraform", "HCL" = "hcl" }
```

(We attach to `Terraform` and `HCL`. Not `Terraform Vars` — `.tfvars` files
contain no resource/module blocks.)

### `Cargo.toml`

```toml
[package]
name = "terraform-link-docs"
version = "0.0.1"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
zed_extension_api = "0.6.0"
```

(Use `0.6.0`; bump only if the local Zed requires a newer API. The functions we
need — `worktree.which`, `latest_github_release`, `download_file`,
`make_file_executable`, `set_language_server_installation_status` — exist in
this line.)

### `src/lib.rs`

Implement `zed::Extension` with `language_server_command`. Binary resolution
order:

1. `worktree.which("terraform-link-docs-lsp")` — found on `PATH`. **This is the
   primary path for v1 / local dev**: the user runs
   `cargo install --path server` which drops the binary into `~/.cargo/bin`
   (typically on `PATH`).
2. A cached previously-downloaded path (`cached_binary_path`), if it still
   exists.
3. **(Scaffold for later)** download from GitHub releases via
   `zed::latest_github_release("laluowen/zed-terraform-link-docs", ...)` +
   `zed::download_file(...)`, mirroring the structure of
   `zed-extensions/terraform`'s `src/terraform.rs`. Since no releases exist yet,
   this path will error at runtime; that's fine for v1. If it's simpler and
   cleaner, v1 may implement **only** steps 1–2 and return a clear, actionable
   error ("Install the server with `cargo install --path server` …") when the
   binary isn't found — the download path can be added when we cut releases.

The launch command is just the binary with no args and empty env
(`Command { command, args: vec![], env: vec![] }`). The server talks LSP over
stdio.

### Build/verify

- `cargo build --release --target wasm32-wasip2` at repo root must succeed.

## Local dev / test flow (document in README)

1. `cargo install --path server` (puts `terraform-link-docs-lsp` on `PATH`).
2. Ensure the official **Terraform** extension is installed in Zed.
3. Zed → Extensions → **Install Dev Extension** → select this repo root.
4. Open a `.tf` file (e.g. copy the `examples/`); resource types and module
   sources render as clickable links. Ctrl/Cmd-click (or click) opens the docs.
5. If links don't appear: check `zed --foreground` logs; confirm the
   `lsp_document_links` editor setting is enabled.

## v1 scope (explicitly out of scope for now)

Hardcoded to `registry.terraform.io`. **No configuration** yet. Later iterations
add the VSCode extension's settings: `enableCommunityProviders`,
`documentationRegistry` (opentofu / library.tf / custom), and the custom URL
templates. These map cleanly onto Zed's per-language-server settings
(`lsp.terraform-link-docs.settings`) read from `initialization_options` /
`workspace/didChangeConfiguration` in the server — a later task.

## Reference source (VSCode extension)

`/Users/owen/Documents/git/misc/vscode-terraform-link-docs/src/`:
- `extension.ts` — DocumentLinkProvider wiring (the thing Zed replaces with LSP).
- `terraform/resource.ts` — resource/data URL logic.
- `terraform/module.ts` — module source URL logic.
- `terraform/provider.ts` — `required_providers` parsing.
