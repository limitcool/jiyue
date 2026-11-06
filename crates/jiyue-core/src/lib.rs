//! jiyue-core: note-library logic with no GPUI dependency.
//!
//! Modules: text buffer, daily notes, editor state, markdown editing helpers and the vault
//! (note library, notes and link graph).

pub mod buffer;
pub mod config;
pub mod daily;
pub mod editor_state;
pub mod fsops;
pub mod links;
pub mod md_edit;
pub mod platform;
pub mod recovery;
pub mod vault;
pub mod watch;
