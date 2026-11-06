//! Command line of `jiyue-mcp`:
//! `jiyue-mcp --vault <dir> [--read-only] [--allow-delete] [--max-read-bytes N]`.

use std::path::PathBuf;

/// Default cap on the bytes of note text returned by one tool call or resource read.
pub const DEFAULT_MAX_READ_BYTES: usize = 256 * 1024;
/// Smallest value accepted for `--max-read-bytes`.
pub const MIN_MAX_READ_BYTES: usize = 1024;
/// Largest value accepted for `--max-read-bytes`.
pub const MAX_MAX_READ_BYTES: usize = 8 * 1024 * 1024;

/// Usage text, printed for `--help` and after a command line error.
pub const USAGE: &str = "usage: jiyue-mcp --vault <dir> [--read-only] [--allow-delete] \
[--max-read-bytes N]";

/// Resolved settings for one server run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Vault root directory (canonicalized when the server opens it).
    pub vault: PathBuf,
    /// Disable every tool that writes.
    pub read_only: bool,
    /// Enable the `delete_note` tool (moves notes to `.trash`).
    pub allow_delete: bool,
    /// Cap on note text per response, in bytes.
    pub max_read_bytes: usize,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    Run(Config),
    Help,
}

/// Parses the arguments that follow the program name.
pub fn parse_args<I, S>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut vault: Option<PathBuf> = None;
    let mut read_only = false;
    let mut allow_delete = false;
    let mut max_read_bytes = DEFAULT_MAX_READ_BYTES;
    let mut iter = args.into_iter();

    while let Some(raw) = iter.next() {
        let arg = raw.as_ref();
        if let Some(value) = arg.strip_prefix("--vault=") {
            vault = Some(PathBuf::from(value));
            continue;
        }
        if let Some(value) = arg.strip_prefix("--max-read-bytes=") {
            max_read_bytes = parse_max_read_bytes(value)?;
            continue;
        }
        match arg {
            "--help" | "-h" => return Ok(Cli::Help),
            "--vault" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--vault needs a directory".to_string())?;
                vault = Some(PathBuf::from(value.as_ref()));
            }
            "--max-read-bytes" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--max-read-bytes needs a number".to_string())?;
                max_read_bytes = parse_max_read_bytes(value.as_ref())?;
            }
            "--read-only" => read_only = true,
            "--allow-delete" => allow_delete = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let vault = vault.ok_or_else(|| "missing --vault <dir>".to_string())?;
    if vault.as_os_str().is_empty() {
        return Err("--vault must not be empty".to_string());
    }
    if read_only && allow_delete {
        return Err("--allow-delete cannot be combined with --read-only".to_string());
    }
    Ok(Cli::Run(Config {
        vault,
        read_only,
        allow_delete,
        max_read_bytes,
    }))
}

fn parse_max_read_bytes(text: &str) -> Result<usize, String> {
    let value: usize = text
        .trim()
        .parse()
        .map_err(|_| format!("--max-read-bytes expects a number, got {text:?}"))?;
    if !(MIN_MAX_READ_BYTES..=MAX_MAX_READ_BYTES).contains(&value) {
        return Err(format!(
            "--max-read-bytes must be between {MIN_MAX_READ_BYTES} and {MAX_MAX_READ_BYTES}"
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().copied())
    }

    fn config(args: &[&str]) -> Config {
        match run(args) {
            Ok(Cli::Run(config)) => config,
            other => panic!("expected a run configuration, got {other:?}"),
        }
    }

    #[test]
    fn minimal_command_line_uses_defaults() {
        let c = config(&["--vault", "D:/notes"]);
        assert_eq!(c.vault, PathBuf::from("D:/notes"));
        assert!(!c.read_only);
        assert!(!c.allow_delete);
        assert_eq!(c.max_read_bytes, DEFAULT_MAX_READ_BYTES);
    }

    #[test]
    fn equals_form_and_flags_are_parsed() {
        let c = config(&["--vault=/x", "--read-only", "--max-read-bytes=4096"]);
        assert_eq!(c.vault, PathBuf::from("/x"));
        assert!(c.read_only);
        assert_eq!(c.max_read_bytes, 4096);
    }

    #[test]
    fn allow_delete_flag_is_parsed() {
        let c = config(&["--vault", "v", "--allow-delete"]);
        assert!(c.allow_delete);
        assert!(!c.read_only);
    }

    #[test]
    fn max_read_bytes_limits_are_enforced() {
        assert!(run(&["--vault", "v", "--max-read-bytes", "10"]).is_err());
        assert!(run(&["--vault", "v", "--max-read-bytes", "99999999999"]).is_err());
        assert!(run(&["--vault", "v", "--max-read-bytes", "lots"]).is_err());
        assert!(run(&["--vault", "v", "--max-read-bytes"]).is_err());
        assert!(run(&["--vault", "v", "--max-read-bytes", "1024"]).is_ok());
    }

    #[test]
    fn missing_vault_and_unknown_flags_are_errors() {
        assert!(run(&["--read-only"]).is_err());
        assert!(run(&["--vault"]).is_err());
        assert!(run(&["--vault", ""]).is_err());
        assert!(run(&["--vault", "v", "--bogus"]).is_err());
    }

    #[test]
    fn read_only_and_allow_delete_conflict() {
        let err = run(&["--vault", "v", "--read-only", "--allow-delete"]).err();
        assert!(err.is_some_and(|e| e.contains("cannot be combined")));
    }

    #[test]
    fn help_is_recognised() {
        assert_eq!(run(&["--help"]), Ok(Cli::Help));
        assert_eq!(run(&["-h"]), Ok(Cli::Help));
    }
}
