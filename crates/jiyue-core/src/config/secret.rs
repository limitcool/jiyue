//! Secret values and their resolution.
//!
//! A value in `credentials.toml` is one of three forms:
//! - a literal, used as is;
//! - `env:NAME`, read from the environment when the secret is used;
//! - `cmd:COMMAND LINE`, run without a shell, its trimmed standard output is the secret.
//!
//! The command comes from the user's own file. It is split into arguments the way a shell would
//! split words (quotes and backslash escapes, no variables, no pipes), run with a 5 second
//! timeout, and its output is never logged. Errors name the problem, never the value.

use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use zeroize::Zeroize;

/// Longest time a `cmd:` secret may take.
pub const CMD_TIMEOUT: Duration = Duration::from_secs(5);

/// A secret. `Debug` and `Display` print `***`; the text leaves only through [`SecretString::expose`]
/// and the memory is wiped on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret text. Use it only where the text must be sent, never in messages.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Why a secret could not be resolved. The text names the variable or the failure, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    Empty,
    EnvMissing(String),
    EnvNotUnicode(String),
    CmdEmpty,
    CmdSplit(String),
    CmdSpawn(String),
    CmdTimeout,
    CmdFailed(Option<i32>),
    CmdNoOutput,
    CmdIo(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Empty => write!(f, "密钥为空"),
            SecretError::EnvMissing(name) => write!(f, "环境变量 {name} 未设置"),
            SecretError::EnvNotUnicode(name) => write!(f, "环境变量 {name} 不是有效的文本"),
            SecretError::CmdEmpty => write!(f, "cmd: 后面没有命令"),
            SecretError::CmdSplit(m) => write!(f, "cmd: 命令格式有误：{m}"),
            SecretError::CmdSpawn(m) => write!(f, "无法运行 cmd: 命令（{m}）"),
            SecretError::CmdTimeout => write!(f, "cmd: 命令超过 5 秒未返回，已停止"),
            SecretError::CmdFailed(code) => match code {
                Some(c) => write!(f, "cmd: 命令失败，退出码 {c}"),
                None => write!(f, "cmd: 命令被终止"),
            },
            SecretError::CmdNoOutput => write!(f, "cmd: 命令没有输出密钥"),
            SecretError::CmdIo(m) => write!(f, "cmd: 读取输出失败（{m}）"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Resolves one stored value to the secret it stands for.
pub fn resolve_secret(stored: &str) -> Result<SecretString, SecretError> {
    let stored = stored.trim();
    if let Some(name) = stored.strip_prefix("env:") {
        return resolve_env(name.trim());
    }
    if let Some(line) = stored.strip_prefix("cmd:") {
        return run_secret_command(line.trim(), CMD_TIMEOUT);
    }
    if stored.is_empty() {
        return Err(SecretError::Empty);
    }
    Ok(SecretString::new(stored))
}

/// Which form a stored value uses, for the settings page (`来自环境变量 NAME` and so on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    Literal,
    Env(String),
    Command,
}

/// Classifies a stored value without reading the environment or running anything.
pub fn source_of(stored: &str) -> SecretSource {
    let stored = stored.trim();
    if let Some(name) = stored.strip_prefix("env:") {
        SecretSource::Env(name.trim().to_string())
    } else if stored.starts_with("cmd:") {
        SecretSource::Command
    } else {
        SecretSource::Literal
    }
}

fn resolve_env(name: &str) -> Result<SecretString, SecretError> {
    if name.is_empty() {
        return Err(SecretError::EnvMissing("(空)".to_string()));
    }
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(SecretString::new(v.trim())),
        Ok(_) => Err(SecretError::Empty),
        Err(std::env::VarError::NotPresent) => Err(SecretError::EnvMissing(name.to_string())),
        Err(std::env::VarError::NotUnicode(_)) => Err(SecretError::EnvNotUnicode(name.to_string())),
    }
}

/// Splits a command line into arguments the way a POSIX shell splits words, without expanding
/// anything: single quotes keep everything, double quotes keep everything but `\"` and `\\`,
/// and a bare backslash escapes the next character. Windows paths keep working because a
/// backslash is only an escape before a quote or another backslash.
pub fn split_command_line(line: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => return Err("缺少结束的单引号".to_string()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.peek() {
                            Some(&n) if n == '"' || n == '\\' => {
                                current.push(n);
                                chars.next();
                            }
                            _ => current.push('\\'),
                        },
                        Some(ch) => current.push(ch),
                        None => return Err("缺少结束的双引号".to_string()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.peek() {
                    Some(&n) if n.is_whitespace() || n == '"' || n == '\'' || n == '\\' => {
                        current.push(n);
                        chars.next();
                    }
                    _ => current.push('\\'),
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        args.push(current);
    }
    Ok(args)
}

/// Runs a `cmd:` line without a shell and returns its trimmed standard output.
pub fn run_secret_command(line: &str, timeout: Duration) -> Result<SecretString, SecretError> {
    if line.is_empty() {
        return Err(SecretError::CmdEmpty);
    }
    let argv = split_command_line(line).map_err(SecretError::CmdSplit)?;
    let (program, args) = argv.split_first().ok_or(SecretError::CmdEmpty)?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| SecretError::CmdSpawn(e.kind().to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| SecretError::CmdIo("no stdout".into()))?;
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SecretError::CmdTimeout);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(SecretError::CmdIo(e.kind().to_string())),
        }
    };
    let mut output = reader
        .join()
        .map_err(|_| SecretError::CmdIo("reader".into()))?;
    if !status.success() {
        output.zeroize();
        return Err(SecretError::CmdFailed(status.code()));
    }
    let text =
        String::from_utf8(output).map_err(|_| SecretError::CmdIo("output is not UTF-8".into()))?;
    let trimmed = text.trim().to_string();
    let mut text = text;
    text.zeroize();
    if trimmed.is_empty() {
        return Err(SecretError::CmdNoOutput);
    }
    Ok(SecretString::new(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_show_the_value() {
        let s = SecretString::new("sk-very-secret");
        assert_eq!(format!("{s:?}"), "SecretString(***)");
        assert_eq!(format!("{s}"), "***");
        assert!(!format!("{s:?} {s}").contains("sk-very"));
    }

    #[test]
    fn literal_values_resolve_to_themselves() {
        assert_eq!(
            resolve_secret("  sk-abc  ").expect("literal").expose(),
            "sk-abc"
        );
        assert_eq!(resolve_secret("   "), Err(SecretError::Empty));
    }

    #[test]
    fn env_values_are_read_at_use_time() {
        let name = format!("JIYUE_TEST_SECRET_{}", std::process::id());
        // SAFETY-free: the test sets and removes its own unique variable.
        unsafe { std::env::set_var(&name, "from-env") };
        let value = resolve_secret(&format!("env:{name}")).expect("set");
        assert_eq!(value.expose(), "from-env");
        unsafe { std::env::remove_var(&name) };
        assert_eq!(
            resolve_secret(&format!("env:{name}")),
            Err(SecretError::EnvMissing(name.clone()))
        );
        assert_eq!(source_of(&format!("env:{name}")), SecretSource::Env(name));
    }

    #[test]
    fn command_line_is_split_without_a_shell() {
        assert_eq!(
            split_command_line(r#"tool  --flag "a b" 'c d' e\ f"#).expect("split"),
            vec!["tool", "--flag", "a b", "c d", "e f"]
        );
        assert_eq!(
            split_command_line(r#""C:\Program Files\x.exe" --k "q\"x""#).expect("split"),
            vec![r"C:\Program Files\x.exe", "--k", "q\"x"]
        );
        assert_eq!(
            split_command_line(r"C:\tools\pass.exe show").expect("split")[0],
            r"C:\tools\pass.exe"
        );
        assert!(split_command_line("tool 'open").is_err());
        assert!(split_command_line("tool \"open").is_err());
        // A shell metacharacter is an ordinary character: no pipes, no expansion.
        assert_eq!(
            split_command_line("echo a|b $HOME").expect("split"),
            vec!["echo", "a|b", "$HOME"]
        );
    }

    #[test]
    fn empty_command_is_refused() {
        assert_eq!(
            run_secret_command("", CMD_TIMEOUT),
            Err(SecretError::CmdEmpty)
        );
        assert_eq!(
            run_secret_command("   ", CMD_TIMEOUT),
            Err(SecretError::CmdEmpty)
        );
    }

    /// Runs this test binary with `--list`, which prints test names and exits 0, so the output is
    /// non-empty and the secret is the first line that the harness prints.
    #[test]
    fn command_output_is_trimmed_and_used() {
        let exe = std::env::current_exe().expect("current exe");
        let line = format!("\"{}\" --list --format terse", exe.display());
        let secret = run_secret_command(&line, Duration::from_secs(20)).expect("command runs");
        assert!(!secret.expose().is_empty());
        assert_eq!(secret.expose(), secret.expose().trim());
        assert!(!format!("{secret:?}").contains(secret.expose()));
    }

    #[test]
    fn a_command_that_takes_too_long_is_stopped() {
        let line = if cfg!(windows) {
            "ping -n 30 127.0.0.1"
        } else {
            "sleep 30"
        };
        let started = Instant::now();
        let err = run_secret_command(line, Duration::from_millis(300)).expect_err("timeout");
        assert_eq!(err, SecretError::CmdTimeout);
        assert!(started.elapsed() < Duration::from_secs(15));
    }

    #[test]
    fn a_failing_command_names_the_exit_code_only() {
        let exe = std::env::current_exe().expect("current exe");
        let line = format!("\"{}\" --no-such-flag-for-jiyue-test", exe.display());
        let err = run_secret_command(&line, Duration::from_secs(20)).expect_err("fails");
        assert!(matches!(err, SecretError::CmdFailed(_)), "{err:?}");
    }
}
