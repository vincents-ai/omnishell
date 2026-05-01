# OmniShell Product Requirements Document

## 1. Executive Summary

**OmniShell** is a natively compiled, POSIX-compliant interactive shell built in Rust using the shrs framework. It integrates `vincents-ai/llm` for intelligent command generation/tutoring and `vincents-ai/gitoxide` for state-safety. The shell supports three execution profiles — Kids, Agent, and Admin — each with distinct ACL rules, output formatting, LLM behaviour, and visual themes.

## 2. Target Personas

### 2.1 Kids (Ages 5–9)

- **Goal:** Learn terminal navigation without risking the host OS
- **Interaction:** Visual, emoji-heavy, AI tutor mode
- **ACL:** Strict allowlist (`ls`, `cd`, `pwd`, `echo`, `cat`, `cowsay`, `fortune`, `date`, `whoami`, `help`, `exit`)
- **Sandbox:** Linux namespace isolation with restricted filesystem
- **Prompt:** `🐚 🧒 {cwd}> ` (configurable via theme)

### 2.2 AI Coding Agent

- **Goal:** Execute multi-step coding/build/deploy tasks in a structured environment
- **Interaction:** Headless or embedded. JSON output envelope
- **ACL:** Blocklist (`sudo`, `su`, `passwd`, `rm -rf /`, `mkfs`, `dd if=/dev/zero`)
- **Output:** `{"type":"success","command":"...","stdout":"...","exitCode":0}`
- **Prompt:** `[🤖 agent] {user}:{cwd}$ ` (configurable via theme)

### 2.3 Admin

- **Goal:** Unrestricted shell access with LLM assistant
- **ACL:** No restrictions
- **Prompt:** `{user}@{host}:{cwd}$ ` (configurable via theme)

## 3. Functional Requirements

### 3.1 Profile & Configuration

| Req | Description |
|-----|-------------|
| REQ-3.1.1 | CLI flags: `--mode kids|agent|admin`, `--profile <name>`, `--config <path>`, `--command <cmd>`, `--no-llm` |
| REQ-3.1.2 | TOML or JSON config at `/etc/omnishell/config.toml`, `$XDG_CONFIG_HOME/omnishell/config.toml`, or `--config` |
| REQ-3.1.3 | Per-profile overrides for LLM, ACL, and theme |
| REQ-3.1.4 | `--profile` CLI flag takes absolute precedence over `$USER` binding |
| REQ-3.1.5 | Theme configuration: configurable PS1 prompt via template variables `{user}`, `{host}`, `{cwd}`, `{mode}`, `{git_branch}`, `{emoji}` |

### 3.2 POSIX Shell Language

| Req | Description |
|-----|-------------|
| REQ-3.2.1 | Full POSIX shell grammar: simple commands, pipes, `if/elif/else/fi`, `while/until/do/done`, `for/in/do/done`, `case/esac`, `&&`, `\|\|`, `!`, `;`, `&` |
| REQ-3.2.2 | Variable assignment: `x=value; echo $x`, `$VAR`, `${VAR}` |
| REQ-3.2.3 | Command substitution: `$(cmd)` via direct fork+exec (no `sh -c`) |
| REQ-3.2.4 | Arithmetic expansion: `$((expr))` with `+`, `-`, `*`, `/`, `%`, parentheses |
| REQ-3.2.5 | Glob expansion in args and `for`/`case` patterns |
| REQ-3.2.6 | Function definitions: `name() { body; }` |
| REQ-3.2.7 | Built-in `test` / `[`: string, file, numeric, logical operators |
| REQ-3.2.8 | File redirections: `<`, `>`, `>>`, `<&N`, `>&N`, `<>` |
| REQ-3.2.9 | Background jobs: `cmd &` |
| REQ-3.2.10 | Subshells: `(cmd)` |

### 3.3 Access Control

| Req | Description |
|-----|-------------|
| REQ-3.3.1 | Every command passes through ACL before execution |
| REQ-3.3.2 | Allowlist (kids) + blocklist (agent) + unrestricted (admin) |
| REQ-3.3.3 | Blocked commands return exit 126 with formatted error |
| REQ-3.3.4 | Runtime ACL modification via `allow`/`block` builtins |
| REQ-3.3.5 | Profile-aware tab completion: kids see allowlist only, agent/admin see full PATH |

### 3.4 LLM Integration

| Req | Description |
|-----|-------------|
| REQ-3.4.1 | Built-in `?` / `ai` command for LLM queries |
| REQ-3.4.2 | Kids: tutor prompt (encouraging, age-appropriate) |
| REQ-3.4.3 | Agent: structured JSON prompt |
| REQ-3.4.4 | Admin: technical assistant prompt |
| REQ-3.4.5 | Providers: OpenAI, Anthropic, Ollama, custom (OpenAI-compatible) |

### 3.5 State Safety

| Req | Description |
|-----|-------------|
| REQ-3.5.1 | Pre/post-execution git snapshots for mutating commands |
| REQ-3.5.2 | Undo/redo stack with `undo`/`redo` builtins |
| REQ-3.5.3 | Audit logging per mode |

### 3.6 Tab Completion

| Req | Description |
|-----|-------------|
| REQ-3.6.1 | Mode-aware command completion (allowlist vs full PATH) |
| REQ-3.6.2 | Built-in command completion |
| REQ-3.6.3 | File path argument completion |

## 4. Non-Functional Requirements

| Category | Requirement |
|----------|-------------|
| Performance | Prompt render <10ms; static env vars cached outside closure |
| Portability | x86_64 and aarch64 Linux/macOS via Nix |
| Reproducibility | `nix profile add github:vincents-ai/omnishell` works out of the box |
| Testing | Unit + integration + BDD tests pass in nix sandbox |
| Security | No `sh -c` in interactive evaluator; direct fork+exec only |
| Licensing | AGPL-3.0-or-later OR LicenseRef-Commercial |

## 5. Build & Distribution

- **Nix flake** with `rustPlatform.buildRustPackage` (real `cargoHash`, not `fakeHash`)
- **Dev shell** via `nix develop` with Rust 1.88.0 stable
- **Single binary** output — no external dependencies
