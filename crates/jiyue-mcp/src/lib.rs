//! jiyue-mcp: a local MCP (Model Context Protocol) server over stdio. It lets AI agents read
//! and, unless started with `--read-only`, edit the notes of one Jiyue vault.
//!
//! Modules: `protocol` (JSON-RPC framing), `server` (method dispatch and the serve loop),
//! `tools` (tool schemas and handlers), `resources` (`jiyue://note/...`), `catalog` (in-memory
//! search index and link graph), `vault` (the path boundary), `analysis` (per-note facts),
//! `config` (command line), `app` (the state shared by all handlers).
//!
//! Stdout carries protocol messages only. Logs go to stderr.

pub mod analysis;
pub mod app;
pub mod catalog;
pub mod config;
pub mod protocol;
pub mod resources;
pub mod server;
pub mod tools;
pub mod vault;

#[cfg(test)]
mod testutil;
