# OmniShell Architecture

## 1. Module Structure

```
omnishell/
├── src/
│   ├── main.rs           # CLI entry, profile resolution, shell launch
│   ├── lib.rs            # Public API surface
│   ├── profile.rs        # OmniShellConfig, Profile, Mode, ThemeOverride
│   ├── config.rs         # TOML/JSON config loading & merging
│   ├── theme.rs          # Theme (colors, prompt template, emoji)
│   ├── acl.rs            # AclEngine (allowlist/blocklist/evaluate)
│   ├── builtins.rs       # Built-in commands (?, ai, mode, help, exit, allow, block, snapshots, undo, redo)
│   ├── lang/
│   │   ├── mod.rs        # envsubst, expand_arg, arithmetic, $(cmd) substitution
│   │   ├── lang_impl.rs  # OmniShellLang — POSIX evaluator (if/for/while/case/pipes/redirects)
│   │   ├── functions.rs  # FunctionTable — user-defined function storage
│   │   └── shell_mode.rs # ShellMode state for shrs
│   ├── completion.rs     # CompletionEngine — mode-aware tab completion
│   ├── history.rs        # Per-mode command history
│   ├── snapshot.rs       # Pre/post-execution git snapshots via gix
│   ├── undo.rs           # UndoStack (snapshot pairs + trigger command)
│   ├── audit.rs          # AuditLogger (structured command log)
│   ├── output.rs         # Mode-formatted output (emoji/JSON/plain)
│   ├── llm_integration.rs# LLM provider abstraction (OpenAI/Anthropic/Ollama)
│   ├── sandbox.rs        # Linux namespace sandbox (kids mode)
│   ├── plugin.rs         # OmniShellPlugin trait + plugin system
│   ├── engram_backend.rs # Engram CLI integration for agent memory
│   ├── picker.rs         # Interactive profile picker
│   ├── picture.rs        # Profile picture rendering via viuer
│   └── error.rs          # OmniShellError type
├── crates/
│   └── anymap_compat/    # Drop-in anymap replacement (IdHasher, no re-hashing TypeId)
├── tests/
│   ├── integration.rs    # 37 tests (unit-style + scripting via binary)
│   ├── bdd.rs            # 20 BDD tests (cucumber)
│   ├── sandbox.rs        # 17 sandbox config tests
│   └── mock_llm.rs       # LLM mocking
└── docs/
    ├── configuration.md
    ├── OmniShell Product Requirements Document.md
    └── OmniShell Architecture Diagrams.md (this file)
```

## 2. Execution Pipeline

```
User Input
    │
    ▼
shrs readline (prompt from Theme template)
    │
    ▼
OmniShellLang::eval()
    │
    ▼
shrs_lang::Lexer + Parser  ──▶  POSIX AST
    │
    ▼
eval_command() recursive evaluator
    ├── Simple command
    │   ├── envsubst() — $VAR, ${VAR}, $((expr)), $(cmd)
    │   ├── expand_arg() — glob, tilde, quote handling
    │   ├── ACL check (AclEngine::evaluate)
    │   ├── Builtin dispatch (break, continue, exit, test, :, true, false)
    │   ├── User-defined function lookup
    │   ├── process_redirects() — <, >, >>, <&N, >&N, <>
    │   └── shrs_job::run_external_command() — fork+exec
    ├── Pipeline — pipe stdout→stdin between processes
    ├── And/Or — &&, || with short-circuit
    ├── If/While/Until/For/Case — compound commands
    ├── Function definition — stored in FunctionTable
    └── Subshell — cloned runtime environment
```

## 3. Profile Resolution (Priority Order)

```
1. --profile CLI flag      (absolute priority)
2. $USER binding in config  (auto-select on login)
3. default_profile field    (config fallback)
4. First available profile  (last resort)
```

## 4. ACL Evaluation Flow

```
Command string
    │
    ▼
Tokenize (split on whitespace, handle quotes)
    │
    ▼
Mode check
    ├── Kids  → Allowlist only (strict)
    ├── Agent → Blocklist (sudo, rm -rf /, etc.)
    └── Admin → Allow all
    │
    ▼
Extra allow/block from config + runtime builtins
    │
    ▼
Verdict::Allow → execute
Verdict::Deny  → exit 126 + formatted error
```

## 5. Redirect Processing

```
Redirect list from AST
    │
    ▼
process_redirects() computes (stdin, stdout, stderr)
    │
    ├── < file     → stdin = File::open(file)
    ├── > file     → stdout = File::create(file)
    ├── >> file    → stdout = OpenOptions::append(file)
    ├── <&N        → stdin = libc::dup(N)
    ├── 2>&1       → stderr = Output::FileDescriptor(1)
    └── <> file    → stdout = OpenOptions::read+write(file)
    │
    ▼
Passed to shrs_job::run_external_command()
```

## 6. Theme System

```
Profile config
    │
    ▼
Profile::theme() — merge mode defaults + overrides
    │
    ▼
Theme { name, primary, secondary, error, success, prompt, emoji }
    │
    ▼
Prompt closure:
  - Static vars (USER, HOSTNAME) cached outside closure
  - Dynamic vars (cwd) computed per render
  - Template: {user}, {host}, {cwd}, {mode}, {git_branch}, {emoji}
```

## 7. Completion Engine

```
CompletionEngine::new(mode)
    ├── scan_path() — cache all executables from $PATH
    ├── builtin_names() — cache built-in command names
    └── AclEngine::new(mode) — pre-build ACL for mode
    │
    ▼
shrs Completer trait:
    ├── Kids  → builtins + ACL allowlist only
    └── Agent/Admin → builtins + full PATH
```

## 8. Key Dependencies

| Crate | Purpose |
|-------|---------|
| `shrs` | Shell framework (readline, builtins, job control) |
| `shrs_lang` | POSIX shell lexer/parser (AST generation) |
| `shrs_job` | Process management, fork+exec, job control |
| `gix` | Pure Rust git (snapshots, state safety) |
| `vincents-llm` | LLM provider abstraction |
| `clap` | CLI argument parsing |
| `shlex` | Shell-style tokenization (handles quotes/escapes) |
| `anymap_compat` | Drop-in anymap with IdHasher (local patch for shrs) |
