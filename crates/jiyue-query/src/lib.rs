//! jiyue-query: a small "database over notes".
//!
//! Notes are Markdown files. Their frontmatter properties and a few built-in fields
//! (name, folder, tags, times, size, links, backlinks) form one record per note in a
//! [`PropertyIndex`]. A query written by the user selects, filters, sorts, groups and
//! limits those records and returns a [`QueryResult`] for a table, list or card view.
//!
//! ```text
//! from folder("项目") and tag("rust")
//! where status = "进行中" and due < today() + 7d
//! select name, status as 状态, due, count(backlinks)
//! sort due asc, mtime desc
//! group by status
//! limit 50
//! view table
//! ```
//!
//! Pure logic: no GPUI and no file access. The caller reads files, passes their text to
//! [`PropertyIndex::upsert`], and supplies the clock through [`QueryContext`].
//!
//! Frontmatter is read and edited by `jiyue-md`. Edits go through
//! [`plan_cell_edit`], which rewrites only the edited key's line.

pub mod ast;
pub mod cell;
pub mod date;
pub mod edit;
pub mod error;
pub mod exec;
pub mod index;
pub mod parser;
pub mod result;
pub mod suggest;

mod eval;
mod lexer;

pub use ast::{AggFn, BinOp, Expr, Func, Query, SelectItem, SortItem, Source, ViewKind};
pub use cell::{Cell, Kind};
pub use date::Duration;
pub use edit::{parse_input, plan_cell_edit, property_types};
pub use error::ParseError;
pub use exec::{QueryContext, execute, execute_with_resolver, run_text};
pub use index::{NoteRecord, PropertyIndex, normalize_path, stem_of};
pub use parser::parse;
pub use result::{Column, Group, QueryResult, Row};
pub use suggest::{Completion, CompletionKind, suggest};
