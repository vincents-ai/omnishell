<div align="center">

# 🐚 OmniShell

**An intelligent shell for humans and AI agents**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![shell](https://img.shields.io/badge/shell-posix-89e051)](https://en.wikipedia.org/wiki/POSIX)

[Features](#features) · [Install](#install) · [Quick Start](#quick-start) · [Configuration](#configuration) · [Agent Mode](#agent-mode) · [Contributing](#contributing)

</div>

---

OmniShell is a modern, secure, and intelligent shell written in Rust. It powers three very different use cases with a single binary:

- 🧒 **Kids mode** — a safe, sandboxed playground for children learning Linux
- 🤖 **Agent mode** — structured JSON I/O for AI agents and automation pipelines
- ⚡ **Admin mode** — a full-power POSIX shell for experienced users

## Why OmniShell?

**For parents and educators:** Give kids a real terminal experience without the risk. Kids mode uses a strict allowlist — only safe commands like `ls`, `cat`, `echo`, `cowsay` — with a built-in AI tutor that explains commands in age-appropriate language.

**For AI/ML engineers:** OmniShell's agent mode speaks JSON. Every command returns a structured envelope with exit code, stdout, stderr, and timing. Pipe commands together, check results programmatically, and let your AI agents operate a real shell safely behind a blocklist that prevents `sudo`, `rm -rf /`, and other destructive operations.

**For system administrators:** Admin mode is a full POSIX shell with pipes, redirections, if/while/for/case, command substitution, arithmetic, functions, and tab completion — backed by a modern Rust implementation. The shell core is Rust, but the complete dependency tree is not C-free: `Cargo.lock` includes `openssl-sys` and `ring` through the HTTP/TLS/LLM dependencies.

## Features

### Shell Language

Full POSIX shell scripting that works the way you expect:

```bash
# Pipes
echo hello | tr a-z A-Z

# Variables and substitution
name=$(whoami)
echo "Hello, $name"

# Conditionals
if [ -f /etc/hostname ]; then
    echo "Hostname file exists"
fi

# Loops
for file in *.txt; do
    echo "Found: $file"
done

# Arithmetic
echo "Total: $((count + 1))"

# Functions
greet() { echo "Hello, $1!"; }
greet world

# Case with glob patterns
case $ext in
    *.txt) echo "Text file" ;;
    *.rs)  echo "Rust source" ;;
esac
```

### Security

| Mode | Strategy | Enforced By |
|------|----------|-------------|
| Kids | Strict allowlist | Only explicitly permitted commands run |
| Agent | Blocklist | `sudo`, `su`, `mkfs*`, `dd`, and deletion of `/` are blocked |
| Admin | No restrictions | Full access |

**Security model.** Kids mode is the only mode with a real containment boundary: an allowlist plus the Linux namespace sandbox described below. Agent mode is a *blocklist*, not a sandbox — it stops a known set of dangerous commands, but anything not on the blocklist runs with your user's full privileges. Admin mode is deliberately unrestricted. Every command passes through the ACL engine *before* execution, in both interactive and non-interactive mode.

### LLM Integration

Built-in AI assistant accessible from the prompt:

```
admin$ ? how do I find large files
```

Each mode gets a different AI personality:
- **Kids:** Patient, encouraging tutor with age-appropriate explanations
- **Agent:** Precise, structured responses optimized for programmatic use
- **Admin:** Concise, technical answers

Works with OpenAI, Anthropic, Ollama, or any OpenAI-compatible API. See [Configuration](#configuration).

### Sandboxing (Linux)

Kids mode runs commands in an isolated Linux namespace sandbox:
- Separate mount namespace (read-only system dirs)
- Separate PID namespace (process limits)
- Separate network namespace (network disabled)
- File size and process count resource limits

> **Note:** Sandboxing works on Linux only. There is no macOS or Windows sandbox implementation.

### Audit Logging

Every command execution is logged with:
- Timestamp, command, exit code
- ACL verdict (allowed/denied)
- Working directory, duration
- Mode at time of execution

Logs are stored per-mode in JSONL format under the local data directory, by default `$XDG_DATA_HOME/omnishell/audit/` (normally `~/.local/share/omnishell/audit/`).

### Mode-Separated History

Each mode maintains its own command history file:
- `~/.local/share/omnishell/history_kids.jsonl`
- `~/.local/share/omnishell/history_agent.jsonl`
- `~/.local/share/omnishell/history_admin.jsonl`

History entries include command, timestamp, exit code, and working directory.

## Install

There is no package-manager formula or crates.io publish step in this repository. The supported install paths are source builds through the Nix devShell and the prebuilt tarballs attached to GitHub Releases.

### From Source (Nix)

```bash
git clone https://github.com/vincents-ai/omnishell.git
cd omnishell
nix develop --command bash -c "cargo build --release"
./target/release/omnishell
```

### Prebuilt binaries

`.github/workflows/release.yml` builds and attaches these tarballs to tagged GitHub Releases:

- `omnishell-x86_64-linux.tar.gz`
- `omnishell-aarch64-linux.tar.gz`
- `omnishell-aarch64-macos.tar.gz`

There is no native Windows build; the release notes point Windows users at the Linux x86_64 binary under WSL.

### Requirements

- Linux or macOS (see [Platform Support](#platform-support))
- [Nix](https://nixos.org/) for the reproducible devShell
- Rust 1.95 via the flake's devShell (the lockfile needs at least Rust 1.89)

## Quick Start

```bash
# Interactive shell (default: admin mode)
omnishell

# Kids mode (safe for children)
omnishell --mode kids

# Agent mode (for AI pipelines)
omnishell --mode agent

# Run a single command
omnishell -c "echo hello | tr a-z A-Z"

# With a specific profile
omnishell --profile kids

# Disable AI features
omnishell --no-llm
```

### Interactive Built-ins

| Command | Description |
|---------|-------------|
| `?` / `ai <prompt>` | Ask the AI assistant |
| `help` | Show available commands |
| `mode` | Show current mode |
| `mode kids` | Switch to kids mode |
| `snapshots` | List command snapshots |
| `undo` / `redo` | Undo/redo last command |
| `exit` | Exit the shell |

## Configuration

OmniShell loads config from:

1. `~/.config/omnishell/config.toml` (or `.json`) — user config
2. `/etc/omnishell/config.toml` (or `.json`) — system config, which overrides user profiles with the same name
3. `--config path` — replaces auto-discovery entirely

If both `config.json` and `config.toml` exist in the same directory, JSON wins. Both TOML and JSON are supported.

### Environment Variables

These are the environment variables OmniShell reads itself:

| Variable | Used for |
|----------|----------|
| `USER` | Profile auto-binding, prompt, audit `user`, engram agent identity |
| `HOME` | `~` expansion and the default Kids sandbox location |
| `HOSTNAME` / `HOST` | Prompt hostname |
| `PATH` | External command lookup and tab completion |
| `ENGram_AGENT` | Engram agent identity fallback when `USER` is unset |
| `_OMNISHELL_PRE_SNAPSHOT` | Internal flag set during single-command snapshot handling |
| `_OMNISHELL_ALIAS_<name>` | Internal storage for the `alias` builtin |

XDG locations come from the `dirs` crate (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, falling back to `~/.config` and `~/.local/share` on Linux). LLM provider credentials are passed through to `vincents-llm`; this repository does not itself read `OMNISHELL_LLM_API_KEY`.

### Example: Kids profile with local Ollama

```toml
default_profile = "kids"

[llm]
provider = "ollama"
model = "llama3"
api_base = "http://localhost:11434"
temperature = 0.3
max_tokens = 256

[profile.kids]
mode = "kids"
username = "child"
display_name = "Kids Mode"
age = 7

[profile.agent]
mode = "agent"

[profile.admin]
mode = "admin"
```

### Example: Agent mode with OpenAI

```toml
default_profile = "agent"

[llm]
provider = "openai"
model = "gpt-4o"
api_key = ""  # Provider API key; see docs/configuration.md

[profile.agent]
mode = "agent"
```

See [docs/configuration.md](docs/configuration.md) for the full reference and [docs/examples/](docs/examples/) for more configs.

## Agent Mode

Agent mode is designed for AI agents and automation:

**Input:** Standard POSIX shell syntax, either one `-c` command or one interactive line at a time.

**Output contract:** exactly one JSON object per command on **stdout**, one object per line. Anything that is not the envelope — startup banner, prompt, progress, diagnostics — stays on **stderr**. Interactive agent mode prints the envelope after each command too.

```json
{"type":"success","command":"echo hi","stdout":"hi\n","stderr":"","exitCode":0,"durationMs":2}
```

`type` is `success` when `exitCode == 0`, otherwise `error`. `durationMs` measures the command execution. The full field schema and more examples are in [docs/agent-mode.md](docs/agent-mode.md).

**Error handling:** Blocked commands exit 126, and the envelope's `stderr` field names the reason:

```json
{"type":"error","command":"rm -rf /","stdout":"","stderr":"omnishell: Command 'rm' blocked: Deletion of the filesystem root blocked (target: /)","exitCode":126,"durationMs":0}
```

**Exit codes:**

| Code | Meaning |
|------|---------|
| `0` | Command succeeded |
| `1` | Command ran and failed (general failure) |
| `126` | Blocked by the ACL before execution |
| `127` | Command not found |

**Non-interactive usage:**
```bash
omnishell --mode agent -c "cargo build 2>&1"
echo $?  # exit code

# Disable the LLM for this session
omnishell --mode agent --no-llm
```

### Using OmniShell as a Library

```rust
use omnishell::{OmniShellConfig, AclEngine, Mode, Verdict};

let acl = AclEngine::new(Mode::Agent);
match acl.evaluate("sudo rm -rf /") {
    Verdict::Deny(reason) => println!("Blocked: {}", reason),
    Verdict::Allow => println!("Allowed"),
}
```

## Architecture

```
omnishell binary
├── OmniShellLang (POSIX shell evaluator)
│   ├── Pipes, if/while/for/case, &&, ||
│   ├── $(cmd) and $((expr)) expansion
│   ├── break/continue, test/[ builtin
│   ├── Function definitions
│   └── ACL enforcement per-mode
├── shrs (readline, prompt, keybindings)
├── CompletionEngine (mode-aware tab completion)
├── History (mode-separated JSONL persistence)
├── SnapshotEngine (git-based undo via gitoxide)
├── AuditLogger (JSONL audit trail)
├── Sandbox (Linux namespace isolation)
└── LLM Integration (OpenAI/Anthropic/Ollama/Custom)
```

## Platform Support

| Platform | Shell | ACL | LLM | Sandbox |
|----------|-------|-----|-----|---------|
| Linux | ✅ | ✅ | ✅ | ✅ (Linux namespaces; degrades with a warning without privileges) |
| macOS | ✅ (aarch64 release build) | ✅ | ✅ | ❌ Linux-only module |
| Windows | ❌ (Unix APIs, no build) | ❌ | ❌ | ❌ |

Windows is not a supported target. The release workflow publishes no Windows artifact and tells users to run the Linux binary under WSL.

## Contributing

Build and run the test suite through the Nix devShell:

```bash
nix develop --command bash -c "cargo build"
nix develop --command bash -c "cargo test"
```

Integration tests that exercise the binary discover it through `OMNISHELL_BIN` (default `./target/debug/omnishell`), so run `cargo build` first.

Report bugs and feature requests at <https://github.com/vincents-ai/omnishell/issues>. The repository uses [AGPL-3.0-or-later](LICENSE) as its contribution license.

## License

OmniShell is dual-licensed under [AGPL-3.0-or-later](LICENSE) or a commercial license from Vincent Palmer. See the [LICENSE](LICENSE) file for details.

---

<div align="center">

Built with ❤️ by [vincents-ai](https://github.com/vincents-ai) using [shrs](https://github.com/MrPicklePinosaur/shrs), [gitoxide](https://github.com/Byron/gitoxide), and Rust.

</div>
