# Agent Mode JSON Contract

Agent mode (`omnishell --mode agent`) is the machine-readable interface for AI
agents and automation. Every command produces exactly one JSON envelope on
stdout.

## Stream Contract

- **stdout:** one JSON object per command, one object per line. Nothing else is
  written there in agent mode.
- **stderr:** startup banners, prompts, progress, and any diagnostics that are
  not part of the envelope.
- **stdin:** POSIX shell syntax, either from `-c "..."` or from an interactive
  line.
- The process exit status equals the envelope's `exitCode`.
- In interactive agent mode an envelope is printed after each command line.

The `command` field is the command string as OmniShell received it. `stdout` and
`stderr` are the command's captured streams as strings; embedded newlines are
JSON-escaped. `durationMs` measures command execution (it is `0` when the
command was blocked before anything ran).

## Schema

```json
{
  "type": "success" | "error",
  "command": "string",
  "stdout": "string",
  "stderr": "string",
  "exitCode": 0,
  "durationMs": 0
}
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `success` when `exitCode == 0`, otherwise `error` |
| `command` | string | The command OmniShell executed |
| `stdout` | string | Captured standard output |
| `stderr` | string | Captured standard error, including ACL denial reasons |
| `exitCode` | integer | Process exit code |
| `durationMs` | integer | Command duration in milliseconds |

This is the `JsonEnvelope` type in `src/output.rs`, serialized by
`format_output`.

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Command succeeded |
| `1` | Command ran and failed (general failure) |
| `126` | Blocked by the ACL before execution |
| `127` | Command not found |

## Request / Response Examples

### Successful command

Request:

```bash
omnishell --mode agent -c "echo hi"
```

stdout (one line):

```json
{"type":"success","command":"echo hi","stdout":"hi\n","stderr":"","exitCode":0,"durationMs":2}
```

stderr: empty. Process exit status: `0`.

### Failing command

Request:

```bash
omnishell --mode agent -c "sh -c 'exit 3'"
```

stdout:

```json
{"type":"error","command":"sh -c 'exit 3'","stdout":"","stderr":"","exitCode":3,"durationMs":15}
```

Process exit status: `3`.

### Blocked command

Request:

```bash
omnishell --mode agent -c "rm -rf /"
```

stdout:

```json
{"type":"error","command":"rm -rf /","stdout":"","stderr":"omnishell: Command 'rm' blocked: Deletion of the filesystem root blocked (target: /)","exitCode":126,"durationMs":0}
```

Process exit status: `126`. The reason is in the envelope's `stderr`, not on the
process's stderr stream.

### Command not found

Request:

```bash
omnishell --mode agent -c "definitely-not-a-command"
```

stdout:

```json
{"type":"error","command":"definitely-not-a-command","stdout":"","stderr":"omnishell: command not found: definitely-not-a-command\n","exitCode":127,"durationMs":3}
```

Process exit status: `127`.

## Relationship to the Other Modes

Kids mode keeps its emoji/colored text output, and admin mode keeps plain
passthrough. Only agent mode wraps output in the JSON envelope.
