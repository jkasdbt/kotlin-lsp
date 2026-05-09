# Editor setup

`kotlin-lsp` is at `~/.cargo/bin/kotlin-lsp` after `cargo install`. Run `which kotlin-lsp` to confirm it's on your `PATH`.

## VS Code

![VS Code with kotlin-lsp](../demo/vscode.png)

Download the `.vsix` for your platform from the [latest release](https://github.com/Hessesian/kotlin-lsp/releases/latest) and install it:

```bash
# Linux x86_64
code --install-extension kotlin-lsp-linux-x64-vX.Y.Z.vsix

# macOS Apple Silicon
code --install-extension kotlin-lsp-darwin-arm64-vX.Y.Z.vsix

# macOS Intel
code --install-extension kotlin-lsp-darwin-x64-vX.Y.Z.vsix
```

Or install the universal `.vsix` (no bundled binary — `kotlin-lsp` must be on your `PATH`):

```bash
code --install-extension kotlin-lsp-vX.Y.Z.vsix
```

The extension activates automatically for `.kt`, `.java`, and `.swift` files — no other Kotlin plugins needed.

> **Tip:** Disable other Kotlin extensions (`fwcd.kotlin`, `jetbrains.kotlin`) to avoid conflicts.

**Configuration** (optional) — in `.vscode/settings.json`:

```json
{
  "kotlinLsp.path": "/path/to/kotlin-lsp"
}
```

Default: `kotlin-lsp` on `$PATH`.

**Install from source** (if you prefer to build locally):

```bash
cd contrib/vscode && npm install
ln -s "$(pwd)/contrib/vscode" ~/.vscode/extensions/kotlin-lsp.kotlin-lsp-client-0.0.1
```

## Helix

Add to `~/.config/helix/languages.toml`:

```toml
[[language]]
name = "kotlin"
language-servers = ["kotlin-lsp"]
auto-format = false

[[language]]
name = "java"
language-servers = ["kotlin-lsp"]
auto-format = false

[[language]]
name = "swift"
language-servers = ["kotlin-lsp"]
auto-format = false

[language-server.kotlin-lsp]
command = "kotlin-lsp"
```

Restart Helix (or run `:lsp-restart`). Check the server is running: `:lsp-workspace-command` or watch `:log-open`.

## Neovim (nvim-lspconfig)

```lua
local lspconfig = require('lspconfig')
local configs   = require('lspconfig.configs')

if not configs.kotlin_lsp then
  configs.kotlin_lsp = {
    default_config = {
      cmd       = { 'kotlin-lsp' },
      filetypes = { 'kotlin', 'java', 'swift' },
      root_dir  = lspconfig.util.root_pattern(
        'build.gradle', 'build.gradle.kts', 'pom.xml', 'settings.gradle', 'Package.swift', '.git'
      ),
      settings  = {},
    },
  }
end

lspconfig.kotlin_lsp.setup {}
```

Place this in your `init.lua` (or a dedicated `after/ftplugin/kotlin.lua`).

**Completion** — pair with [nvim-cmp](https://github.com/hrsh7th/nvim-cmp):

```lua
require('cmp').setup {
  sources = {
    { name = 'nvim_lsp' },
    -- other sources …
  },
}
```

## Zed

Add to your Zed settings (`~/.config/zed/settings.json` or `.zed/settings.json` per-project):

```json
{
  "languages": {
    "Kotlin": {
      "language_servers": ["kotlin-lsp"]
    },
    "Java": {
      "language_servers": ["kotlin-lsp"]
    },
    "Swift": {
      "language_servers": ["kotlin-lsp"]
    }
  },
  "lsp": {
    "kotlin-lsp": {
      "binary": {
        "path": "kotlin-lsp",
        "arguments": ["--stdio"]
      }
    }
  }
}
```

> **Note:** Zed requires a restart (not just workspace reload) after changing LSP config. If Zed doesn't recognise Kotlin files, you may need the [Kotlin extension](https://zed.dev/extensions?query=kotlin) installed.
