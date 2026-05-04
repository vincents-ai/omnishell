//! Interactive shell construction for OmniShell.

use omnishell::theme::Theme;
use omnishell::{completion, Mode};

/// Build and run the interactive shell.
pub fn run_interactive_shell(mode: Mode, theme: &Theme) {
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
    let theme_name = theme.name.clone();

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
        .with_state(omnishell::acl::AclEngine::new(mode))
        .with_state(omnishell::history::History::new(
            mode,
            omnishell::history::HistoryConfig::default(),
        ))
        .with_completer(completer)
        .with_prompt(prompt)
        .build()
        .unwrap();

    myshell.run().unwrap();
}
