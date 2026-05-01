# OmniShell Configuration Reference

## File Locations

OmniShell loads configuration from (later overrides earlier):

| Priority | Location | Format |
|----------|----------|--------|
| 1 (lowest) | `/etc/omnishell/config.toml` or `config.json` | System-wide defaults |
| 2 | `$XDG_CONFIG_HOME/omnishell/config.toml` or `config.json` | User overrides |
| 3 (highest) | `--config path/to/config.toml` | CLI override |

Both TOML and JSON are supported. If no config file exists, OmniShell uses built-in defaults (admin mode, LLM enabled with OpenAI).

## Top-Level Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_profile` | string | `"default"` | Profile to use when no `--profile` or `$USER` binding matches |
| `profile` | map of Profile | `{ "default": { mode: "admin" } }` | Named execution profiles |
| `llm` | LlmConfig | *(see below)* | Global LLM configuration |
| `acl` | AclConfig | *(empty)* | Global ACL overrides |

## Profile

A profile defines a complete execution context. Profiles are selected by:

1. **$USER binding** — if a profile has `username` matching `$USER`, it's enforced (no override)
2. **`--profile` flag** — explicit CLI selection
3. **`default_profile`** — fallback from config
4. **First available** — last resort

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mode` | `"kids"` \| `"agent"` \| `"admin"` | `"admin"` | Execution mode |
| `username` | string? | none | OS username to auto-bind to |
| `display_name` | string? | none | Human-readable name for interactive picker |
| `age` | u8? | none | Age for kids mode (drives LLM tutor tone) |
| `llm` | LlmConfig? | none | Per-profile LLM overrides (merged with global) |
| `acl` | AclConfig? | none | Per-profile ACL overrides |

## LlmConfig

Controls how OmniShell connects to LLM providers.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable LLM features |
| `provider` | string | `"openai"` | Provider: `openai`, `anthropic`, `ollama`, `custom` |
| `model` | string | `"gpt-4o"` | Model identifier (provider-specific) |
| `api_base` | string? | *(provider default)* | API base URL (required for Ollama/custom) |
| `api_key` | string? | none | API key. **Prefer env var `OMNISHELL_LLM_API_KEY`** |
| `temperature` | float | `0.7` | Generation temperature (0.0–2.0) |
| `max_tokens` | int | `1024` | Maximum tokens per request |
| `timeout_secs` | int | `30` | Request timeout in seconds |

### Merge Strategy

Per-profile `llm` overrides global `llm`. If a profile sets `llm.model = "llama3"` but doesn't set `llm.provider`, the global `llm.provider` is used. This allows:

```toml
# Global: use Ollama
[llm]
provider = "ollama"
api_base = "http://localhost:11434"

# Agent profile: override model only
[profile.agent]
mode = "agent"
[profile.agent.llm]
model = "codellama"
max_tokens = 4096
```

### API Keys

**Recommended:** environment variable:
```bash
export OMNISHELL_LLM_API_KEY="sk-..."
```

**Fallback:** config file (note: api_key is never serialized back to disk):
```toml
[llm]
api_key = "sk-..."
```

### Provider-Specific Notes

#### OpenAI
```toml
[llm]
provider = "openai"
model = "gpt-4o"
```

#### Anthropic
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
```

#### Ollama (local)
```toml
[llm]
provider = "ollama"
model = "llama3"
api_base = "http://localhost:11434"
```

#### Custom (OpenAI-compatible)
```toml
[llm]
provider = "custom"
api_base = "http://my-llm-host:8080/v1"
model = "my-model"
```

## Theme Configuration

Each profile can override the PS1 prompt template and emoji via an optional `theme` section.
If no `theme` is set, the mode default is used.

### ThemeOverride

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `prompt` | string? | *(mode default)* | PS1 prompt template |
| `emoji` | string? | *(mode default)* | Mode emoji |

### Prompt Template Variables

| Variable | Description |
|----------|-------------|
| `{user}` | Current `$USER` |
| `{host}` | Hostname (short) |
| `{cwd}` | Current working directory |
| `{mode}` | Mode name (kids/agent/admin) |
| `{git_branch}` | Git branch name (when available) |
| `{emoji}` | Mode emoji |

### Mode Default Prompts

| Mode | Prompt | Emoji |
|------|--------|-------|
| Kids | `🐚 {emoji} {cwd}> ` | 🧒 |
| Agent | `[{mode}] {user}:{cwd}$ ` | 🤖 |
| Admin | `{user}@{host}:{cwd}$ ` | ⚡ |

### Example: Custom prompt for agent profile

```toml
[profile.agent]
mode = "agent"

[profile.agent.theme]
prompt = "{emoji} [{mode}] {user} in {cwd}$ "
emoji = "🛸"
```

### Example: Minimal kids prompt

```toml
[profile.kids]
mode = "kids"

[profile.kids.theme]
prompt = "🐚 > "
```

## AclConfig

Additional ACL rules layered on top of the mode defaults.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `extra_allow` | string[] | `[]` | Commands to allow (added to mode defaults) |
| `extra_block` | string[] | `[]` | Commands to block (added to mode defaults) |

### Mode Defaults

| Mode | Strategy | Default rules |
|------|----------|---------------|
| Kids | Allowlist only | `ls`, `cd`, `pwd`, `echo`, `cat`, `cowsay`, `fortune`, `date`, `whoami`, `help`, `exit` |
| Agent | Blocklist | `sudo`, `su`, `passwd`, `rm -rf /`, `mkfs`, `dd if=/dev/zero` |
| Admin | No restrictions | Everything allowed |

### Example: Extend kids allowlist

```toml
[profile.kids]
mode = "kids"
age = 10

[profile.kids.acl]
extra_allow = ["python3", "git", "code"]
```

### Example: Lock down admin

```toml
[acl]
extra_block = ["format", "fdisk", "mkfs"]
```

## Mode Behavior Summary

### Kids Mode
- **ACL**: Strict allowlist — only explicitly permitted commands
- **Output**: Emoji prefixes (`📁`, `✅`, `❌`), colorized
- **LLM**: Tutor tone — encouraging, age-appropriate explanations
- **Sandbox**: Linux namespace isolation with restricted filesystem
- **Prompt**: `[😊 kids]$` *(configurable via theme.prompt)*

### Agent Mode
- **ACL**: Blocklist — blocks dangerous commands (sudo, rm -rf /, etc.)
- **Output**: JSON envelope `{"type":"output","command":"...","stdout":"...","exitCode":0}`
- **LLM**: Structured — generates commands, explains errors in JSON
- **Sandbox**: None
- **Prompt**: `[🤖 agent]$` *(configurable via theme.prompt)*

### Admin Mode
- **ACL**: No restrictions
- **Output**: Plain passthrough (unchanged)
- **LLM**: Technical assistant
- **Sandbox**: None
- **Prompt**: `[⚡ admin]$` *(configurable via theme.prompt)*
