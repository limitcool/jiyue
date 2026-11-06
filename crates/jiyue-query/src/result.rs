//! Result model for a query, ready for a table, list or card view.

use serde::Serialize;

use crate::ast::ViewKind;
use crate::cell::{Cell, Kind};

/// One output column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Column {
    /// Alias, or the expression as written.
    pub name: String,
    /// Dominant type of the values in the column.
    pub kind: Kind,
    /// True for aggregate columns (`sum(size)`): their values live in `Group::aggregates`.
    pub aggregate: bool,
}

/// One note in the output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub path: String,
    /// One cell per column. Aggregate columns hold `Null` here.
    pub cells: Vec<Cell>,
}

/// A run of rows with the same group key. Without `group by` there is one group with a
/// null key holding all rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    pub key: Cell,
    pub rows: Vec<Row>,
    /// One value per column: the aggregate for aggregate columns, `Null` otherwise.
    pub aggregates: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub groups: Vec<Group>,
    pub view: ViewKind,
    /// Notes that matched the source and filter, before `limit`.
    pub total: usize,
    /// Rows in the output after `limit`.
    pub shown: usize,
    pub elapsed_us: u64,
    /// Non-fatal issues, such as a property that no note has.
    pub warnings: Vec<String>,
}

impl QueryResult {
    /// Rows of all groups, in output order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.groups.iter().flat_map(|g| g.rows.iter())
    }
}
