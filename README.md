# OmniShell

An intelligent, ACL-fortified shell powered by [shrs](https://github.com/MrPicklePinosaur/shrs), [gitoxide](https://github.com/vincents-ai/gitoxide), and [vincents-ai/llm](https://github.com/vincents-ai/llm).

OmniShell is both a standalone interactive shell and an embeddable library for agentic-loop's tool system. Three execution modes with per-profile configuration for ACL rules, snapshots, LLM integration, and sandboxing.

## Quick Start

```bash
# Build (requires nix)
nix develop --command bash -c "cargo build"

# Run interactive shell (default: admin mode)
./target/debug/omnishell

# Run in kids mode
./target/debug/omnishell --mode kids

# Run a single command
./target/debug/omnishell -c "ls -la"

# With a specific profile
./target/debug/omnishell --profile kids

# Disable LLM
./target/debug/omnishell --no-llm
```

## Execution Modes

| Mode | ACL | Output | LLM | Sandbox |
|------|-----|--------|-----|---------|
| **Kids** | Strict allowlist (ls, cd, echo, cowsay, etc.) | Emoji + colors | Tutor (encouraging, age-appropriate) | Linux namespace isolation |
| **Agent** | Blocklist only (sudo, rm -rf /) | JSON envelope | Structured JSON commands | None |
| **Admin** | Everything allowed | Plain passthrough | Technical assistant | None |

## Configuration

OmniShell loads config from (in order, later overrides earlier):

1. **System:** `/etc/omnishell/config.toml` or `config.json`
2. **User:** `$XDG_CONFIG_HOME/omnishell/config.toml` or `config.json`
3. **CLI:** `--config path/to/config.toml`

If no config file is found, sensible defaults are used.

See [docs/configuration.md](docs/configuration.md) for the full reference.

### Example (TOML)

```toml
# ~/.config/omnishell/config.toml
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

[profile.kids.llm]
model = "llama3"
temperature = 0.3
max_tokens = 256

[profile.agent]
mode = "agent"

[profile.admin]
mode = "admin"
```

More examples in [docs/examples/](docs/examples/).

## Built-in Commands

| Command | Description |
|---------|-------------|
| `?` or `ai <prompt>` | Ask the LLM assistant |
| `snapshots` | List command snapshots |
| `undo [n]` | Undo last n commands (default: 1) |
| `redo [n]` | Redo last n undone commands (default: 1) |
| `allow <cmd>` | Add command to allowlist (admin only) |
| `block <cmd>` | Add command to blocklist |
| `mode [mode]` | Show or switch mode (kids/agent/admin) |
| `help` | Show help |
| `exit` / `quit` | Exit the shell |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  omnishell binary                                      │
│                                                         │
│  ┌──────────┐  ┌─────────┐  ┌───────────────────────┐  │
│  │   shrs   │  │   ACL   │  │    LLM Integration    │  │
│  │  (REPL)  │  │ Engine  │  │  (OpenAI/Anthropic/   │  │
│  │          │  │         │  │   Ollama/Custom)      │  │
│  └────┬─────┘  └────┬────┘  └───────────┬───────────┘  │
│       │              │                   │              │
│  ┌────┴──────────────┴───────────────────┴───────────┐  │
│  │               OmniShell Core                      │  │
│  │  Profiles · Config · History · Audit · Snapshots  │  │
│  └───────────────────────┬───────────────────────────┘  │
│                          │                              │
│  ┌───────────────────────┴───────────────────────────┐  │
│  │               Sandbox (Kids mode)                  │  │
│  │         Linux namespace + cgroup isolation         │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

## Library Usage

OmniShell is also a library crate. Use it to embed shell execution in your agent loop:

```rust
use omnishell::{OmniShellConfig, AclEngine, Mode, Verdict, load_config};

let config = load_config(None).unwrap();
let mut acl = AclEngine::new(Mode::Agent);

match acl.evaluate("rm -rf /") {
    Verdict::Deny(reason) => println!("Blocked: {}", reason),
    Verdict::Allow => println!("Allowed"),
}
```

## License

AGPL-3.0-or-later OR LicenseRef-Commercial
