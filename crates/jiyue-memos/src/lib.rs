//! Memos integration library for Jiyue (霁月).
//!
//! Provides API client for Memos v1 REST service, Todo/Tag extraction,
//! and markdown task state manipulation.

pub mod client;
pub mod config;
pub mod model;

pub use client::{MemosClient, MemosError};
pub use config::MemosConfig;
pub use model::{Memo, TodoItem, extract_tags, extract_todos, toggle_markdown_todo};
