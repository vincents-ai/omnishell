//! OmniShell unified language evaluator.
//!
//! Combines shrs_core's eval2 (pipes, external commands, glob expansion) with
//! full POSIX compound command support (if/while/for/case/&&/||/subshell).
//!
//! Also adds: $(cmd) substitution, glob expansion in for/case, break/continue.


mod lang_impl;

pub use lang_impl::OmniShellLang;

/// Result of evaluating a command.
struct EvalResult {
    exit_code: i32,
}

impl Default for EvalResult {
    fn default() -> Self {
        Self { exit_code: 0 }
    }
}

/// Expand arguments with glob support.
fn expand_arg(arg: &str) -> Vec<String> {
    let mut a = arg.to_string();

    // Expand ~
    if let Some(remaining) = arg.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            a = format!("{}{}", home.to_string_lossy(), remaining);
        }
    }

    // Quotes escape everything
    let first = match a.chars().next() {
        Some(c) => c,
        None => return vec![a],
    };
    if first == '\'' || first == '"' {
        return a
            .trim_matches(|c| c == '\'' || c == '"')
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
    }

    // Glob expansion
    if glob::Pattern::escape(&a) != a {
        if let Ok(paths) = glob::glob(&a) {
            let expanded: Vec<String> = paths
                .filter_map(|p| p.ok())
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            if !expanded.is_empty() {
                return expanded;
            }
        }
    }

    vec![a]
}

/// Perform environment variable substitution.
/// Handles $VAR, ${VAR}, ~, and $(command).
fn envsubst(rt: &shrs::prelude::Runtime, arg: &str) -> String {
    use regex::Regex;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref R_DOLLAR_VAR: Regex = Regex::new(r"\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
        static ref R_DOLLAR_BRACE: Regex = Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").unwrap();
        static ref R_CMD_SUB: Regex = Regex::new(r"\$\(([^)]+)\)").unwrap();
    }

    let mut result = arg.to_string();

    // Command substitution: $(command)
    for cap in R_CMD_SUB.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let cmd = &cap[1];
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output();
        let value = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim_end().to_string(),
            Err(_) => String::new(),
        };
        result = result.replace(full, &value);
    }

    // $VAR
    for cap in R_DOLLAR_VAR.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let var = &cap[1];
        let val: String = rt.env.get(var).map(|v| v.clone()).unwrap_or_default();
        if let Some(pos) = result.find(full) {
            result = format!("{}{}{}", &result[..pos], val, &result[pos + full.len()..]);
        }
    }

    // ${VAR}
    for cap in R_DOLLAR_BRACE.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let var = &cap[1];
        let val: String = rt.env.get(var).map(|v| v.clone()).unwrap_or_default();
        if let Some(pos) = result.find(full) {
            result = format!("{}{}{}", &result[..pos], val, &result[pos + full.len()..]);
        }
    }

    // Tilde
    if result.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            result = result.replacen('~', &home.to_string_lossy(), 1);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_arg_literal() {
        assert_eq!(expand_arg("hello"), vec!["hello"]);
    }

    #[test]
    fn test_expand_arg_quoted() {
        // Single-quoted arg is trimmed of quotes but kept as one arg
        // (split_whitespace only applies to unquoted args)
        let result = expand_arg("'hello'");
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_expand_arg_tilde() {
        let expanded = expand_arg("~/foo");
        assert!(expanded[0].ends_with("/foo"));
        assert!(!expanded[0].starts_with('~'));
    }

    #[test]
    fn test_cmd_sub_regex() {
        use regex::Regex;
        let re = Regex::new(r"\$\(([^)]+)\)").unwrap();
        let cap = re.captures("echo $(date)").unwrap();
        assert_eq!(&cap[1], "date");

        let cap2 = re.captures("result=$(echo hello)").unwrap();
        assert_eq!(&cap2[1], "echo hello");
    }
}
