//! `jiyue-mcp`: a Model Context Protocol server over stdio for one Jiyue vault.
//!
//! Usage: `jiyue-mcp --vault <dir> [--read-only] [--allow-delete] [--max-read-bytes N]`.
//! Stdout carries protocol messages only. Logs go to stderr.

use std::io::{self, BufReader};
use std::process::ExitCode;

use jiyue_mcp::app::App;
use jiyue_mcp::config::{Cli, USAGE, parse_args};
use jiyue_mcp::server::{Server, serve};

fn main() -> ExitCode {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("jiyue-mcp: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = match cli {
        Cli::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Run(config) => config,
    };

    let mode = match (config.read_only, config.allow_delete) {
        (true, _) => "read-only",
        (false, true) => "read-write, delete enabled",
        (false, false) => "read-write",
    };
    let app = match App::open(config) {
        Ok(app) => app,
        Err(message) => {
            eprintln!("jiyue-mcp: {message}");
            return ExitCode::from(1);
        }
    };
    let shown = app.root.path().display().to_string();
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown);
    eprintln!(
        "jiyue-mcp: vault {shown} ({} notes, {mode})",
        app.catalog.len()
    );

    let mut server = Server::new(app);
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    match serve(&mut server, BufReader::new(stdin.lock()), &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("jiyue-mcp: stdio error: {err}");
            ExitCode::from(1)
        }
    }
}
