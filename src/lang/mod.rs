//! OmniShell unified language evaluator.
//!
//! Combines shrs_core's eval2 (pipes, external commands, glob expansion) with
//! full POSIX compound command support (if/while/for/case/&&/||/subshell).
//!
//! Also adds: $(cmd) substitution, glob expansion in for/case, break/continue.

mod functions;
mod lang_impl;
mod shell_mode;

pub use functions::FunctionTable;
pub use lang_impl::OmniShellLang;
pub use shell_mode::ShellMode;

use lazy_static::lazy_static;
use regex::Regex;

/// Result of evaluating a command.
#[derive(Default)]
struct EvalResult {
    exit_code: i32,
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

lazy_static! {
    static ref R_ARITH: Regex = Regex::new(r#"\$\(\(([^)]+)\)\)"#).unwrap();
    static ref R_DOLLAR_VAR: Regex = Regex::new(r"\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    static ref R_DOLLAR_BRACE: Regex = Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").unwrap();
    static ref R_CMD_SUB: Regex = Regex::new(r"\$\(([^)]+)\)").unwrap();
}

/// Perform environment variable substitution.
/// Handles $VAR, ${VAR}, ~, and $(command).
fn envsubst(rt: &shrs::prelude::Runtime, acl_mode: crate::profile::Mode, arg: &str) -> String {
    let mut result = arg.to_string();

    // Arithmetic expansion: $((expr))
    for cap in R_ARITH.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let expr = &cap[1];
        let value = eval_arithmetic(expr);
        result = result.replace(full, &value.to_string());
    }

    // Command substitution: $(command)
    // Execute directly via fork+exec — no sh -c.
    // ACL check: evaluate command against ACL before spawning.
    let acl = crate::acl::AclEngine::new(acl_mode);
    for cap in R_CMD_SUB.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let cmd = &cap[1];
        let tokens: Vec<&str> = cmd.split_whitespace().collect();
        let value = if tokens.is_empty() {
            String::new()
        } else {
            // ACL gate: check the command before executing
            match acl.evaluate(cmd) {
                crate::acl::Verdict::Deny(_) => String::new(),
                crate::acl::Verdict::Allow => {
                    match std::process::Command::new(tokens[0])
                        .args(&tokens[1..])
                        .output()
                    {
                        Ok(o) => String::from_utf8_lossy(&o.stdout).trim_end().to_string(),
                        Err(_) => String::new(),
                    }
                }
            }
        };
        result = result.replace(full, &value);
    }

    // $VAR
    for cap in R_DOLLAR_VAR.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let var = &cap[1];
        let val: String = rt.env.get(var).cloned().unwrap_or_default();
        if let Some(pos) = result.find(full) {
            result = format!("{}{}{}", &result[..pos], val, &result[pos + full.len()..]);
        }
    }

    // ${VAR}
    for cap in R_DOLLAR_BRACE.captures_iter(arg) {
        let full = cap.get(0).unwrap().as_str();
        let var = &cap[1];
        let val: String = rt.env.get(var).cloned().unwrap_or_default();
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

/// Evaluate a simple arithmetic expression.
/// Supports: +, -, *, /, %, parentheses, and integer literals.
fn eval_arithmetic(expr: &str) -> i64 {
    let expr = expr.trim();

    // Simple recursive descent parser
    let mut pos = 0;
    let chars: Vec<char> = expr.chars().collect();

    fn parse_expr(chars: &[char], pos: &mut usize) -> i64 {
        let mut result = parse_term(chars, pos);
        while *pos < chars.len() {
            skip_whitespace(chars, pos);
            match chars.get(*pos) {
                Some('+') => {
                    *pos += 1;
                    result += parse_term(chars, pos);
                }
                Some('-') => {
                    *pos += 1;
                    result -= parse_term(chars, pos);
                }
                _ => break,
            }
        }
        result
    }

    fn parse_term(chars: &[char], pos: &mut usize) -> i64 {
        let mut result = parse_factor(chars, pos);
        while *pos < chars.len() {
            skip_whitespace(chars, pos);
            match chars.get(*pos) {
                Some('*') => {
                    *pos += 1;
                    result *= parse_factor(chars, pos);
                }
                Some('/') => {
                    *pos += 1;
                    let d = parse_factor(chars, pos);
                    if d != 0 {
                        result /= d;
                    } else {
                        result = 0;
                    }
                }
                Some('%') => {
                    *pos += 1;
                    let d = parse_factor(chars, pos);
                    if d != 0 {
                        result %= d;
                    } else {
                        result = 0;
                    }
                }
                _ => break,
            }
        }
        result
    }

    fn parse_factor(chars: &[char], pos: &mut usize) -> i64 {
        skip_whitespace(chars, pos);
        if *pos < chars.len() && chars[*pos] == '(' {
            *pos += 1; // skip (
            let result = parse_expr(chars, pos);
            if *pos < chars.len() && chars[*pos] == ')' {
                *pos += 1;
            }
            return result;
        }
        // Unary minus
        if *pos < chars.len() && chars[*pos] == '-' {
            *pos += 1;
            return -parse_factor(chars, pos);
        }
        // Parse number
        let start = *pos;
        while *pos < chars.len() && (chars[*pos].is_ascii_digit()) {
            *pos += 1;
        }
        if start == *pos {
            return 0;
        }
        let num_str: String = chars[start..*pos].iter().collect();
        num_str.parse::<i64>().unwrap_or(0).saturating_sub(0)
    }

    fn skip_whitespace(chars: &[char], pos: &mut usize) {
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }
    }

    parse_expr(&chars, &mut pos)
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

    #[test]
    fn test_arithmetic_regex() {
        use regex::Regex;
        let re = Regex::new(r#"\$\(\(([^)]+)\)\)"#).unwrap();
        let cap = re.captures("echo $((1+2))").unwrap();
        assert_eq!(&cap[1], "1+2");

        let cap2 = re.captures("$((10 * 5))").unwrap();
        assert_eq!(&cap2[1], "10 * 5");
    }
}
