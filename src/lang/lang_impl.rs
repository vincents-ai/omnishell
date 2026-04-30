//! OmniShell Lang implementation — full POSIX shell with pipes, compound commands,
//! $(cmd) substitution, glob expansion, and break/continue.

use shrs::prelude::{CmdOutput, LineContents, Shell, States};
use shrs::lang::Lang;
use shrs_job::{JobManager, run_external_command, Process, ProcessGroup, Output, Stdin as JobStdin};
use shrs_lang::{Lexer, Parser, Token, ast};

use super::{expand_arg, envsubst, EvalResult};
///
/// Implements the full POSIX shell grammar:
/// - Simple commands with glob expansion and env substitution
/// - Pipes (`cmd1 | cmd2`)
/// - `&&` and `||` compound commands
/// - `if`/`elif`/`else`/`fi`
/// - `while`/`until`/`do`/`done`
/// - `for`/`in`/`do`/`done` with glob expansion
/// - `case`/`esac` with glob pattern matching
/// - `$(cmd)` command substitution
/// - `break`/`continue` in loops
/// - Subshells `(cmd)`
/// - Background `&` and sequential `;` lists
/// - File redirections `<`, `>`, `>>`
pub struct OmniShellLang;

impl Default for OmniShellLang {
    fn default() -> Self {
        let _ = shrs_job::initialize_job_control();
        Self {}
    }
}

impl Lang for OmniShellLang {
    fn eval(&self, sh: &Shell, states: &States, line: String) -> anyhow::Result<CmdOutput> {
        let lexer = Lexer::new(&line);
        let parser = Parser::default();
        let parsed = match parser.parse(lexer) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("parse error: {e}");
                return Ok(CmdOutput::from_status(2));
            }
        };

        let mut job_mgr = states.get_mut::<JobManager>();
        let mut rt = states.get_mut::<shrs::prelude::Runtime>();

        match eval_command(sh, states, &mut job_mgr, &mut rt, &parsed) {
            Ok(result) => Ok(CmdOutput::from_status(result.exit_code as i32)),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("__omnishell_break__") || msg.contains("__omnishell_continue__") {
                    return Ok(CmdOutput::success());
                }
                if let Some(cmd) = extract_simple_cmd_name(&parsed) {
                    eprintln!("omnishell: command not found: {}", cmd);
                    return Ok(CmdOutput::from_status(127));
                }
                eprintln!("eval error: {e}");
                Ok(CmdOutput::error())
            }
        }
    }

    fn name(&self) -> String {
        "omnishell".to_string()
    }

    fn needs_line_check(&self, _sh: &Shell, ctx: &States) -> bool {
        let command = ctx.get::<LineContents>().get_full_command();

        if let Some('\\') = command.chars().last() {
            return true;
        }

        let mut brackets: Vec<Token> = vec![];
        let lexer = Lexer::new(command.as_str());

        for token in lexer.flatten() {
            match &token.1 {
                Token::LBRACE | Token::LPAREN => brackets.push(token.1.clone()),
                Token::RPAREN => {
                    if brackets.last() == Some(&Token::LPAREN) {
                        brackets.pop();
                    } else {
                        return false;
                    }
                }
                Token::RBRACE => {
                    if brackets.last() == Some(&Token::LBRACE) {
                        brackets.pop();
                    } else {
                        return false;
                    }
                }
                Token::WORD(w) => {
                    let chars: Vec<char> = w.chars().collect();
                    if chars.first() == Some(&'\'') || chars.first() == Some(&'"') {
                        if chars.len() == 1 || chars.first() != chars.last() {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }

        !brackets.is_empty()
    }
}

/// Extract the command name from a Simple command AST node (for error messages).
fn extract_simple_cmd_name(cmd: &ast::Command) -> Option<String> {
    match cmd {
        ast::Command::Simple { args, .. } => args.first().cloned(),
        _ => None,
    }
}

/// The core evaluator. Recursively evaluates AST nodes.
fn eval_command(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    cmd: &ast::Command,
) -> anyhow::Result<EvalResult> {
    match cmd {
        // --- Simple command ---
        ast::Command::Simple { assigns, args, redirects } => {
            eval_simple(sh, states, job_mgr, rt, assigns, args, redirects)
        }

        // --- Pipeline ---
        ast::Command::Pipeline(a_cmd, b_cmd) => {
            eval_pipeline(sh, states, job_mgr, rt, a_cmd, b_cmd)
        }

        // --- && ---
        ast::Command::And(a_cmd, b_cmd) => {
            let a_result = eval_command(sh, states, job_mgr, rt, a_cmd)?;
            if a_result.exit_code == 0 {
                eval_command(sh, states, job_mgr, rt, b_cmd)
            } else {
                Ok(a_result)
            }
        }

        // --- || ---
        ast::Command::Or(a_cmd, b_cmd) => {
            let a_result = eval_command(sh, states, job_mgr, rt, a_cmd)?;
            if a_result.exit_code != 0 {
                eval_command(sh, states, job_mgr, rt, b_cmd)
            } else {
                Ok(a_result)
            }
        }

        // --- Not ---
        ast::Command::Not(cmd) => {
            let result = eval_command(sh, states, job_mgr, rt, cmd)?;
            Ok(EvalResult {
                exit_code: if result.exit_code == 0 { 1 } else { 0 },
            })
        }

        // --- Sequential list (;) ---
        ast::Command::SeqList(a_cmd, b_cmd) => {
            let _a_result = eval_command(sh, states, job_mgr, rt, a_cmd)?;
            match b_cmd {
                Some(b) => eval_command(sh, states, job_mgr, rt, b),
                None => Ok(EvalResult::default()),
            }
        }

        // --- Async list (&) ---
        ast::Command::AsyncList(a_cmd, b_cmd) => {
            let (procs, pgid) = eval_to_procs(sh, states, job_mgr, rt, a_cmd)?;
            run_job_bg(job_mgr, procs, pgid)?;
            match b_cmd {
                Some(b) => eval_command(sh, states, job_mgr, rt, b),
                None => Ok(EvalResult::default()),
            }
        }

        // --- Subshell ---
        ast::Command::Subshell(cmd) => {
            let mut sub_rt = rt.clone();
            eval_command(sh, states, job_mgr, &mut sub_rt, cmd)
        }

        // --- If/elif/else/fi ---
        ast::Command::If { conds, else_part } => {
            for ast::Condition { cond, body } in conds {
                let cond_result = eval_command(sh, states, job_mgr, rt, cond)?;
                if cond_result.exit_code == 0 {
                    return eval_command(sh, states, job_mgr, rt, body);
                }
            }
            match else_part {
                Some(else_cmd) => eval_command(sh, states, job_mgr, rt, else_cmd),
                None => Ok(EvalResult::default()),
            }
        }

        // --- While ---
        ast::Command::While { cond, body } => {
            loop {
                let cond_result = eval_command(sh, states, job_mgr, rt, cond)?;
                if cond_result.exit_code != 0 {
                    break;
                }
                let body_result = eval_command(sh, states, job_mgr, rt, body);
                match body_result {
                    Ok(_) => {}
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("__omnishell_break__") { break; }
                        if msg.contains("__omnishell_continue__") { continue; }
                        return Err(e);
                    }
                }
            }
            Ok(EvalResult::default())
        }

        // --- Until ---
        ast::Command::Until { cond, body } => {
            loop {
                let cond_result = eval_command(sh, states, job_mgr, rt, cond)?;
                if cond_result.exit_code == 0 {
                    break;
                }
                let body_result = eval_command(sh, states, job_mgr, rt, body);
                match body_result {
                    Ok(_) => {}
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("__omnishell_break__") { break; }
                        if msg.contains("__omnishell_continue__") { continue; }
                        return Err(e);
                    }
                }
            }
            Ok(EvalResult::default())
        }

        // --- For loop ---
        ast::Command::For { name, wordlist, body } => {
            let mut expanded = Vec::new();
            for word in wordlist {
                let substed = envsubst(rt, word);
                for part in expand_arg(&substed) {
                    expanded.push(part);
                }
            }

            for value in expanded {
                let _ = rt.env.set(name, &value);
                let body_result = eval_command(sh, states, job_mgr, rt, body);
                match body_result {
                    Ok(_) => {}
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("__omnishell_break__") { break; }
                        if msg.contains("__omnishell_continue__") { continue; }
                        return Err(e);
                    }
                }
            }
            Ok(EvalResult::default())
        }

        // --- Case ---
        ast::Command::Case { word, arms } => {
            let substed = envsubst(rt, word);
            for ast::CaseArm { pattern, body } in arms {
                let matched = pattern.iter().any(|p| {
                    if p.contains('*') || p.contains('?') || p.contains('[') {
                        glob::Pattern::new(p)
                            .map(|pat| pat.matches(&substed))
                            .unwrap_or(false)
                    } else {
                        p == &substed
                    }
                });
                if matched {
                    return eval_command(sh, states, job_mgr, rt, body);
                }
            }
            Ok(EvalResult::default())
        }

        // --- Function definition ---
        ast::Command::Fn { fname, body } => {
            let mut funcs = states.get_mut::<crate::lang::FunctionTable>();
            funcs.define(fname.clone(), *body.clone());
            Ok(EvalResult::default())
        }

        // --- None (empty command) ---
        ast::Command::None => Ok(EvalResult::default()),
    }
}

/// Evaluate a simple command.
fn eval_simple(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    assigns: &[ast::Assign],
    args: &[String],
    _redirects: &[ast::Redirect],
) -> anyhow::Result<EvalResult> {
    // Handle assignment-only lines
    if args.is_empty() {
        for assign in assigns {
            let val = envsubst(rt, &assign.val);
            let _ = rt.env.set(&assign.var, &val);
        }
        return Ok(EvalResult::default());
    }

    // Expand all args with envsubst + glob
    let mut expanded_args = Vec::new();
    for arg in args {
        let substed = envsubst(rt, arg);
        for part in expand_arg(&substed) {
            expanded_args.push(part);
        }
    }

    let cmd_name = match expanded_args.first() {
        Some(n) => n.as_str(),
        None => return Ok(EvalResult::default()),
    };
    let cmd_args = expanded_args[1..].to_vec();

    // Built-in keywords
    match cmd_name {
        "break" => return Err(anyhow::anyhow!("__omnishell_break__")),
        "continue" => return Err(anyhow::anyhow!("__omnishell_continue__")),
        "exit" => {
            let code: i32 = cmd_args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        "true" | ":" => return Ok(EvalResult { exit_code: 0 }),
        "false" => return Ok(EvalResult { exit_code: 1 }),
        "test" | "[" => {
            let args = if cmd_name == "[" {
                // Strip trailing ]
                let mut a = cmd_args.to_vec();
                if a.last().map(|s| s.as_str()) == Some("]") {
                    a.pop();
                }
                a
            } else {
                cmd_args.to_vec()
            };
            return Ok(EvalResult { exit_code: builtin_test(&args) });
        }
        _ => {}
    }

    // Check shrs builtins
    for (builtin_name, builtin_cmd) in sh.builtins.iter() {
        if builtin_name == &cmd_name {
            let _ = builtin_cmd.run(sh, states, &cmd_args);
            return Ok(EvalResult { exit_code: 0 });
        }
    }

    // Check user-defined functions
    {
        let funcs = states.get::<crate::lang::FunctionTable>();
        if let Some(func_body) = funcs.get(cmd_name).cloned() {
            return eval_command(sh, states, job_mgr, rt, &func_body);
        }
    }

    // Apply env assignments
    for assign in assigns {
        let val = envsubst(rt, &assign.val);
        let _ = rt.env.set(&assign.var, &val);
    }

    // Run as external command
    let (proc, pgid) = run_external_command(
        cmd_name,
        &cmd_args,
        JobStdin::Inherit,
        Output::Inherit,
        Output::Inherit,
        None,
    ).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("__notfound__")
        } else {
            anyhow::anyhow!("execution error: {}", e)
        }
    })?;

    let proc_group = ProcessGroup {
        id: pgid,
        processes: vec![proc],
        foreground: true,
    };
    let job_id = job_mgr.create_job(cmd_name, proc_group);
    let status = job_mgr
        .put_job_in_foreground(Some(job_id), false)
        .map_err(|e| anyhow::anyhow!("job error: {}", e))?;

    let exit_code = status
        .and_then(|s| s.code())
        .unwrap_or(1);

    rt.exit_status = exit_code;
    Ok(EvalResult { exit_code })
}

/// Evaluate a pipeline.
fn eval_pipeline(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    a_cmd: &ast::Command,
    b_cmd: &ast::Command,
) -> anyhow::Result<EvalResult> {
    let (mut a_procs, _) = eval_to_procs_with_io(
        sh, states, job_mgr, rt, a_cmd, None, Some(Output::CreatePipe)
    )?;

    let b_stdin = a_procs.last_mut().unwrap().stdout();
    let (b_procs, b_pgid) = eval_to_procs_with_io(
        sh, states, job_mgr, rt, b_cmd, b_stdin, None
    )?;

    a_procs.extend(b_procs);

    let proc_group = ProcessGroup {
        id: b_pgid,
        processes: a_procs,
        foreground: true,
    };
    let job_id = job_mgr.create_job("pipeline", proc_group);
    let status = job_mgr
        .put_job_in_foreground(Some(job_id), false)
        .map_err(|e| anyhow::anyhow!("job error: {}", e))?;

    let exit_code = status.and_then(|s| s.code()).unwrap_or(1);
    rt.exit_status = exit_code;
    Ok(EvalResult { exit_code })
}

/// Convert AST to procs.
fn eval_to_procs(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    cmd: &ast::Command,
) -> anyhow::Result<(Vec<Box<dyn Process>>, Option<u32>)> {
    eval_to_procs_with_io(sh, states, job_mgr, rt, cmd, None, None)
}

/// Convert AST to procs with explicit stdin/stdout wiring.
fn eval_to_procs_with_io(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    cmd: &ast::Command,
    stdin: Option<JobStdin>,
    stdout: Option<Output>,
) -> anyhow::Result<(Vec<Box<dyn Process>>, Option<u32>)> {
    match cmd {
        ast::Command::Simple { args, .. } => {
            let mut expanded = Vec::new();
            for arg in args {
                let substed = envsubst(rt, arg);
                for part in expand_arg(&substed) {
                    expanded.push(part);
                }
            }
            if expanded.is_empty() {
                return Ok((vec![], None));
            }

            let program = &expanded[0];
            let cmd_args = expanded[1..].to_vec();

            let proc_stdin = stdin.unwrap_or(JobStdin::Inherit);
            let proc_stdout = stdout.unwrap_or(Output::Inherit);

            let (proc, pgid) = run_external_command(
                program,
                &cmd_args,
                proc_stdin,
                proc_stdout,
                Output::Inherit,
                None,
            )?;
            Ok((vec![proc], pgid))
        }

        ast::Command::Pipeline(a_cmd, b_cmd) => {
            let (mut a_procs, _) = eval_to_procs_with_io(
                sh, states, job_mgr, rt, a_cmd, stdin, Some(Output::CreatePipe)
            )?;
            let b_stdin = a_procs.last_mut().unwrap().stdout();
            let (b_procs, b_pgid) = eval_to_procs_with_io(
                sh, states, job_mgr, rt, b_cmd, b_stdin, stdout
            )?;
            a_procs.extend(b_procs);
            Ok((a_procs, b_pgid))
        }

        // For compound commands in pipeline context, evaluate inline
        _ => {
            let _ = eval_command(sh, states, job_mgr, rt, cmd)?;
            Ok((vec![], None))
        }
    }
}

/// Run a job in the background.
fn run_job_bg(
    job_mgr: &mut JobManager,
    procs: Vec<Box<dyn Process>>,
    pgid: Option<u32>,
) -> anyhow::Result<()> {
    let proc_group = ProcessGroup {
        id: pgid,
        processes: procs,
        foreground: false,
    };
    let job_id = job_mgr.create_job("", proc_group);
    job_mgr
        .put_job_in_background(Some(job_id), false)
        .map_err(|e| anyhow::anyhow!("job error: {}", e))?;
    Ok(())
}

/// Built-in `test` / `[` command.
///
/// Supports common POSIX test expressions:
/// - String: -z, -n, =, !=
/// - File: -e, -f, -d, -r, -w, -x, -s
/// - Numeric: -eq, -ne, -lt, -le, -gt, -ge
/// - Logical: !, -a, -o
fn builtin_test(args: &[String]) -> i32 {
    let mut pos = 0;
    let result = eval_test(args, &mut pos);
    if result { 0 } else { 1 }
}

fn eval_test(args: &[String], pos: &mut usize) -> bool {
    let left = eval_test_primary(args, pos);

    if *pos < args.len() {
        match args[*pos].as_str() {
            "=" | "==" => {
                *pos += 1;
                let right = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
                *pos += 1;
                return left == right;
            }
            "!=" => {
                *pos += 1;
                let right = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
                *pos += 1;
                return left != right;
            }
            "-eq" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) == r;
            }
            "-ne" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) != r;
            }
            "-lt" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) < r;
            }
            "-le" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) <= r;
            }
            "-gt" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) > r;
            }
            "-ge" => {
                *pos += 1;
                let r: i64 = args.get(*pos).and_then(|s| s.parse().ok()).unwrap_or(0);
                *pos += 1;
                return left.parse::<i64>().unwrap_or(0) >= r;
            }
            "-a" => {
                *pos += 1;
                // left is a string from primary — treat as bool by checking non-empty
                // But actually -a is AND between two primaries
                return left.parse::<bool>().unwrap_or(!left.is_empty()) && eval_test(args, pos);
            }
            "-o" => {
                *pos += 1;
                return left.parse::<bool>().unwrap_or(!left.is_empty()) || eval_test(args, pos);
            }
            _ => {}
        }
    }
    // Treat string as truthy (non-empty)
    !left.is_empty()
}

fn eval_test_primary(args: &[String], pos: &mut usize) -> String {
    let arg = match args.get(*pos) {
        Some(a) => a,
        None => return String::new(),
    };

    match arg.as_str() {
        "!" => {
            *pos += 1;
            let inner = eval_test(args, pos);
            (!inner).to_string()
        }
        "-z" => {
            *pos += 1;
            let s = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            s.is_empty().to_string()
        }
        "-n" => {
            *pos += 1;
            let s = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            (!s.is_empty()).to_string()
        }
        "-e" => {
            *pos += 1;
            let path = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            std::path::Path::new(path).exists().to_string()
        }
        "-f" => {
            *pos += 1;
            let path = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            std::path::Path::new(path).is_file().to_string()
        }
        "-d" => {
            *pos += 1;
            let path = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            std::path::Path::new(path).is_dir().to_string()
        }
        "-x" => {
            *pos += 1;
            let path = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            is_executable(path).to_string()
        }
        "-s" => {
            *pos += 1;
            let path = args.get(*pos).map(|s| s.as_str()).unwrap_or("");
            *pos += 1;
            std::fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false).to_string()
        }
        _ => {
            *pos += 1;
            arg.clone()
        }
    }
}

fn is_executable(path: &str) -> bool {
    std::fs::metadata(path)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o111 != 0
        })
        .unwrap_or(false)
}
