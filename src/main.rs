use clap::{Parser, ValueEnum};
use shrs::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use colored::*;
use serde::Serialize;

// ==========================================
// 1. CLI Arguments & Mode Configuration
// ==========================================

#[derive(Parser, Debug)]
#[command(author, version, about = "OmniShell: An intelligent, ACL-fortified shell")]
struct Args {
    /// Execution mode of the shell
    #[arg(short, long, value_enum, default_value_t = ShellMode::Admin)]
    mode: ShellMode,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum ShellMode {
    Kids,
    Agent,
    Admin,
}

// Ensure ShellMode can be stored in shrs state by implementing Any (already handled if we just use static types, but we'll wrap it)
struct ModeState(ShellMode);

// 2. Access Control List (ACL)
// ==========================================

#[derive(Clone)]
struct CommandAcl {
    mode: ShellMode,
    allowlist: Option<HashSet<String>>,
    blocklist: HashSet<String>,
}

impl CommandAcl {
    fn new(mode: ShellMode) -> Self {
        let mut blocklist = HashSet::new();
        let mut allowlist = None;

        match mode {
            ShellMode::Kids => {
                let mut allowed = HashSet::new();
                for cmd in ["ls", "cd", "pwd", "echo", "cowsay", "fortune", "sl", "cat", "clear"] {
                    allowed.insert(cmd.to_string());
                }
                allowlist = Some(allowed);
            }
            ShellMode::Agent => {
                // Agent has a loose leash, but hard boundaries
                for cmd in ["rm", "reboot", "shutdown", "mkfs", "dd", "wget", "curl"] {
                    blocklist.insert(cmd.to_string());
                }
            }
            ShellMode::Admin => {
                // Empty blocklist, None allowlist = open season
            }
        }

        Self { mode, allowlist, blocklist }
    }

    fn is_permitted(&self, command_line: &str) -> bool {
        let base_cmd = command_line.split_whitespace().next().unwrap_or("");
        
        if self.blocklist.contains(base_cmd) { return false; }
        if let Some(allowed) = &self.allowlist {
            if !allowed.contains(base_cmd) { return false; }
        }
        true
    }
}

// ==========================================
// 3. Gitoxide Auto-Snapshot Hooks
// ==========================================

fn is_mutating(cmd: &str) -> bool {
    let base_cmd = cmd.split_whitespace().next().unwrap_or("");
    matches!(base_cmd, "rm" | "mv" | "cp" | "touch" | "mkdir" | "cargo" | "python" | "git")
}

fn create_snapshot(msg: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    
    // Discover repo. If Err, we are not in a repo, so do nothing.
    let _repo = match gix::discover(&cwd) {
        Ok(r) => r,
        Err(_) => return Ok(()), 
    };

    println!("{}", format!("📦 [Auto-Sync]: {}", msg).dimmed());
    
    // Note: For a fully featured implementation using ONLY gix, you would build the tree here.
    // std::process is used purely as a robust fallback for the shell environment if gix tree APIs are highly unstable in your fork.
    let _ = std::process::Command::new("git").arg("add").arg(".").current_dir(&cwd).status();
    let _ = std::process::Command::new("git").arg("commit").arg("-m").arg(msg).current_dir(&cwd).status();

    Ok(())
}

fn pre_command_hook(
    _sh: &Shell, _ctx: &mut Context, _rt: &mut Runtime, hook_ctx: &BeforeCommandCtx,
) -> anyhow::Result<()> {
    if is_mutating(&hook_ctx.command) {
        let _ = create_snapshot(&format!("PRE-EXEC: {}", hook_ctx.command));
    }
    Ok(())
}

fn post_command_hook(
    _sh: &Shell, ctx: &mut Context, _rt: &mut Runtime, hook_ctx: &AfterCommandCtx,
) -> anyhow::Result<()> {
    // 1. Handle Gitoxide Snapshots
    if is_mutating(&hook_ctx.command) {
        let status = if hook_ctx.cmd_output.status.success() { "SUCCESS" } else { "FAILED" };
        let _ = create_snapshot(&format!("POST-EXEC [{}]: {}", status, hook_ctx.command));
    }

    // 2. Fetch the execution mode from the shell state
    let mode = ctx.state.get::<ModeState>().map(|m| &m.0).unwrap_or(&ShellMode::Admin);

    // 3. The "Explainer" Layer for the Agent
    if *mode == ShellMode::Agent {
        let success = hook_ctx.cmd_output.status.success();
        let code = hook_ctx.cmd_output.status.code().unwrap_or(-1);

        if !success {
            // Generate a contextual explanation for the LLM based on common POSIX exit codes
            let hint = match code {
                127 => "Command not found. Check if the binary is installed and in the PATH.",
                126 => "Command invoked cannot execute. Likely a permission issue (try chmod +x).",
                1   => "Catchall for general errors. Check the stderr output.",
                2   => "Misuse of shell builtins. Check the syntax of the command.",
                130 => "Script terminated by Control-C.",
                _   => "Process returned a non-zero exit code indicating failure.",
            };

            // Format the explanation into a structured, parseable JSON payload
            let explanation = serde_json::json!({
                "event": "execution_failure",
                "command": hook_ctx.command,
                "exit_code": code,
                "shell_explanation": hint,
                "action_required": "Please analyze the error and issue a corrected command."
            });

            // Output this to stdout so the parent Agent Framework can read it
            println!("{}", explanation.to_string());
        } else {
            // Optional: Confirm success in JSON so the agent knows it can proceed
            let confirmation = serde_json::json!({
                "event": "execution_success",
                "command": hook_ctx.command
            });
            println!("{}", confirmation.to_string());
        }
    }

    Ok(())
}

// ==========================================
// 4. LLM Integration Built-in (`?`)
// ==========================================

struct AiBuiltin {
    acl: CommandAcl,
}

impl BuiltinCmd for AiBuiltin {
    fn run(&self, _sh: &Shell, _ctx: &mut Context, _rt: &mut Runtime, args: &[String]) -> anyhow::Result<CmdOutput> {
        let prompt = args.join(" ");
        
        match self.acl.mode {
            ShellMode::Kids => {
                println!("🤖 {}", "Let me think about that...".cyan());
                // MOCK LLM CALL: vincents_ai_llm::generate(&format!("Tutor a 5yo: {}", prompt));
                println!("💡 {}", "To do that, you can use the `ls` command! Try typing it.".green());
                Ok(CmdOutput::success())
            }
            ShellMode::Agent => {
                // MOCK LLM CALL: vincents_ai_llm::generate(&format!("Output JSON posix command for: {}", prompt));
                let llm_cmd = "ls -la"; // Mock output
                
                if self.acl.is_permitted(llm_cmd) {
                    // Agent execution is handled by shrs OS dispatch natively if returned as a command string
                    // For the builtin, we run it manually.
                    let output = std::process::Command::new("sh").arg("-c").arg(llm_cmd).output()?;
                    let json_out = serde_json::json!({
                        "command": llm_cmd,
                        "stdout": String::from_utf8_lossy(&output.stdout),
                        "stderr": String::from_utf8_lossy(&output.stderr),
                        "success": output.status.success()
                    });
                    println!("{}", json_out.to_string());
                    Ok(CmdOutput::success())
                } else {
                    let err = serde_json::json!({"error": "ACL Violation", "command": llm_cmd});
                    eprintln!("{}", err.to_string());
                    Ok(CmdOutput::error())
                }
            }
            ShellMode::Admin => {
                println!("AI processing for admin is open.");
                Ok(CmdOutput::success())
            }
        }
    }
}

// ==========================================
// 5. Main Initialization
// ==========================================

fn main() {
    let args = Args::parse();
    let acl = CommandAcl::new(args.mode.clone());

    // Setup shrs Hooks
    let mut hooks = Hooks::default();
    hooks.insert(pre_command_hook);
    hooks.insert(post_command_hook);
    
    // Setup shrs Builtins
    let mut builtins = Builtins::default();
    builtins.insert("?", AiBuiltin { acl: acl.clone() });

    let mut myshell_builder = ShellBuilder::default()
        .with_hooks(hooks)
        .with_builtins(builtins);

    let mut myshell = myshell_builder.build().expect("Failed to build OmniShell");

    // Inject the mode into the shell's runtime state so hooks can access it
    myshell.ctx.state.insert(ModeState(args.mode.clone()));

    if args.mode == ShellMode::Kids {
        println!("{}", "🚀 Welcome to the Sandbox! Type '?' if you need help.".bold().blue());
    }

    myshell.run().expect("Shell encountered a fatal error");
}
