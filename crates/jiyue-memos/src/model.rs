//! Data models for Memos and Todo items parsed from content.

use serde::{Deserialize, Serialize};

/// Represents a single todo task extracted from Markdown content (e.g. `- [ ] Task` or `- [x] Task`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    /// Zero-based line index in the memo content.
    pub line_index: usize,
    /// The task description text (stripped of `- [ ] ` or `- [x] `).
    pub text: String,
    /// Whether the task is checked/completed.
    pub done: bool,
}

/// A memo retrieved from the Memos API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memo {
    /// Resource name in Memos v1 API (e.g. `memos/123`).
    pub name: String,
    /// Plain unique identifier or suffix (e.g. `123`).
    pub id: String,
    /// Full markdown content of the memo.
    pub content: String,
    /// Creation timestamp (ISO-8601 string).
    #[serde(default)]
    pub create_time: Option<String>,
    /// Update timestamp (ISO-8601 string).
    #[serde(default)]
    pub update_time: Option<String>,
    /// Visibility (e.g. "PRIVATE", "PUBLIC", "PROTECTED").
    #[serde(default)]
    pub visibility: String,
    /// Extracted tags (e.g. `todo`, `idea`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Extracted todo items from content.
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    /// Pinned flag.
    #[serde(default)]
    pub pinned: bool,
}

impl Memo {
    /// Creates a Memo and parses its tags and todo tasks from `content`.
    pub fn from_raw(
        name: String,
        content: String,
        create_time: Option<String>,
        update_time: Option<String>,
        visibility: Option<String>,
        pinned: bool,
    ) -> Self {
        let id = name.strip_prefix("memos/").unwrap_or(&name).to_string();
        let tags = extract_tags(&content);
        let todos = extract_todos(&content);

        Self {
            name,
            id,
            content,
            create_time,
            update_time,
            visibility: visibility.unwrap_or_else(|| "PRIVATE".to_string()),
            tags,
            todos,
            pinned,
        }
    }

    /// Checks if this memo contains any todo items.
    pub fn has_todos(&self) -> bool {
        !self.todos.is_empty()
    }

    /// Checks if all todo items in this memo are completed.
    pub fn is_all_todos_done(&self) -> bool {
        !self.todos.is_empty() && self.todos.iter().all(|t| t.done)
    }

    /// Toggles the todo item at `line_index`, returning the new content if changed.
    pub fn toggle_todo_content(&self, line_index: usize) -> Option<String> {
        toggle_markdown_todo(&self.content, line_index)
    }
}

/// Extracts hashtags (e.g. `#todo`, `#work/project`) from markdown content.
pub fn extract_tags(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for word in content.split_whitespace() {
        if let Some(tag) = word.strip_prefix('#') {
            let clean = tag
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '_' && c != '-');
            if !clean.is_empty() && !tags.contains(&clean.to_string()) {
                tags.push(clean.to_string());
            }
        }
    }
    tags
}

/// Extracts `- [ ]` and `- [x]` items along with their zero-based line indices.
pub fn extract_todos(content: &str) -> Vec<TodoItem> {
    let mut todos = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            todos.push(TodoItem {
                line_index: idx,
                text: rest.trim().to_string(),
                done: false,
            });
        } else if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
        {
            todos.push(TodoItem {
                line_index: idx,
                text: rest.trim().to_string(),
                done: true,
            });
        }
    }
    todos
}

/// Toggles the checkbox state on line `target_line` in `content`.
pub fn toggle_markdown_todo(content: &str, target_line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if target_line >= lines.len() {
        return None;
    }

    let line = lines[target_line];
    let indent = line.len() - line.trim_start().len();
    let prefix_spaces = &line[..indent];
    let trimmed = &line[indent..];

    let new_line = if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        format!("{}- [x] {}", prefix_spaces, rest)
    } else {
        let rest = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))?;
        format!("{}- [ ] {}", prefix_spaces, rest)
    };

    let mut result = String::with_capacity(content.len() + 8);
    for (i, l) in lines.iter().enumerate() {
        if i == target_line {
            result.push_str(&new_line);
        } else {
            result.push_str(l);
        }
        if i + 1 < lines.len() || content.ends_with('\n') {
            result.push('\n');
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_todos_and_tags() {
        let text = "#inbox\nHere is a note\n- [ ] First task\n- [x] Done task\n#todo/work";
        let tags = extract_tags(text);
        assert_eq!(tags, vec!["inbox", "todo/work"]);

        let todos = extract_todos(text);
        assert_eq!(todos.len(), 2);
        assert_eq!(
            todos[0],
            TodoItem {
                line_index: 2,
                text: "First task".to_string(),
                done: false,
            }
        );
        assert_eq!(
            todos[1],
            TodoItem {
                line_index: 3,
                text: "Done task".to_string(),
                done: true,
            }
        );
    }

    #[test]
    fn test_toggle_markdown_todo() {
        let text = "- [ ] Task 1\n- [x] Task 2\nNormal text";
        let toggled = toggle_markdown_todo(text, 0).expect("toggle line 0");
        assert_eq!(toggled, "- [x] Task 1\n- [x] Task 2\nNormal text");

        let toggled_back = toggle_markdown_todo(&toggled, 0).expect("toggle line 0 back");
        assert_eq!(toggled_back, "- [ ] Task 1\n- [x] Task 2\nNormal text");

        let toggled2 = toggle_markdown_todo(text, 1).expect("toggle line 1");
        assert_eq!(toggled2, "- [ ] Task 1\n- [ ] Task 2\nNormal text");
    }
}
