//! Interactive shell construction for OmniShell.

use omnishell::audit::AuditLogger;
use omnishell::engram_backend::EngramContext;
use omnishell::lang::ShellContext;
use omnishell::snapshot::SnapshotEngine;
use omnishell::theme::Theme;
use omnishell::{completion, Mode};

/// Build and run the interactive shell.
///
/// Now accepts all subsystems that were previously only available in
/// single-command mode: snapshot engine, audit logger, engram context,
/// and profile ACL config.
pub fn run_interactive_shell(
    mode: Mode,
    theme: &Theme,
    snapshot_engine: SnapshotEngine,
    audit_logger: AuditLogger,
    acl_config: Option<omnishell::profile::AclConfig>,
) {
    let working_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let engram_context = EngramContext::new();

    let shell_context = ShellContext::new(
        mode,
        &working_dir,
        snapshot_engine,
        audit_logger,
        engram_context,
        &theme.name,
    );

    build_and_run(mode, theme, shell_context, acl_config);
}

/// Build and run the interactive shell with a pre-built context.
fn build_and_run(mode: Mode, theme: &Theme, shell_context: ShellContext, acl_config: Option<omnishell::profile::AclConfig>) {
    use ::crossterm::style::Stylize;
    use shrs::prelude::*;
    use shrs::readline::prompt::Prompt;

    // Cache static env vars outside the prompt closure
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "localhost".to_string());
    let short_host = hostname.split('.').next().unwrap_or(&hostname).to_string();
    let prompt_template = theme.prompt.clone();
    // Clone the Arc<Mutex<String>> so the prompt closure can read the live theme name
    let current_theme_name = shell_context.current_theme_name.clone();
    let _static_theme_name = theme.name.clone();

    let prompt = Prompt::from_sides(
        move || -> shrs_utils::StyledBuf {
            let cwd = shrs::readline::prompt::top_pwd();

            // Git branch detection via gix
            let git_branch = gix::open(&cwd).ok().and_then(|repo| {
                let name = repo.head_name().ok()??;
                Some(name.shorten().to_string())
            });
            let branch_str = match &git_branch {
                Some(b) => format!(" ({b})"),
                None => String::new(),
            };

            // Read live theme name from shared state (updates on mode switch)
            let theme_name = current_theme_name
                .lock()
                .map(|t| t.clone())
                .unwrap_or_else(|_| _static_theme_name.clone());

            let rendered = prompt_template
                .replace("{user}", &user)
                .replace("{host}", &short_host)
                .replace("{cwd}", &cwd)
                .replace("{mode}", &theme_name)
                .replace("{git_branch}", &branch_str)
                .replace("{emoji}", "");

            styled_buf!(rendered.cyan(),)
        },
        || -> shrs_utils::StyledBuf { styled_buf!() },
    );

    let completer = completion::CompletionEngine::new(mode);

    let myshell = ShellBuilder::default()
        .with_lang(omnishell::lang::OmniShellLang)
        .with_state(omnishell::lang::FunctionTable::new())
        .with_state(omnishell::lang::ShellMode(mode))
        .with_state(omnishell::acl::AclEngine::with_config(mode, acl_config.as_ref()))
        .with_state(omnishell::history::History::new(
            mode,
            omnishell::history::HistoryConfig::default(),
        ))
        .with_state(shell_context)
        .with_completer(completer)
        .with_prompt(prompt)
        .build()
        .unwrap();

    myshell.run().unwrap();
}
