//! Pure bookmark logic (no GPUI): the file format of `.jiyue/bookmarks.json`, the tree edits,
//! remapping after renames and moves, heading lookup and the picker filter. Unit tested.

/// Version of the bookmarks file.
pub const VERSION: u64 = 1;
/// Groups may nest one level: a group inside a top-level group, and no deeper.
pub const MAX_GROUP_DEPTH: usize = 2;

/// What a bookmark points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A note (vault-relative path).
    Note { path: String },
    /// A heading of a note: the note's path, the heading text and its level (1 to 6).
    Heading {
        path: String,
        text: String,
        level: u8,
    },
    /// A saved search (the query of the search panel).
    Search { query: String },
    /// A folder (folder key, vault-relative).
    Folder { path: String },
    /// A named group of bookmarks.
    Group { items: Vec<Bookmark> },
}

/// One entry of the bookmarks list. `title` is the custom title (for a group, its name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub id: String,
    pub title: Option<String>,
    pub kind: Kind,
}

impl Bookmark {
    pub fn is_group(&self) -> bool {
        matches!(self.kind, Kind::Group { .. })
    }
}

/// Builds an id from the time and a sequence number (unique within a run).
pub fn new_id(now_ms: u64, seq: u64) -> String {
    format!("{now_ms:x}-{seq:x}")
}

// ---------------------------------------------------------------------------
// File format
// ---------------------------------------------------------------------------

/// On-disk shape of `bookmarks.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BookmarksDoc {
    pub version: u32,
    pub items: Vec<StoredBookmark>,
}

/// One entry as written to `bookmarks.toml`. `kind` names the variant; the other fields belong to
/// some kinds only. Groups hold their entries in `items`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StoredBookmark {
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub path: String,
    pub text: String,
    pub level: u8,
    pub query: String,
    pub items: Vec<StoredBookmark>,
}

/// Turns a loaded document into bookmarks. Entries that cannot be read are skipped, duplicate ids
/// are dropped (the first one wins), and groups nested deeper than `MAX_GROUP_DEPTH` are dropped.
pub fn from_doc(doc: BookmarksDoc) -> Vec<Bookmark> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    convert_items(&doc.items, 0, &mut seen, &mut out);
    out
}

fn convert_items(
    items: &[StoredBookmark],
    depth: usize,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<Bookmark>,
) {
    for stored in items {
        if stored.id.is_empty() || !seen.insert(stored.id.clone()) {
            continue;
        }
        let kind = match stored.kind.as_str() {
            "note" if !stored.path.is_empty() => Kind::Note {
                path: stored.path.clone(),
            },
            "heading" if !stored.path.is_empty() && (1..=6).contains(&stored.level) => {
                Kind::Heading {
                    path: stored.path.clone(),
                    text: stored.text.clone(),
                    level: stored.level,
                }
            }
            "search" if !stored.query.is_empty() => Kind::Search {
                query: stored.query.clone(),
            },
            "folder" if !stored.path.is_empty() => Kind::Folder {
                path: stored.path.clone(),
            },
            "group" if depth < MAX_GROUP_DEPTH => {
                let mut children = Vec::new();
                convert_items(&stored.items, depth + 1, seen, &mut children);
                Kind::Group { items: children }
            }
            _ => continue,
        };
        out.push(Bookmark {
            id: stored.id.clone(),
            title: stored.title.clone(),
            kind,
        });
    }
}

/// The document for `items`, ready to be written as `bookmarks.toml`.
pub fn to_doc(items: &[Bookmark]) -> BookmarksDoc {
    BookmarksDoc {
        version: VERSION as u32,
        items: items.iter().map(to_stored).collect(),
    }
}

fn to_stored(b: &Bookmark) -> StoredBookmark {
    let mut out = StoredBookmark {
        id: b.id.clone(),
        title: b.title.clone(),
        ..StoredBookmark::default()
    };
    match &b.kind {
        Kind::Note { path } => {
            out.kind = "note".into();
            out.path = path.clone();
        }
        Kind::Heading { path, text, level } => {
            out.kind = "heading".into();
            out.path = path.clone();
            out.text = text.clone();
            out.level = *level;
        }
        Kind::Search { query } => {
            out.kind = "search".into();
            out.query = query.clone();
        }
        Kind::Folder { path } => {
            out.kind = "folder".into();
            out.path = path.clone();
        }
        Kind::Group { items } => {
            out.kind = "group".into();
            out.items = items.iter().map(to_stored).collect();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// The bookmark with `id`, at any depth.
pub fn find<'a>(items: &'a [Bookmark], id: &str) -> Option<&'a Bookmark> {
    for b in items {
        if b.id == id {
            return Some(b);
        }
        if let Kind::Group { items: kids } = &b.kind
            && let Some(found) = find(kids, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_mut<'a>(items: &'a mut [Bookmark], id: &str) -> Option<&'a mut Bookmark> {
    for b in items.iter_mut() {
        if b.id == id {
            return Some(b);
        }
        if let Kind::Group { items: kids } = &mut b.kind
            && let Some(found) = find_mut(kids, id)
        {
            return Some(found);
        }
    }
    None
}

/// The list that holds the bookmark `id` (the root list or a group's list).
fn container_mut<'a>(items: &'a mut Vec<Bookmark>, id: &str) -> Option<&'a mut Vec<Bookmark>> {
    if items.iter().any(|b| b.id == id) {
        return Some(items);
    }
    for b in items.iter_mut() {
        if let Kind::Group { items: kids } = &mut b.kind
            && let Some(list) = container_mut(kids, id)
        {
            return Some(list);
        }
    }
    None
}

/// Nesting level of the list that holds `id`: 0 for the root list, 1 inside a top-level group.
fn container_level(items: &[Bookmark], id: &str, level: usize) -> Option<usize> {
    if items.iter().any(|b| b.id == id) {
        return Some(level);
    }
    for b in items {
        if let Kind::Group { items: kids } = &b.kind
            && let Some(found) = container_level(kids, id, level + 1)
        {
            return Some(found);
        }
    }
    None
}

/// True when `id` is `root` itself or lies inside it.
fn subtree_has(root: &Bookmark, id: &str) -> bool {
    if root.id == id {
        return true;
    }
    match &root.kind {
        Kind::Group { items } => items.iter().any(|b| subtree_has(b, id)),
        _ => false,
    }
}

/// How many group levels a bookmark brings with it: 0 for an entry, 1 for a group without
/// subgroups, 2 for a group that holds one group, and so on.
pub fn group_depth(b: &Bookmark) -> usize {
    match &b.kind {
        Kind::Group { items } => {
            1 + items
                .iter()
                .filter(|k| k.is_group())
                .map(group_depth)
                .max()
                .unwrap_or(0)
        }
        _ => 0,
    }
}

/// Identity of a bookmark's target: two entries with the same key point at the same thing.
/// Groups have no key.
fn key_of(kind: &Kind) -> Option<String> {
    match kind {
        Kind::Note { path } => Some(format!("note:{path}")),
        Kind::Heading { path, text, .. } => Some(format!("heading:{path}#{text}")),
        Kind::Search { query } => Some(format!("search:{}", query.trim())),
        Kind::Folder { path } => Some(format!("folder:{path}")),
        Kind::Group { .. } => None,
    }
}

/// Ids of the non-group entries with the same key as `kind`, at any depth.
fn ids_with_key(items: &[Bookmark], kind: &Kind) -> Vec<String> {
    let Some(key) = key_of(kind) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_with_key(items, &key, &mut out);
    out
}

fn collect_with_key(items: &[Bookmark], key: &str, out: &mut Vec<String>) {
    for b in items {
        match &b.kind {
            Kind::Group { items: kids } => collect_with_key(kids, key, out),
            kind => {
                if key_of(kind).as_deref() == Some(key) {
                    out.push(b.id.clone());
                }
            }
        }
    }
}

/// True when an entry with the same target is already bookmarked.
pub fn has_duplicate(items: &[Bookmark], kind: &Kind) -> bool {
    !ids_with_key(items, kind).is_empty()
}

/// Number of entries (groups not counted), at any depth.
pub fn entry_count(items: &[Bookmark]) -> usize {
    items
        .iter()
        .map(|b| match &b.kind {
            Kind::Group { items: kids } => entry_count(kids),
            _ => 1,
        })
        .sum()
}

/// The bookmark of a note path, if any (non-group, any depth).
pub fn note_bookmarked(items: &[Bookmark], path: &str) -> bool {
    has_duplicate(
        items,
        &Kind::Note {
            path: path.to_string(),
        },
    )
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

/// Removes the bookmark `id` (with its children when it is a group). True when it was found.
pub fn remove(items: &mut Vec<Bookmark>, id: &str) -> bool {
    if let Some(pos) = items.iter().position(|b| b.id == id) {
        items.remove(pos);
        return true;
    }
    for b in items.iter_mut() {
        if let Kind::Group { items: kids } = &mut b.kind
            && remove(kids, id)
        {
            return true;
        }
    }
    false
}

/// Removes the bookmark `id` and returns it.
fn take(items: &mut Vec<Bookmark>, id: &str) -> Option<Bookmark> {
    if let Some(pos) = items.iter().position(|b| b.id == id) {
        return Some(items.remove(pos));
    }
    for b in items.iter_mut() {
        if let Kind::Group { items: kids } = &mut b.kind
            && let Some(found) = take(kids, id)
        {
            return Some(found);
        }
    }
    None
}

/// Sets the custom title; an empty or blank title clears it. True when the bookmark exists.
pub fn set_title(items: &mut [Bookmark], id: &str, title: &str) -> bool {
    let Some(b) = find_mut(items, id) else {
        return false;
    };
    let t = title.trim();
    b.title = if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    };
    true
}

/// Adds an entry at the end of the root list unless one with the same target exists. Returns
/// false for a duplicate (nothing changes).
pub fn add_unique(items: &mut Vec<Bookmark>, kind: Kind, id: String) -> bool {
    if has_duplicate(items, &kind) {
        return false;
    }
    items.push(Bookmark {
        id,
        title: None,
        kind,
    });
    true
}

/// Result of `toggle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Added,
    Removed,
}

/// Adds the entry when no entry has its target, otherwise removes every entry with that target.
pub fn toggle(items: &mut Vec<Bookmark>, kind: Kind, id: String) -> Toggle {
    let existing = ids_with_key(items, &kind);
    if existing.is_empty() {
        items.push(Bookmark {
            id,
            title: None,
            kind,
        });
        return Toggle::Added;
    }
    for old in existing {
        take(items, &old);
    }
    Toggle::Removed
}

/// Creates a top-level group. Returns its id.
pub fn add_group(items: &mut Vec<Bookmark>, name: &str, id: String) -> String {
    let t = name.trim();
    items.push(Bookmark {
        id: id.clone(),
        title: if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        },
        kind: Kind::Group { items: Vec::new() },
    });
    id
}

/// Moves the bookmark one step up (`delta` -1) or down (`delta` +1) inside its list. False at the
/// ends of the list.
pub fn move_step(items: &mut Vec<Bookmark>, id: &str, delta: isize) -> bool {
    let Some(list) = container_mut(items, id) else {
        return false;
    };
    let Some(pos) = list.iter().position(|b| b.id == id) else {
        return false;
    };
    let next = pos as isize + delta;
    if next < 0 || next >= list.len() as isize {
        return false;
    }
    list.swap(pos, next as usize);
    true
}

/// Whether `dragged` may be placed into a list at nesting `dest_level` (0 root).
fn fits(dragged: &Bookmark, dest_level: usize) -> bool {
    dest_level + group_depth(dragged) <= MAX_GROUP_DEPTH
}

/// True when `dragged` can be moved into the group `group` (not into itself or its own
/// subgroups, and the nesting stays within `MAX_GROUP_DEPTH`).
pub fn can_move_into(items: &[Bookmark], dragged: &str, group: &str) -> bool {
    let Some(g) = find(items, group) else {
        return false;
    };
    let Some(src) = find(items, dragged) else {
        return false;
    };
    if !g.is_group() || subtree_has(src, group) {
        return false;
    }
    // Items inside a group sit one level below it.
    let Some(level) = container_level(items, group, 0) else {
        return false;
    };
    fits(src, level + 1)
}

/// Moves `dragged` to the end of the group `group`. False when `can_move_into` says no.
pub fn move_into_group(items: &mut Vec<Bookmark>, dragged: &str, group: &str) -> bool {
    if !can_move_into(items, dragged, group) {
        return false;
    }
    let Some(item) = take(items, dragged) else {
        return false;
    };
    match find_mut(items, group).map(|b| &mut b.kind) {
        Some(Kind::Group { items: kids }) => {
            kids.push(item);
            true
        }
        _ => {
            // Cannot happen after `can_move_into`; keep the entry rather than lose it.
            items.push(item);
            false
        }
    }
}

/// Moves `dragged` out of its group to the end of the root list.
pub fn move_to_root(items: &mut Vec<Bookmark>, dragged: &str) -> bool {
    let Some(src) = find(items, dragged) else {
        return false;
    };
    if !fits(src, 0) {
        return false;
    }
    match take(items, dragged) {
        Some(item) => {
            items.push(item);
            true
        }
        None => false,
    }
}

/// Moves `dragged` into the list of `target`, just before `target`. False for an unknown id, a
/// drop on itself, or a nesting that does not fit.
pub fn move_before(items: &mut Vec<Bookmark>, dragged: &str, target: &str) -> bool {
    if dragged == target {
        return false;
    }
    let (Some(src), Some(level)) = (find(items, dragged), container_level(items, target, 0)) else {
        return false;
    };
    if subtree_has(src, target) || !fits(src, level) {
        return false;
    }
    let Some(item) = take(items, dragged) else {
        return false;
    };
    let Some(list) = container_mut(items, target) else {
        items.push(item);
        return false;
    };
    let Some(pos) = list.iter().position(|b| b.id == target) else {
        list.push(item);
        return false;
    };
    list.insert(pos, item);
    true
}

/// Position of `id` in its list (0 based) and the length of that list.
pub fn position_in_list(items: &[Bookmark], id: &str) -> Option<(usize, usize)> {
    if let Some(pos) = items.iter().position(|b| b.id == id) {
        return Some((pos, items.len()));
    }
    for b in items {
        if let Kind::Group { items: kids } = &b.kind
            && let Some(found) = position_in_list(kids, id)
        {
            return Some(found);
        }
    }
    None
}

/// Replaces the group `gid` by its children, in its place. False when `gid` is not a group.
pub fn ungroup(items: &mut Vec<Bookmark>, gid: &str) -> bool {
    if let Some(pos) = items.iter().position(|b| b.id == gid) {
        if !items[pos].is_group() {
            return false;
        }
        let group = items.remove(pos);
        if let Kind::Group { items: kids } = group.kind {
            for (i, kid) in kids.into_iter().enumerate() {
                items.insert(pos + i, kid);
            }
        }
        return true;
    }
    for b in items.iter_mut() {
        if let Kind::Group { items: kids } = &mut b.kind
            && ungroup(kids, gid)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Renames, moves and deletions
// ---------------------------------------------------------------------------

/// Where a vault path went: an exact rename pair first, then a moved folder (`from` to `to`) that
/// contains it. `None` when the path does not change.
pub fn map_path(
    path: &str,
    renames: &[(String, String)],
    moved: Option<(&str, &str)>,
) -> Option<String> {
    if let Some((_, new)) = renames.iter().find(|(old, _)| old == path) {
        return if new == path { None } else { Some(new.clone()) };
    }
    let (from, to) = moved?;
    if from.is_empty() || from == to {
        return None;
    }
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(&format!("{from}/"))
        .map(|rest| format!("{to}/{rest}"))
}

/// Updates every path of the bookmarks after a rename or a move. True when something changed.
pub fn remap(
    items: &mut [Bookmark],
    renames: &[(String, String)],
    moved: Option<(&str, &str)>,
) -> bool {
    let mut changed = false;
    for b in items.iter_mut() {
        match &mut b.kind {
            Kind::Note { path } | Kind::Heading { path, .. } | Kind::Folder { path } => {
                if let Some(new) = map_path(path, renames, moved) {
                    *path = new;
                    changed = true;
                }
            }
            Kind::Search { .. } => {}
            Kind::Group { items: kids } => {
                if remap(kids, renames, moved) {
                    changed = true;
                }
            }
        }
    }
    changed
}

/// True when the target of a bookmark is gone from the vault. Searches never are.
pub fn is_missing(
    kind: &Kind,
    has_note: &dyn Fn(&str) -> bool,
    has_folder: &dyn Fn(&str) -> bool,
) -> bool {
    match kind {
        Kind::Note { path } | Kind::Heading { path, .. } => !has_note(path),
        Kind::Folder { path } => !has_folder(path),
        Kind::Search { .. } | Kind::Group { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Last segment of a vault-relative path, without the `.md` extension.
pub fn stem_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// The title shown for a bookmark: the custom title, else the note's title (`note_title`), the
/// heading text, the search query, or the folder name.
pub fn display_title(b: &Bookmark, note_title: &dyn Fn(&str) -> Option<String>) -> String {
    if let Some(t) = b.title.as_ref().filter(|t| !t.trim().is_empty()) {
        return t.clone();
    }
    match &b.kind {
        Kind::Note { path } => note_title(path).unwrap_or_else(|| stem_of(path)),
        Kind::Heading { text, .. } => text.clone(),
        Kind::Search { query } => format!("搜索：{query}"),
        Kind::Folder { path } => path.rsplit('/').next().unwrap_or(path).to_string(),
        Kind::Group { .. } => "未命名分组".to_string(),
    }
}

/// One visible row of the bookmarks section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub depth: usize,
    pub is_group: bool,
}

/// The rows the section shows, in order. The children of a group are listed only when the group's
/// id is not in `collapsed`.
pub fn visible(items: &[Bookmark], collapsed: &[String]) -> Vec<Row> {
    let mut out = Vec::new();
    push_rows(items, 0, collapsed, &mut out);
    out
}

fn push_rows(items: &[Bookmark], depth: usize, collapsed: &[String], out: &mut Vec<Row>) {
    for b in items {
        out.push(Row {
            id: b.id.clone(),
            depth,
            is_group: b.is_group(),
        });
        if let Kind::Group { items: kids } = &b.kind
            && !collapsed.contains(&b.id)
        {
            push_rows(kids, depth + 1, collapsed, out);
        }
    }
}

/// A group offered as a destination in the 移到分组 picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupChoice {
    pub id: String,
    /// Names from the top group down, joined with ` / `.
    pub path: String,
}

/// Every group, in order, with its path of names. Groups inside `exclude` (and `exclude`
/// itself) are left out.
pub fn group_choices(items: &[Bookmark], exclude: Option<&str>) -> Vec<GroupChoice> {
    let mut out = Vec::new();
    collect_groups(items, "", exclude, &mut out);
    out
}

fn collect_groups(
    items: &[Bookmark],
    prefix: &str,
    exclude: Option<&str>,
    out: &mut Vec<GroupChoice>,
) {
    for b in items {
        if let Kind::Group { items: kids } = &b.kind {
            if Some(b.id.as_str()) == exclude {
                continue;
            }
            let name = b.title.clone().unwrap_or_else(|| "未命名分组".to_string());
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix} / {name}")
            };
            out.push(GroupChoice {
                id: b.id.clone(),
                path: path.clone(),
            });
            collect_groups(kids, &path, exclude, out);
        }
    }
}

// ---------------------------------------------------------------------------
// Picker
// ---------------------------------------------------------------------------

/// One row of the 打开书签 picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRow {
    pub id: String,
    pub label: String,
    /// Group path and target, for example `工作 / 想法/a.md`.
    pub sub: String,
}

/// Every entry (groups excluded) as a picker row.
pub fn picker_rows(
    items: &[Bookmark],
    note_title: &dyn Fn(&str) -> Option<String>,
) -> Vec<PickRow> {
    let mut out = Vec::new();
    collect_picker(items, "", note_title, &mut out);
    out
}

fn collect_picker(
    items: &[Bookmark],
    prefix: &str,
    note_title: &dyn Fn(&str) -> Option<String>,
    out: &mut Vec<PickRow>,
) {
    for b in items {
        if let Kind::Group { items: kids } = &b.kind {
            let name = b.title.clone().unwrap_or_else(|| "未命名分组".to_string());
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix} / {name}")
            };
            collect_picker(kids, &path, note_title, out);
            continue;
        }
        let target = match &b.kind {
            Kind::Note { path } | Kind::Heading { path, .. } => path.clone(),
            Kind::Folder { path } => format!("文件夹 {path}"),
            Kind::Search { .. } => "保存的搜索".to_string(),
            Kind::Group { .. } => String::new(),
        };
        let sub = if prefix.is_empty() {
            target
        } else {
            format!("{prefix} · {target}")
        };
        out.push(PickRow {
            id: b.id.clone(),
            label: display_title(b, note_title),
            sub,
        });
    }
}

/// Indexes of the rows that match `query`: every whitespace-separated term must appear in the
/// label or the sub line (case-insensitive). An empty query matches all rows.
pub fn picker_matches(rows: &[PickRow], query: &str) -> Vec<usize> {
    let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    rows.iter()
        .enumerate()
        .filter(|(_, r)| {
            let hay = format!("{} {}", r.label, r.sub).to_lowercase();
            terms.iter().all(|t| hay.contains(t))
        })
        .map(|(i, _)| i)
        .collect()
}

// ---------------------------------------------------------------------------
// Headings
// ---------------------------------------------------------------------------

/// Heading level and text of a Markdown ATX heading line (`## Title`), or `None`.
pub fn parse_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let text = rest.trim();
    // A closing run of `#` counts only after a space: `## C#` keeps its `#`.
    let stripped = text.trim_end_matches('#');
    let text = if stripped.len() < text.len()
        && (stripped.is_empty() || stripped.ends_with(' ') || stripped.ends_with('\t'))
    {
        stripped.trim()
    } else {
        text
    };
    if text.is_empty() {
        return None;
    }
    Some((hashes as u8, text.to_string()))
}

/// Lines of `content` with a flag telling whether each is inside a fenced code block.
fn heading_lines(content: &str) -> Vec<(usize, &str, bool)> {
    let mut fence = false;
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            out.push((i, line, true));
            continue;
        }
        out.push((i, line, fence));
    }
    out
}

/// The nearest heading at or above the 1-based line `line_no`, outside code fences.
pub fn heading_at_line(content: &str, line_no: usize) -> Option<(u8, String)> {
    let mut found = None;
    for (i, line, in_code) in heading_lines(content) {
        if i + 1 > line_no {
            break;
        }
        if in_code {
            continue;
        }
        if let Some(h) = parse_heading(line) {
            found = Some(h);
        }
    }
    found
}

/// The 0-based row of the heading with this text (and level when one matches exactly), or `None`.
pub fn heading_row(content: &str, text: &str, level: u8) -> Option<usize> {
    let mut loose = None;
    for (i, line, in_code) in heading_lines(content) {
        if in_code {
            continue;
        }
        if let Some((lvl, t)) = parse_heading(line)
            && t == text
        {
            if lvl == level {
                return Some(i);
            }
            loose.get_or_insert(i);
        }
    }
    loose
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, path: &str) -> Bookmark {
        Bookmark {
            id: id.into(),
            title: None,
            kind: Kind::Note { path: path.into() },
        }
    }

    fn heading(id: &str, path: &str, text: &str, level: u8) -> Bookmark {
        Bookmark {
            id: id.into(),
            title: None,
            kind: Kind::Heading {
                path: path.into(),
                text: text.into(),
                level,
            },
        }
    }

    fn group(id: &str, name: &str, kids: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            id: id.into(),
            title: Some(name.into()),
            kind: Kind::Group { items: kids },
        }
    }

    fn no_titles(_: &str) -> Option<String> {
        None
    }

    fn to_text(items: &[Bookmark]) -> String {
        toml::to_string(&to_doc(items)).expect("serialise")
    }

    fn from_text(text: &str) -> Result<Vec<Bookmark>, String> {
        toml::from_str::<BookmarksDoc>(text)
            .map(from_doc)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn round_trip_keeps_every_kind() {
        let items = vec![
            note("a", "想法/a.md"),
            heading("b", "想法/a.md", "设计", 2),
            Bookmark {
                id: "c".into(),
                title: Some("查询".into()),
                kind: Kind::Search {
                    query: "tag:todo -done".into(),
                },
            },
            Bookmark {
                id: "d".into(),
                title: None,
                kind: Kind::Folder {
                    path: "项目".into(),
                },
            },
            group("g", "工作", vec![note("e", "工作/x.md")]),
        ];
        let text = to_text(&items);
        assert_eq!(from_text(&text).unwrap(), items);
    }

    #[test]
    fn broken_file_is_an_error() {
        assert!(from_text("items = [").is_err());
        assert!(
            from_text(
                "version = 1
items = 5"
            )
            .is_err()
        );
    }

    #[test]
    fn bad_entries_are_skipped_and_duplicate_ids_dropped() {
        let text = r#"
version = 1
[[items]]
id = "x"
kind = "note"
path = "a.md"

[[items]]
id = "y"
kind = "bogus"

[[items]]
id = "x"
kind = "note"
path = "dup.md"

[[items]]
id = "z"
kind = "heading"
path = "a.md"
text = "T"
level = 9

[[items]]
id = "w"
kind = "search"
"#;
        let items = from_text(text).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].kind,
            Kind::Note {
                path: "a.md".into()
            }
        );
    }

    #[test]
    fn groups_nest_at_most_one_level() {
        let text = r#"
version = 1
[[items]]
id = "g1"
kind = "group"
title = "A"

[[items.items]]
id = "g2"
kind = "group"
title = "B"

[[items.items.items]]
id = "g3"
kind = "group"
title = "C"
items = []

[[items.items.items]]
id = "n"
kind = "note"
path = "n.md"
"#;
        let items = from_text(text).unwrap();
        let g2 = find(&items, "g2").expect("second level kept");
        assert!(g2.is_group());
        assert!(find(&items, "g3").is_none(), "third level dropped");
        assert!(find(&items, "n").is_some());
    }

    #[test]
    fn duplicate_prevention_for_add_and_toggle() {
        let mut items = vec![note("a", "a.md")];
        assert!(!add_unique(
            &mut items,
            Kind::Note {
                path: "a.md".into()
            },
            "b".into()
        ));
        assert_eq!(items.len(), 1);
        assert!(add_unique(
            &mut items,
            Kind::Note {
                path: "c.md".into()
            },
            "c".into()
        ));
        assert_eq!(items.len(), 2);

        // Toggle removes an existing entry wherever it sits, including inside a group.
        let mut nested = vec![group("g", "G", vec![note("n", "a.md")])];
        assert_eq!(
            toggle(
                &mut nested,
                Kind::Note {
                    path: "a.md".into()
                },
                "t".into()
            ),
            Toggle::Removed
        );
        assert!(find(&nested, "n").is_none());
        assert_eq!(
            toggle(
                &mut nested,
                Kind::Note {
                    path: "a.md".into()
                },
                "t2".into()
            ),
            Toggle::Added
        );
        assert!(find(&nested, "t2").is_some());
    }

    #[test]
    fn heading_bookmark_survives_rename_of_its_note() {
        let mut items = vec![
            heading("h", "想法/旧.md", "设计", 2),
            note("n", "想法/旧.md"),
        ];
        let renames = vec![("想法/旧.md".to_string(), "想法/新.md".to_string())];
        assert!(remap(
            &mut items,
            &renames,
            Some(("想法/旧.md", "想法/新.md"))
        ));
        match &items[0].kind {
            Kind::Heading { path, text, level } => {
                assert_eq!(path, "想法/新.md");
                assert_eq!(text, "设计");
                assert_eq!(*level, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            items[1].kind,
            Kind::Note {
                path: "想法/新.md".into()
            }
        );
    }

    #[test]
    fn folder_rename_remaps_children() {
        let mut items = vec![
            Bookmark {
                id: "f".into(),
                title: None,
                kind: Kind::Folder {
                    path: "项目".into(),
                },
            },
            Bookmark {
                id: "f2".into(),
                title: None,
                kind: Kind::Folder {
                    path: "项目/子".into(),
                },
            },
            note("n", "项目/a.md"),
            note("other", "项目二/a.md"),
            group("g", "G", vec![heading("h", "项目/b.md", "T", 1)]),
        ];
        let renames = vec![("项目/a.md".to_string(), "资料/a.md".to_string())];
        assert!(remap(&mut items, &renames, Some(("项目", "资料"))));
        assert_eq!(
            find(&items, "f").unwrap().kind,
            Kind::Folder {
                path: "资料".into()
            }
        );
        assert_eq!(
            find(&items, "f2").unwrap().kind,
            Kind::Folder {
                path: "资料/子".into()
            }
        );
        assert_eq!(
            find(&items, "n").unwrap().kind,
            Kind::Note {
                path: "资料/a.md".into()
            }
        );
        // A sibling whose name only starts the same way is not touched.
        assert_eq!(
            find(&items, "other").unwrap().kind,
            Kind::Note {
                path: "项目二/a.md".into()
            }
        );
        match &find(&items, "h").unwrap().kind {
            Kind::Heading { path, .. } => assert_eq!(path, "资料/b.md"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unchanged_paths_report_no_change() {
        let mut items = vec![note("n", "a.md")];
        assert!(!remap(&mut items, &[], Some(("x", "y"))));
        assert!(!remap(&mut items, &[("a.md".into(), "a.md".into())], None));
    }

    #[test]
    fn reorder_bounds_and_moves() {
        let mut items = vec![note("a", "a.md"), note("b", "b.md"), note("c", "c.md")];
        assert!(!move_step(&mut items, "a", -1), "top stays put");
        assert!(!move_step(&mut items, "c", 1), "bottom stays put");
        assert!(move_step(&mut items, "a", 1));
        assert_eq!(
            items.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
        assert!(!move_step(&mut items, "missing", 1));

        assert!(move_before(&mut items, "c", "b"));
        assert_eq!(
            items.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            ["c", "b", "a"]
        );
        assert!(!move_before(&mut items, "a", "a"));
        assert!(!move_before(&mut items, "a", "nope"));
    }

    #[test]
    fn moves_into_and_out_of_groups_respect_depth() {
        let mut items = vec![
            group("g1", "一", vec![]),
            group("g2", "二", vec![]),
            note("n", "n.md"),
        ];
        assert!(move_into_group(&mut items, "n", "g1"));
        assert!(find(&items, "n").is_some());
        assert_eq!(entry_count(&items), 1);

        // g2 into g1 is a second level: allowed. A group that already holds a group cannot nest.
        assert!(move_into_group(&mut items, "g2", "g1"));
        let mut deep = vec![
            group("top", "T", vec![group("mid", "M", vec![])]),
            group("x", "X", vec![]),
        ];
        assert!(
            !move_into_group(&mut deep, "x", "mid"),
            "third level refused"
        );
        assert!(
            !move_into_group(&mut deep, "top", "mid"),
            "group into its own subgroup refused"
        );
        assert!(
            !move_into_group(&mut deep, "top", "top"),
            "group into itself refused"
        );

        assert!(move_to_root(&mut items, "n"));
        assert_eq!(items.last().unwrap().id, "n");
        assert!(ungroup(&mut items, "g1"));
        assert!(find(&items, "g1").is_none());
        assert!(find(&items, "g2").is_some(), "children move up, not lost");
    }

    #[test]
    fn group_choices_skip_excluded_subtree() {
        let items = vec![
            group("g1", "工作", vec![group("g2", "周报", vec![])]),
            group("g3", "个人", vec![]),
        ];
        let all: Vec<String> = group_choices(&items, None)
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(all, ["工作", "工作 / 周报", "个人"]);
        let ex: Vec<String> = group_choices(&items, Some("g1"))
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(ex, ["个人"]);
    }

    #[test]
    fn heading_lookup_skips_code_fences() {
        let text = "# 标题\n正文\n```\n## 不是标题\n```\n## 设计 ##\n细节\n";
        assert_eq!(heading_at_line(text, 2), Some((1, "标题".to_string())));
        assert_eq!(heading_at_line(text, 4), Some((1, "标题".to_string())));
        assert_eq!(heading_at_line(text, 7), Some((2, "设计".to_string())));
        assert_eq!(heading_row(text, "设计", 2), Some(5));
        assert_eq!(heading_row(text, "不存在", 2), None);
        assert_eq!(parse_heading("#  C#"), Some((1, "C#".to_string())));
        assert_eq!(parse_heading("#nospace"), None);
        assert_eq!(parse_heading("#"), None);
    }

    #[test]
    fn missing_targets_are_detected() {
        let has_note = |p: &str| p == "a.md";
        let has_folder = |p: &str| p == "项目";
        assert!(!is_missing(
            &Kind::Note {
                path: "a.md".into()
            },
            &has_note,
            &has_folder
        ));
        assert!(is_missing(
            &Kind::Note {
                path: "gone.md".into()
            },
            &has_note,
            &has_folder
        ));
        assert!(is_missing(
            &Kind::Heading {
                path: "gone.md".into(),
                text: "T".into(),
                level: 1
            },
            &has_note,
            &has_folder
        ));
        assert!(!is_missing(
            &Kind::Folder {
                path: "项目".into()
            },
            &has_note,
            &has_folder
        ));
        assert!(is_missing(
            &Kind::Folder { path: "无".into() },
            &has_note,
            &has_folder
        ));
        assert!(!is_missing(
            &Kind::Search { query: "x".into() },
            &has_note,
            &has_folder
        ));
    }

    #[test]
    fn picker_filter_matches_terms_in_any_field() {
        let items = vec![
            note("a", "想法/产品.md"),
            group("g", "工作", vec![heading("h", "会议.md", "周计划", 2)]),
        ];
        let rows = picker_rows(&items, &no_titles);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].sub, "工作 · 会议.md");
        assert_eq!(picker_matches(&rows, ""), vec![0, 1]);
        assert_eq!(picker_matches(&rows, "周计划"), vec![1]);
        assert_eq!(picker_matches(&rows, "工作 会议"), vec![1]);
        assert_eq!(picker_matches(&rows, "产品 工作"), Vec::<usize>::new());
        assert_eq!(picker_matches(&rows, "PRODUCT"), Vec::<usize>::new());
    }

    #[test]
    fn display_title_prefers_custom_title() {
        let titled = Bookmark {
            id: "a".into(),
            title: Some("  自定义 ".trim().into()),
            kind: Kind::Note {
                path: "x/y.md".into(),
            },
        };
        assert_eq!(display_title(&titled, &no_titles), "自定义");
        assert_eq!(display_title(&note("b", "x/y.md"), &no_titles), "y");
        let lookup = |_: &str| Some("笔记标题".to_string());
        assert_eq!(display_title(&note("b", "x/y.md"), &lookup), "笔记标题");
    }
}
