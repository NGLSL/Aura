//! Windows argument quoting and launch-target preparation.

use super::LaunchError;
use crate::command::{resolve_command, CommandError, ResolvedCommand};
use envbox_core::LaunchTarget;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Join argv for display/edit (CommandLineToArgvW-safe; inverse of `parse_args`).
pub fn format_args(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a Windows argument string into argv (CommandLineToArgvW rules).
/// Inverse of `format_args`; do not use `split_whitespace` (drops quoting).
pub fn parse_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut n = 1usize;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    n += 1;
                }
                if chars.peek() == Some(&'"') {
                    cur.extend(std::iter::repeat('\\').take(n / 2));
                    chars.next();
                    if n % 2 == 1 {
                        cur.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    has_token = true;
                } else {
                    cur.extend(std::iter::repeat('\\').take(n));
                    has_token = true;
                }
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if !in_quotes && (c == ' ' || c == '\t') => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// Quote one Windows argument for CreateProcess command line (CommandLineToArgvW rules).
pub fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = arg.contains(' ') || arg.contains('\t') || arg.contains('"');
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

/// `cmd.exe /c` parses its command tail itself rather than using the C runtime
/// argument rules. Escaping the tail with `quote_arg` inserts literal
/// backslashes before quotes and breaks paths such as `C:\Program Files`.
pub(super) fn create_process_command_line(program: &Path, args: &[String]) -> String {
    let mut line = quote_arg(&program.display().to_string());
    let cmd_tail = program
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
        && args.len() >= 2
        && args[..args.len() - 1]
            .iter()
            .any(|arg| arg.eq_ignore_ascii_case("/c"));
    for (index, arg) in args.iter().enumerate() {
        line.push(' ');
        if cmd_tail && index == args.len() - 1 {
            line.push_str(arg);
        } else {
            line.push_str(&quote_arg(arg));
        }
    }
    line
}

pub(super) fn classify_exe(path: &Path) -> Result<ResolvedCommand, LaunchError> {
    if !path.is_file() {
        return Err(CommandError::CommandNotFound(path.display().to_string()).into());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let via_comspec = ext == "cmd" || ext == "bat";
    Ok(ResolvedCommand {
        program: path.to_path_buf(),
        via_comspec,
        comspec_payload: via_comspec.then(|| path.display().to_string()),
    })
}

/// Resolve the executable that actually runs, including ComSpec, before
/// staging an architecture-specific Runtime for the Session bootstrap.
pub(crate) fn activation_program(
    target: &LaunchTarget,
    arguments: &[String],
    environment: &HashMap<String, String>,
) -> Result<PathBuf, LaunchError> {
    let resolved = match target {
        LaunchTarget::Executable { path } => classify_exe(path)?,
        LaunchTarget::Command { command } => {
            let path = environment
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
                .map(|(_, value)| value.as_str());
            resolve_command(command, path)?
        }
        LaunchTarget::Packaged { .. } => {
            return Err(LaunchError::create_process_msg(
                "packaged target has no Win32 activation executable",
            ));
        }
    };
    spawn_args(&resolved, arguments, environment).map(|(program, _)| program)
}

pub(super) fn spawn_args(
    resolved: &ResolvedCommand,
    user_args: &[String],
    env: &HashMap<String, String>,
) -> Result<(PathBuf, Vec<String>), LaunchError> {
    if resolved.via_comspec {
        let comspec = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("ComSpec"))
            .map(|(_, v)| v.clone())
            .or_else(|| {
                std::env::var("ComSpec")
                    .ok()
                    .or_else(|| std::env::var("COMSPEC").ok())
            })
            .ok_or(LaunchError::ComSpecMissing)?;
        let payload = resolved
            .comspec_payload
            .clone()
            .unwrap_or_else(|| resolved.program.display().to_string());
        validate_cmd_value(&payload)?;
        for arg in user_args {
            validate_cmd_value(arg)?;
        }
        // cmd /s /c "…" — outer quotes required when payload or args have spaces.
        let mut line = String::new();
        let needs_outer = std::iter::once(payload.as_str())
            .chain(user_args.iter().map(String::as_str))
            .any(cmd_token_needs_quotes);
        if needs_outer {
            line.push('"');
        }
        line.push_str(&quote_cmd_token(&payload));
        for arg in user_args {
            line.push(' ');
            line.push_str(&quote_cmd_token(arg));
        }
        if needs_outer {
            line.push('"');
        }
        Ok((
            PathBuf::from(comspec),
            vec!["/d".into(), "/v:off".into(), "/s".into(), "/c".into(), line],
        ))
    } else {
        Ok((resolved.program.clone(), user_args.to_vec()))
    }
}

/// Quote for cmd.exe /c payload (not CreateProcess argv).
fn quote_cmd_token(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if !cmd_token_needs_quotes(s) {
        s.to_string()
    } else {
        let trailing_slashes = s.chars().rev().take_while(|ch| *ch == '\\').count();
        format!("\"{}{}\"", s, "\\".repeat(trailing_slashes))
    }
}

fn cmd_token_needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '&' | '|' | '<' | '>' | '^' | '(' | ')'))
}

fn validate_cmd_value(value: &str) -> Result<(), LaunchError> {
    if value
        .chars()
        .any(|ch| matches!(ch, '%' | '!' | '"' | '\r' | '\n' | '\0'))
    {
        return Err(LaunchError::create_process_msg(
            "cmd.exe cannot preserve arguments containing %, !, quotes, or control characters; use an .exe target",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_command_tail_keeps_its_own_quotes() {
        let args = [
            "/d".into(),
            "/s".into(),
            "/c".into(),
            r#"""C:\Program Files\nodejs\node.exe" check.js""#.into(),
        ];
        let line = create_process_command_line(Path::new("cmd.exe"), &args);
        assert_eq!(
            line,
            r#"cmd.exe /d /s /c ""C:\Program Files\nodejs\node.exe" check.js""#
        );
    }

    #[test]
    fn quote_arg_plain() {
        assert_eq!(quote_arg("foo"), "foo");
    }

    #[test]
    fn quote_arg_spaces() {
        assert_eq!(quote_arg("a b"), "\"a b\"");
    }

    #[test]
    fn quote_arg_empty() {
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn quote_arg_embedded_quote() {
        let q = quote_arg("a\"b");
        assert!(q.starts_with('"') && q.ends_with('"'));
    }

    #[test]
    fn parse_format_args_round_trip() {
        let cases: Vec<Vec<String>> = vec![
            vec![],
            vec!["foo".into()],
            vec!["a b".into()],
            vec!["".into()],
            vec!["a\"b".into()],
            vec!["a\\".into()],
            vec!["--flag".into(), "value with space".into(), "".into()],
            vec![r"C:\path with space\app.exe".into()],
        ];
        for args in cases {
            let line = format_args(&args);
            let back = parse_args(&line);
            assert_eq!(back, args, "round-trip failed for {args:?} via {line:?}");
        }
    }

    #[test]
    fn parse_args_preserves_quoted_empty_and_spaces() {
        assert_eq!(parse_args("a \"b c\" d"), vec!["a", "b c", "d"]);
        assert_eq!(parse_args("\"\""), vec![""]);
        assert_eq!(parse_args("\"a\\\"b\""), vec!["a\"b"]);
    }

    #[test]
    fn quote_cmd_token_spaces() {
        assert_eq!(quote_cmd_token(r"C:\a b\x.cmd"), r#""C:\a b\x.cmd""#);
        assert_eq!(
            quote_cmd_token("C:\\dir with space\\"),
            "\"C:\\dir with space\\\\\""
        );
        assert_eq!(quote_cmd_token("safe&ver"), "\"safe&ver\"");
    }

    #[test]
    fn comspec_payload_quotes_path_with_spaces() {
        let resolved = ResolvedCommand {
            program: PathBuf::from(r"C:\Program Files\app\run.cmd"),
            via_comspec: true,
            comspec_payload: Some(r"C:\Program Files\app\run.cmd".into()),
        };
        let env = HashMap::from([("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into())]);
        let (prog, args) = spawn_args(&resolved, &["a b".into()], &env).unwrap();
        assert!(prog.ends_with("cmd.exe"));
        assert_eq!(args[0], "/d");
        assert_eq!(args[3], "/c");
        let line = &args[4];
        assert!(line.starts_with('"') && line.ends_with('"'));
        assert!(line.contains(r#"C:\Program Files\app\run.cmd"#));
        assert!(line.contains("\"a b\""));
    }

    #[test]
    fn comspec_rejects_expanding_arguments() {
        let resolved = ResolvedCommand {
            program: PathBuf::from(r"C:\app\run.cmd"),
            via_comspec: true,
            comspec_payload: None,
        };
        let env = HashMap::from([("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into())]);
        assert!(spawn_args(&resolved, &["%PATH%".into()], &env).is_err());
        assert!(spawn_args(&resolved, &["a!b".into()], &env).is_err());
    }
}
