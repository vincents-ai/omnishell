<div align="center">

# 🐚 OmniShell

**An intelligent shell for humans and AI agents**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![shell](https://img.shields.io/badge/shell-posix-89e051)](https://en.wikipedia.org/wiki/POSIX)

[Features](#features) · [Install](#install) · [Quick Start](#quick-start) · [Configuration](#configuration) · [Agent Mode](#agent-mode) · [Contributing](#contributing)

</div>

---

OmniShell is a modern, secure, and intelligent shell built in pure Rust. It powers three very different use cases with a single binary:

- 🧒 **Kids mode** — a safe, sandboxed playground for children learning Linux
- 🤖 **Agent mode** — structured JSON I/O for AI agents and automation pipelines
- ⚡ **Admin mode** — a full-power POSIX shell for experienced users

## Why OmniShell?

**For parents and educators:** Give kids a real terminal experience without the risk. Kids mode uses a strict allowlist — only safe commands like `ls`, `cat`, `echo`, `cowsay` — with a built-in AI tutor that explains commands in age-appropriate language.

**For AI/ML engineers:** OmniShell's agent mode speaks JSON. Every command returns a structured envelope with exit code, stdout, stderr, and timing. Pipe commands together, check results programmatically, and let your AI agents operate a real shell safely behind a blocklist that prevents `sudo`, `rm -rf /`, and other destructive operations.

**For system administrators:** Admin mode is a full POSIX shell with pipes, redirections, if/while/for/case, command substitution, arithmetic, functions, and tab completion — backed by a modern Rust implementation with no C dependencies.

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
| Agent | Blocklist | `sudo`, `rm -rf /`, and dangerous flags are blocked |
| Admin | No restrictions | Full access |

Every command passes through the ACL engine *before* execution — in both interactive and non-interactive mode.

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

> **Note:** Sandboxing currently works on Linux only. macOS and Windows support is planned.

### Audit Logging

Every command execution is logged with:
- Timestamp, command, exit code
- ACL verdict (allowed/denied)
- Working directory, duration
- Mode at time of execution

Logs are stored per-mode in JSONL format under `$XDG_DATA_DIR/omnishell/audit/`.

### Mode-Separated History

Each mode maintains its own command history file:
- `~/.local/share/omnishell/history_kids.jsonl`
- `~/.local/share/omnishell/history_agent.jsonl`
- `~/.local/share/omnishell/history_admin.jsonl`

History entries include command, timestamp, exit code, and working directory.

## Install

### From Source (requires Nix)

```bash
git clone https://github.com/vincents-ai/omnishell.git
cd omnishell
nix develop --command bash -c "cargo build --release"
./target/release/omnishell
```

### Requirements

- Linux (macOS/Windows planned)
- [Nix](https://nixos.org/) (for reproducible builds)
- Rust 1.70+ (via Nix devShell)

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

OmniShell loads config from (later overrides earlier):

1. `/etc/omnishell/config.toml` — system-wide defaults
2. `~/.config/omnishell/config.toml` — user overrides
3. `--config path` — CLI override

Both TOML and JSON are supported.

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
api_key = ""  # Prefer OMNISHELL_LLM_API_KEY env var

[profile.agent]
mode = "agent"
```

See [docs/configuration.md](docs/configuration.md) for the full reference and [docs/examples/](docs/examples/) for more configs.

## Agent Mode

Agent mode is designed for AI agents and automation:

**Input:** Standard POSIX shell syntax.

**Output:** JSON envelope on stderr:
```json
{"type":"error","command":"ls","stdout":"","stderr":"...","exitCode":0,"durationMs":42}
```

**Error handling:** Blocked commands return exit code 126 with a structured message.

**Non-interactive usage:**
```bash
omnishell --mode agent -c "cargo build 2>&1"
echo $?  # exit code
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
| Linux | ✅ | ✅ | ✅ | ✅ |
| macOS | ✅ | ✅ | ✅ | 🔜 Planned |
| Windows | 🔜 | ✅ | ✅ | 🔜 Planned |

## License

OmniShell is dual-licensed under [AGPL-3.0-or-later](LICENSE) or a commercial license from Vincent Palmer. See the [LICENSE](LICENSE) file for details.

---

<div align="center">

Built with ❤️ by [vincents-ai](https://github.com/vincents-ai) using [shrs](https://github.com/MrPicklePinosaur/shrs), [gitoxide](https://github.com/Byron/gitoxide), and pure Rust.

</div>
