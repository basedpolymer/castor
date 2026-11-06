# Castor MCP server

Castor ships an [MCP](https://modelcontextprotocol.io) server so that AI agents (Claude Desktop,
Claude Code, any MCP client) can use every Castor tool on your computer — compress a PDF,
make a QR code, rename a folder of photos — without uploading anything. It speaks MCP over
**stdio** and is built with the official Rust SDK (`rmcp`).

- Command: the Castor app binary with the argument `mcp` (`castor mcp`). The engines live in one
  binary; `castor-mcp` remains a development build target only.
- Same tools, options, runner and history as the app and `castor-cli`: a run made by an agent
  appears in the app's **Runs** page, and in-place runs (rename, dates, sort, duplicates moved
  aside, fixed extensions) can be undone there.
- Nothing leaves the machine, except what a tool does by nature when asked (the video
  downloader, HTML of a web address) and one-time model downloads, which follow the same
  consent rules as the app.

## Tools exposed to the agent

| MCP tool | arguments | returns |
|---|---|---|
| `list_tools` | `family?` (`pdf`, `image`, `video`, `audio`, `create`, `files`), `query?` (keywords, any language) | id, name, description, family, output formats, status, whether input files are needed, whether it edits files in place, why it is unavailable (if so) |
| `describe_tool` | `id` | description, accepted inputs, requirements, downloads still needed, and a **JSON Schema of the options** (types, ranges, choices with their labels, defaults, "used when …" conditions) |
| `run_tool` | `id`, `files?` (absolute paths; folders are expanded), `values?` (option values by key), `output_dir?` | run id, status, outputs, notes ("−69 %", "Version 3 · correction Q"…), errors |
| `run_chain` | `toml` (chain file text), `files?`, `output_dir?` | as `run_tool`, for the whole chain |
| `history` | `limit?` (default 20) | recent runs |

Without `output_dir`, results go next to the input files; tools that take no input (QR code,
gradients…) write to `Documents/Castor`. Existing files are never overwritten (`name (2).ext`).

Option values follow `describe_tool`'s schema: strings for choices, colours (`"#1b1616"`),
paths, sizes (`"1200x630"` or `[1200, 630]`); numbers for numeric options; booleans for
toggles. Unknown keys are rejected with the list of valid ones.

Example call (what the agent sends):

```json
{"name": "run_tool", "arguments": {
  "id": "create.qr-code",
  "values": {"content": "wifi", "ssid": "Atelier", "wifi_password": "s3cret", "card": "wifi", "format": "pdf"},
  "output_dir": "C:/Users/me/Desktop"
}}
```

## Setup

### Claude Desktop

Settings → Developer → Edit Config, then add Castor to `claude_desktop_config.json`:

Windows (`%APPDATA%\Claude\claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "castor": {
      "command": "C:\\Users\\YOU\\AppData\\Local\\Castor\\current\\castor.exe",
      "args": ["mcp"],
      "env": { "CASTOR_LANG": "en" }
    }
  }
}
```

macOS (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "castor": {
      "command": "/Applications/Castor.app/Contents/MacOS/castor",
      "args": ["mcp"],
      "env": { "CASTOR_LANG": "en" }
    }
  }
}
```

Restart Claude Desktop; "castor" appears in the tools menu.

### Claude Code

```sh
# Windows
claude mcp add castor -- "C:\Users\YOU\AppData\Local\Castor\current\castor.exe" mcp
# macOS
claude mcp add castor -- /Applications/Castor.app/Contents/MacOS/castor mcp
```

Add `--scope user` to make it available in every project. Check with `claude mcp list`.

### Other clients

Any MCP client that can launch a stdio server: command = the `castor` app binary, argument `mcp`.

### From a development checkout

```sh
CARGO_TARGET_DIR=target-e cargo build -p castor-mcp     # (Git Bash: `. crates/engine-pdf/env.sh` first)
claude mcp add castor-dev -- "$PWD/target-e/debug/castor-mcp.exe"
```

## Environment variables

| variable | effect |
|---|---|
| `CASTOR_LANG` | language of names, descriptions and messages sent to the agent (`en` by default for the MCP server; `fr`, `de`…) |
| `CASTOR_HOME` | portable profile: history, settings and downloads under this folder |
| `CASTOR_LOG` | log level on stderr (`info`, `debug`); stdout carries the protocol and stays clean |
| `CASTOR_FFMPEG`, `CASTOR_CHROME` | use another ffmpeg / Chromium-based browser |

## Notes for agents and users

- Long runs (video, OCR, transcription) block the call until they finish; the agent sees the
  result at the end. Cancelling from the client stops waiting, not yet the run itself.
- Tools that need a one-time download (OCR languages, whisper models, ML models, yt-dlp)
  download it on first use during the run. `describe_tool` lists what is still missing
  (`downloads_needed_with_defaults`) so the agent can ask you first.
- In-place tools (`in_place: true`) rename, move or re-date your files: they ask nothing.
  Ask your agent to preview first (e.g. run `files.rename` on a copy), or undo from the app.
- Passwords given in `values` are used for the run and never stored in history.
