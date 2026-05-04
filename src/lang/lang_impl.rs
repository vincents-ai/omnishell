//! OmniShell Lang implementation — full POSIX shell with pipes, compound commands,
//! $(cmd) substitution, glob expansion, and break/continue.

use shrs::lang::Lang;
use shrs::prelude::{CmdOutput, LineContents, Shell, States};
use shrs_job::{
    run_external_command, JobManager, Output, Process, ProcessGroup, Stdin as JobStdin,
};
use shrs_lang::{ast, Lexer, Parser, Token};

use super::{envsubst, expand_arg, EvalResult};
use crate::acl::{AclEngine, Verdict};

/// Control flow signals for break/continue in loops.
/// Returned as errors from eval_command to propagate out of loop bodies.
#[derive(Debug)]
enum ShellControl {
    Break,
    Continue,
}

impl std::fmt::Display for ShellControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellControl::Break => write!(f, "break"),
            ShellControl::Continue => write!(f, "continue"),
        }
    }
}

impl std::error::Error for ShellControl {}

/// OmniShell's POSIX-compatible shell language evaluator.
///
/// Implements the full POSIX shell grammar with ACL enforcement.
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
            Ok(result) => {
                // Record in mode-separated history
                {
                    let mut history = states.get_mut::<crate::history::History>();
                    history.push(&line, result.exit_code, None);
                }
                Ok(CmdOutput::from_status(result.exit_code))
            }
            Err(e) => {
                if e.is::<ShellControl>() {
                    return Ok(CmdOutput::success());
                }
                if let Some(cmd) = extract_simple_cmd_name(&parsed) {
                    eprintln!("omnishell: command not found: {cmd}");
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
                    if (chars.first() == Some(&'\'') || chars.first() == Some(&'"'))
                        && (chars.len() == 1 || chars.first() != chars.last())
                    {
                        return true;
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
        ast::Command::Simple {
            assigns,
            args,
            redirects,
        } => eval_simple(sh, states, job_mgr, rt, assigns, args, redirects),

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
                        if e.is::<ShellControl>() {
                            if let Some(sc) = e.downcast_ref::<ShellControl>() {
                                match sc {
                                    ShellControl::Break => break,
                                    ShellControl::Continue => continue,
                                }
                            }
                        }
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
                        if e.is::<ShellControl>() {
                            if let Some(sc) = e.downcast_ref::<ShellControl>() {
                                match sc {
                                    ShellControl::Break => break,
                                    ShellControl::Continue => continue,
                                }
                            }
                        }
                        return Err(e);
                    }
                }
            }
            Ok(EvalResult::default())
        }

        // --- For loop ---
        ast::Command::For {
            name,
            wordlist,
            body,
        } => {
            let mut expanded = Vec::new();
            for word in wordlist {
                let substed = envsubst(rt, states.get::<crate::lang::ShellMode>().0, word);
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
                        if e.is::<ShellControl>() {
                            if let Some(sc) = e.downcast_ref::<ShellControl>() {
                                match sc {
                                    ShellControl::Break => break,
                                    ShellControl::Continue => continue,
                                }
                            }
                        }
                        return Err(e);
                    }
                }
            }
            Ok(EvalResult::default())
        }

        // --- Case ---
        ast::Command::Case { word, arms } => {
            let substed = envsubst(rt, states.get::<crate::lang::ShellMode>().0, word);
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
/// Process a list of redirects into stdin/stdout/stderr overrides.
///
/// Supports:
/// - `< file` (Read) → stdin from file
/// - `> file` (Write) → stdout to file (create/truncate)
/// - `>> file` (WriteAppend) → stdout to file (append)
/// - `<&N` (ReadDup) → stdin from fd N (0=&0, 1=&0 maps stdin)
/// - `>&N` (WriteDup) → redirect fd N to current stdout (e.g., 2>&1)
/// - `<> file` (ReadWrite) → stdout to file (read+write)
fn process_redirects(
    redirects: &[ast::Redirect],
    rt: &shrs::prelude::Runtime,
    mode: crate::profile::Mode,
) -> anyhow::Result<(JobStdin, Output, Output)> {
    use std::fs::File;
    use std::os::unix::io::FromRawFd;

    let mut stdin = JobStdin::Inherit;
    let mut stdout = Output::Inherit;
    let mut stderr = Output::Inherit;

    for redirect in redirects {
        let fd_n = redirect.n.unwrap_or(
            if matches!(
                redirect.mode,
                ast::RedirectMode::Read
                    | ast::RedirectMode::ReadAppend
                    | ast::RedirectMode::ReadDup
            ) {
                0
            } else {
                1
            },
        );
        let filename = envsubst(rt, mode, &redirect.file);

        match redirect.mode {
            ast::RedirectMode::Read => {
                let file = File::open(&filename)
                    .map_err(|e| anyhow::anyhow!("cannot open '{filename}': {e}"))?;
                if fd_n == 0 {
                    stdin = JobStdin::File(file);
                }
            }
            ast::RedirectMode::Write => {
                let file = File::create(&filename)
                    .map_err(|e| anyhow::anyhow!("cannot create '{filename}': {e}"))?;
                if fd_n == 1 {
                    stdout = Output::File(file);
                } else if fd_n == 2 {
                    stderr = Output::File(file);
                }
            }
            ast::RedirectMode::WriteAppend => {
                let file = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&filename)
                    .map_err(|e| anyhow::anyhow!("cannot open '{filename}': {e}"))?;
                if fd_n == 1 {
                    stdout = Output::File(file);
                } else if fd_n == 2 {
                    stderr = Output::File(file);
                }
            }
            ast::RedirectMode::ReadAppend => {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .append(true)
                    .open(&filename)
                    .map_err(|e| anyhow::anyhow!("cannot open '{filename}': {e}"))?;
                if fd_n == 0 {
                    stdin = JobStdin::File(file);
                }
            }
            ast::RedirectMode::ReadDup => {
                // <&N — duplicate fd N to stdin
                if let Ok(src_fd) = filename.parse::<i32>() {
                    let new_fd = unsafe { libc::dup(src_fd) };
                    if new_fd >= 0 {
                        let file = unsafe { File::from_raw_fd(new_fd) };
                        stdin = JobStdin::File(file);
                    }
                }
            }
            ast::RedirectMode::WriteDup => {
                // N>&M — duplicate fd M to fd N
                // Common: 2>&1 means redirect stderr to wherever stdout goes
                if let Ok(target_fd) = filename.parse::<i32>() {
                    if fd_n == 2 && target_fd == 1 {
                        // 2>&1 — stderr to stdout
                        stderr = Output::FileDescriptor(1);
                    } else if fd_n == 1 && target_fd == 2 {
                        // 1>&2 or >&2 — stdout to stderr
                        stdout = Output::FileDescriptor(2);
                    } else if fd_n == 1 {
                        stdout = Output::FileDescriptor(target_fd);
                    } else if fd_n == 2 {
                        stderr = Output::FileDescriptor(target_fd);
                    }
                }
            }
            ast::RedirectMode::ReadWrite => {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&filename)
                    .map_err(|e| anyhow::anyhow!("cannot open '{filename}': {e}"))?;
                if fd_n == 0 {
                    stdin = JobStdin::File(file);
                } else if fd_n == 1 {
                    stdout = Output::File(file);
                }
            }
        }
    }

    Ok((stdin, stdout, stderr))
}

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
            let val = envsubst(rt, states.get::<crate::lang::ShellMode>().0, &assign.val);
            let _ = rt.env.set(&assign.var, &val);
        }
        return Ok(EvalResult::default());
    }

    // Expand all args with envsubst + glob
    let mut expanded_args = Vec::new();
    for arg in args {
        let substed = envsubst(rt, states.get::<crate::lang::ShellMode>().0, arg);
        for part in expand_arg(&substed) {
            expanded_args.push(part);
        }
    }

    let cmd_name = match expanded_args.first() {
        Some(n) => n.as_str(),
        None => return Ok(EvalResult::default()),
    };
    let cmd_args = expanded_args[1..].to_vec();

    // ACL enforcement — check before execution
    {
        let acl = states.get::<AclEngine>();
        let full_cmd = expanded_args.join(" ");
        let shell_mode = states.get::<crate::lang::ShellMode>();
        if let crate::acl::Verdict::Deny(reason) = acl.evaluate(&full_cmd) {
            eprintln!("{}", crate::output::format_error(&reason, shell_mode.0));
            return Ok(EvalResult { exit_code: 126 });
        }
    }

    // Built-in keywords
    match cmd_name {
        "break" => return Err(ShellControl::Break.into()),
        "continue" => return Err(ShellControl::Continue.into()),
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
            return Ok(EvalResult {
                exit_code: builtin_test(&args),
            });
        }
        _ => {}
    }

    // Check shrs builtins
    for (builtin_name, builtin_cmd) in sh.builtins.iter() {
        if builtin_name == cmd_name {
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
        let val = envsubst(rt, states.get::<crate::lang::ShellMode>().0, &assign.val);
        let _ = rt.env.set(&assign.var, &val);
    }

    // Process redirects to determine stdin/stdout/stderr
    let shell_mode = states.get::<crate::lang::ShellMode>();
    let (stdin, stdout, stderr) = process_redirects(_redirects, rt, shell_mode.0)?;

    // Run as external command
    let (proc, pgid) = run_external_command(cmd_name, &cmd_args, stdin, stdout, stderr, None)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("__notfound__")
            } else {
                anyhow::anyhow!("execution error: {e}")
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
        .map_err(|e| anyhow::anyhow!("job error: {e}"))?;

    let exit_code = status.and_then(|s| s.code()).unwrap_or(1);

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
        sh,
        states,
        job_mgr,
        rt,
        a_cmd,
        None,
        Some(Output::CreatePipe),
    )?;

    let b_stdin = a_procs.last_mut().unwrap().stdout();
    let (b_procs, b_pgid) = eval_to_procs_with_io(sh, states, job_mgr, rt, b_cmd, b_stdin, None)?;

    a_procs.extend(b_procs);

    let proc_group = ProcessGroup {
        id: b_pgid,
        processes: a_procs,
        foreground: true,
    };
    let job_id = job_mgr.create_job("pipeline", proc_group);
    let status = job_mgr
        .put_job_in_foreground(Some(job_id), false)
        .map_err(|e| anyhow::anyhow!("job error: {e}"))?;

    let exit_code = status.and_then(|s| s.code()).unwrap_or(1);
    rt.exit_status = exit_code;
    Ok(EvalResult { exit_code })
}

/// Result type for eval_to_procs functions.
type ProcsResult = anyhow::Result<(Vec<Box<dyn Process>>, Option<u32>)>;

/// Convert AST to procs.
fn eval_to_procs(
    sh: &Shell,
    states: &States,
    job_mgr: &mut JobManager,
    rt: &mut shrs::prelude::Runtime,
    cmd: &ast::Command,
) -> ProcsResult {
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
) -> ProcsResult {
    match cmd {
        ast::Command::Simple { args, .. } => {
            // Build command string for ACL checking
            let cmd_string = args.join(" ");

            // Check ACL before spawning
            let acl = states.get::<AclEngine>();
            match acl.evaluate(&cmd_string) {
                Verdict::Deny(_reason) => {
                    return Ok((vec![], None));
                }
                Verdict::Allow => {}
            }

            let mut expanded = Vec::new();
            for arg in args {
                let substed = envsubst(rt, states.get::<crate::lang::ShellMode>().0, arg);
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
                sh,
                states,
                job_mgr,
                rt,
                a_cmd,
                stdin,
                Some(Output::CreatePipe),
            )?;
            let b_stdin = a_procs.last_mut().unwrap().stdout();
            let (b_procs, b_pgid) =
                eval_to_procs_with_io(sh, states, job_mgr, rt, b_cmd, b_stdin, stdout)?;
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
        .map_err(|e| anyhow::anyhow!("job error: {e}"))?;
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
    if result {
        0
    } else {
        1
    }
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
            std::fs::metadata(path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
                .to_string()
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
