//! Central keyboard shortcut registry.
//!
//! Every customisable command has one row in `SPECS`: a stable id, its Chinese label, its settings
//! category, the context it works in (`When`), and its default chords. The user overrides chords in
//! `config.toml` `[keys]` (see `config_path`). `Keymap` holds only the overrides, so a command without an
//! override uses its defaults.
//!
//! Dispatch: `main.rs` looks up Global commands with `lookup_key` and runs them through
//! `action_for` (or `run_local` for the commands handled by the pane code). Editor commands are bound
//! as GPUI key bindings by `rebind`. Sidebar commands are checked in the file tree.
//!
//! Text-editing keys (arrows, Ctrl+C/V/X/Z/A/Y, Backspace...) are not customisable: they are listed in
//! `RESERVED` and count as conflicts when a shortcut is chosen.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use gpui::{App, KeyBinding};

use crate::AppCommand;
use crate::ui::ext::ExtCommand;
use crate::ui::{
    ai_ui, attach_ui, crumb, files_ui, history_ui, search_ui, sync_ui, template_ui, text_input,
};

/// Settings categories, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    File,
    Edit,
    View,
    Search,
    Ai,
    Sync,
    Pane,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::File,
        Category::Edit,
        Category::View,
        Category::Search,
        Category::Ai,
        Category::Sync,
        Category::Pane,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::File => "文件",
            Category::Edit => "编辑",
            Category::View => "视图",
            Category::Search => "搜索",
            Category::Ai => "AI",
            Category::Sync => "同步",
            Category::Pane => "窗格",
        }
    }
}

/// Where a command works. `Global` overlaps every context; `Editor` and `Sidebar` do not overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Global,
    Editor,
    Sidebar,
}

/// One registry row.
pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub when: When,
    pub defaults: &'static [&'static str],
}

/// Every customisable command. Ids are stable: they are the keys of `[keys]` in `config.toml`.
pub const SPECS: &[Spec] = &[
    // 文件
    Spec {
        id: "file.new_note",
        label: "新建笔记",
        category: Category::File,
        when: When::Global,
        defaults: &["ctrl-n"],
    },
    Spec {
        id: "file.save",
        label: "保存笔记",
        category: Category::File,
        when: When::Global,
        defaults: &["ctrl-s"],
    },
    Spec {
        id: "file.today",
        label: "今日日记",
        category: Category::File,
        when: When::Global,
        defaults: &["ctrl-shift-d"],
    },
    Spec {
        id: "file.daily_prev",
        label: "上一篇日记",
        category: Category::File,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "file.daily_next",
        label: "下一篇日记",
        category: Category::File,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "file.new_folder",
        label: "新建文件夹",
        category: Category::File,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "file.copy_path",
        label: "复制当前笔记的文件路径",
        category: Category::File,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "file.reveal",
        label: "在资源管理器中显示当前笔记",
        category: Category::File,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "file.rename",
        label: "重命名当前笔记",
        category: Category::File,
        when: When::Sidebar,
        defaults: &["f2"],
    },
    Spec {
        id: "file.delete",
        label: "删除当前笔记",
        category: Category::File,
        when: When::Sidebar,
        defaults: &["delete"],
    },
    // 编辑
    Spec {
        id: "edit.footnote",
        label: "插入脚注",
        category: Category::Edit,
        when: When::Editor,
        defaults: &["ctrl-shift-e"],
    },
    Spec {
        id: "edit.insert_template",
        label: "插入模板…",
        category: Category::Edit,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "edit.template_note",
        label: "用模板新建笔记…",
        category: Category::Edit,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "edit.insert_embed",
        label: "插入嵌入",
        category: Category::Edit,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "edit.paste_image",
        label: "粘贴图片",
        category: Category::Edit,
        when: When::Global,
        defaults: &[],
    },
    // 视图
    Spec {
        id: "view.quick_switch",
        label: "快速切换",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-k", "ctrl-o"],
    },
    Spec {
        id: "view.command_palette",
        label: "命令面板",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-p"],
    },
    Spec {
        id: "view.settings",
        label: "打开设置",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-,"],
    },
    Spec {
        id: "view.toggle_sidebar",
        label: "切换侧栏",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-b"],
    },
    Spec {
        id: "view.toggle_right",
        label: "切换右栏",
        category: Category::View,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "view.graph",
        label: "打开知识图谱",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-g"],
    },
    Spec {
        id: "view.theme",
        label: "切换主题",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-t"],
    },
    Spec {
        id: "view.history",
        label: "历史版本…",
        category: Category::View,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "view.tab_prev",
        label: "上一个标签",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-pageup"],
    },
    Spec {
        id: "view.tab_next",
        label: "下一个标签",
        category: Category::View,
        when: When::Global,
        defaults: &["ctrl-pagedown"],
    },
    // 搜索
    Spec {
        id: "search.global",
        label: "全局搜索",
        category: Category::Search,
        when: When::Global,
        defaults: &["ctrl-shift-f"],
    },
    Spec {
        id: "search.find",
        label: "在笔记中查找",
        category: Category::Search,
        when: When::Global,
        defaults: &["ctrl-f"],
    },
    Spec {
        id: "search.replace",
        label: "替换",
        category: Category::Search,
        when: When::Global,
        defaults: &["ctrl-h"],
    },
    // AI
    Spec {
        id: "ai.toggle",
        label: "AI：打开或关闭助手",
        category: Category::Ai,
        when: When::Global,
        defaults: &["ctrl-shift-a"],
    },
    Spec {
        id: "ai.action_menu",
        label: "AI：选区操作…",
        category: Category::Ai,
        when: When::Global,
        defaults: &["ctrl-j"],
    },
    // 同步
    Spec {
        id: "sync.now",
        label: "立即同步",
        category: Category::Sync,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "sync.settings",
        label: "同步设置",
        category: Category::Sync,
        when: When::Global,
        defaults: &[],
    },
    // 窗格
    Spec {
        id: "pane.split_right",
        label: "向右拆分",
        category: Category::Pane,
        when: When::Global,
        defaults: &["ctrl-\\"],
    },
    Spec {
        id: "pane.split_down",
        label: "向下拆分",
        category: Category::Pane,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "pane.close_split",
        label: "关闭分屏",
        category: Category::Pane,
        when: When::Global,
        defaults: &[],
    },
    Spec {
        id: "pane.focus_prev",
        label: "上一个窗格",
        category: Category::Pane,
        when: When::Global,
        defaults: &["ctrl-alt-left", "ctrl-alt-up"],
    },
    Spec {
        id: "pane.focus_next",
        label: "下一个窗格",
        category: Category::Pane,
        when: When::Global,
        defaults: &["ctrl-alt-right", "ctrl-alt-down"],
    },
    Spec {
        id: "pane.tab_move_prev",
        label: "标签左移",
        category: Category::Pane,
        when: When::Global,
        defaults: &["ctrl-shift-pageup"],
    },
    Spec {
        id: "pane.tab_move_next",
        label: "标签右移",
        category: Category::Pane,
        when: When::Global,
        defaults: &["ctrl-shift-pagedown"],
    },
    Spec {
        id: "nav.back",
        label: "后退",
        category: Category::View,
        when: When::Global,
        defaults: &["alt-left"],
    },
    Spec {
        id: "nav.forward",
        label: "前进",
        category: Category::View,
        when: When::Global,
        defaults: &["alt-right"],
    },
];

/// Ids handled outside `action_for`: the sidebar (`files_ui::tree_key`), the editor (GPUI binding)
/// and the tab and pane code (`run_local`).
pub const LOCAL_IDS: &[&str] = &[
    "file.rename",
    "file.delete",
    "view.tab_prev",
    "view.tab_next",
    "pane.tab_move_prev",
    "pane.tab_move_next",
    "pane.focus_prev",
    "pane.focus_next",
];

/// Text-editing keys that are not customisable, with their meaning (settings page, conflicts).
pub const RESERVED: &[(&str, &str)] = &[
    ("ctrl-a", "全选"),
    ("ctrl-c", "复制"),
    ("ctrl-x", "剪切"),
    ("ctrl-v", "粘贴"),
    ("ctrl-z", "撤销"),
    ("ctrl-y", "重做"),
    ("ctrl-shift-z", "重做"),
    ("left", "光标左移"),
    ("right", "光标右移"),
    ("up", "光标上移"),
    ("down", "光标下移"),
    ("shift-left", "选择左移"),
    ("shift-right", "选择右移"),
    ("shift-up", "选择上移"),
    ("shift-down", "选择下移"),
    ("ctrl-left", "按词左移"),
    ("ctrl-right", "按词右移"),
    ("ctrl-shift-left", "按词选择"),
    ("ctrl-shift-right", "按词选择"),
    ("home", "行首"),
    ("end", "行尾"),
    ("shift-home", "选到行首"),
    ("shift-end", "选到行尾"),
    ("ctrl-home", "文首"),
    ("ctrl-end", "文末"),
    ("ctrl-shift-home", "选到文首"),
    ("ctrl-shift-end", "选到文末"),
    ("pageup", "上一页"),
    ("pagedown", "下一页"),
    ("shift-pageup", "选择上一页"),
    ("shift-pagedown", "选择下一页"),
    ("backspace", "删除前一个字符"),
    ("delete", "删除后一个字符"),
    ("ctrl-backspace", "删除前一个词"),
    ("ctrl-delete", "删除后一个词"),
    ("enter", "换行"),
    ("tab", "缩进"),
    ("shift-tab", "取消缩进"),
    ("escape", "关闭或取消"),
    ("alt-up", "上移行"),
    ("alt-down", "下移行"),
    ("alt-shift-down", "复制行"),
];

/// Localized meaning of a reserved editing key.
pub fn reserved_meaning(keys: &str, default_meaning: &str) -> String {
    match keys {
        "ctrl-a" => crate::t!("keymap-reserved-select-all"),
        "ctrl-c" => crate::t!("keymap-reserved-copy"),
        "ctrl-x" => crate::t!("keymap-reserved-cut"),
        "ctrl-v" => crate::t!("keymap-reserved-paste"),
        "ctrl-z" => crate::t!("keymap-reserved-undo"),
        "ctrl-y" | "ctrl-shift-z" => crate::t!("keymap-reserved-redo"),
        "left" => crate::t!("keymap-reserved-move-left"),
        "right" => crate::t!("keymap-reserved-move-right"),
        "up" => crate::t!("keymap-reserved-move-up"),
        "down" => crate::t!("keymap-reserved-move-down"),
        "shift-left" => crate::t!("keymap-reserved-select-left"),
        "shift-right" => crate::t!("keymap-reserved-select-right"),
        "shift-up" => crate::t!("keymap-reserved-select-up"),
        "shift-down" => crate::t!("keymap-reserved-select-down"),
        "ctrl-left" => crate::t!("keymap-reserved-word-left"),
        "ctrl-right" => crate::t!("keymap-reserved-word-right"),
        "ctrl-shift-left" | "ctrl-shift-right" => crate::t!("keymap-reserved-select-word"),
        "home" => crate::t!("keymap-reserved-line-start"),
        "end" => crate::t!("keymap-reserved-line-end"),
        "shift-home" => crate::t!("keymap-reserved-select-line-start"),
        "shift-end" => crate::t!("keymap-reserved-select-line-end"),
        "ctrl-home" => crate::t!("keymap-reserved-doc-start"),
        "ctrl-end" => crate::t!("keymap-reserved-doc-end"),
        "ctrl-shift-home" => crate::t!("keymap-reserved-select-doc-start"),
        "ctrl-shift-end" => crate::t!("keymap-reserved-select-doc-end"),
        "pageup" => crate::t!("keymap-reserved-page-up"),
        "pagedown" => crate::t!("keymap-reserved-page-down"),
        "shift-pageup" => crate::t!("keymap-reserved-select-page-up"),
        "shift-pagedown" => crate::t!("keymap-reserved-select-page-down"),
        "backspace" => crate::t!("keymap-reserved-del-prev-char"),
        "delete" => crate::t!("keymap-reserved-del-next-char"),
        "ctrl-backspace" => crate::t!("keymap-reserved-del-prev-word"),
        "ctrl-delete" => crate::t!("keymap-reserved-del-next-word"),
        "enter" => crate::t!("keymap-reserved-line-break"),
        "tab" => crate::t!("keymap-reserved-indent"),
        "shift-tab" => crate::t!("keymap-reserved-outdent"),
        "escape" => crate::t!("keymap-reserved-close-cancel"),
        "alt-up" => crate::t!("keymap-reserved-move-line-up"),
        "alt-down" => crate::t!("keymap-reserved-move-line-down"),
        "alt-shift-down" => crate::t!("keymap-reserved-dup-line"),
        _ => default_meaning.to_string(),
    }
}

/// Looks up a registry row by id.
pub fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

// ---------------------------------------------------------------------------
// Chords
// ---------------------------------------------------------------------------

/// One key press: modifiers and a lowercase key name in gpui spelling (`a`, `,`, `\`, `pageup`, `f2`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    pub key: String,
}

const NAMED_KEYS: &[&str] = &[
    "pageup",
    "pagedown",
    "home",
    "end",
    "left",
    "right",
    "up",
    "down",
    "delete",
    "backspace",
    "enter",
    "tab",
    "space",
    "escape",
    "insert",
];

/// Normalises a key name typed by a user or read from a file.
fn normalize_key(raw: &str) -> Result<String, String> {
    let lower = raw.trim().to_ascii_lowercase();
    let alias = match lower.as_str() {
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        "esc" => "escape",
        "del" => "delete",
        "return" => "enter",
        "ins" => "insert",
        other => other,
    };
    if alias.chars().count() == 1 {
        let c = alias.chars().next().unwrap_or(' ');
        if c.is_ascii_alphanumeric() || ",./\\;'[]-=`".contains(c) {
            return Ok(alias.to_string());
        }
    } else if NAMED_KEYS.contains(&alias) {
        return Ok(alias.to_string());
    } else if let Some(n) = alias.strip_prefix('f')
        && let Ok(num) = n.parse::<u8>()
        && (1..=12).contains(&num)
    {
        return Ok(alias.to_string());
    }
    Err(format!("未知按键：{}", raw.trim()))
}

impl Chord {
    /// Parses `ctrl-shift-a`, `Ctrl+Shift+A`, `ctrl-,`, `ctrl-\`, `ctrl+pageup`, `f2`. Case and the
    /// order of modifiers do not matter.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("快捷键为空".to_string());
        }
        let sep = if text.contains('+') { '+' } else { '-' };
        let parts: Vec<&str> = text.split(sep).map(str::trim).collect();
        let Some((key_raw, mods)) = parts.split_last() else {
            return Err("快捷键为空".to_string());
        };
        let mut chord = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            platform: false,
            key: String::new(),
        };
        for m in mods {
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut chord.ctrl,
                "alt" | "option" => &mut chord.alt,
                "shift" => &mut chord.shift,
                "win" | "cmd" | "super" | "meta" => &mut chord.platform,
                other => return Err(format!("未知修饰键：{other}")),
            };
            if *flag {
                return Err(format!("修饰键重复：{m}"));
            }
            *flag = true;
        }
        chord.key = normalize_key(key_raw)?;
        Ok(chord)
    }

    /// Builds a chord from a gpui keystroke (the key is lowercased).
    pub fn from_keystroke(ks: &gpui::Keystroke) -> Chord {
        Chord {
            ctrl: ks.modifiers.control,
            alt: ks.modifiers.alt,
            shift: ks.modifiers.shift,
            platform: ks.modifiers.platform,
            key: ks.key.to_ascii_lowercase(),
        }
    }

    /// Display form: `Ctrl+Shift+A`, `Ctrl+Alt+Left`, `Ctrl+,`, `F2` (macOS: `⌘⇧A`).
    pub fn format(&self) -> String {
        self.format_for(cfg!(target_os = "macos"))
    }

    /// Display form for macOS (`mac`) or for Windows and Linux.
    pub fn format_for(&self, mac: bool) -> String {
        if mac {
            let mut out = String::new();
            if self.ctrl {
                out.push('⌃');
            }
            if self.alt {
                out.push('⌥');
            }
            if self.shift {
                out.push('⇧');
            }
            if self.platform {
                out.push('⌘');
            }
            out.push_str(&display_key(&self.key));
            return out;
        }
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.platform {
            parts.push("Win");
        }
        let key = display_key(&self.key);
        parts.push(&key);
        parts.join("+")
    }

    /// gpui keystroke spelling for `KeyBinding::new`: `ctrl-shift-a`.
    pub fn to_gpui(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.platform {
            parts.push("cmd");
        }
        parts.push(&self.key);
        parts.join("-")
    }

    /// No modifier at all (plain typing keys).
    pub fn is_bare(&self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.platform)
    }

    /// Ctrl+Alt+letter: on some layouts Windows reports these as AltGr and the app never sees them.
    pub fn altgr_risk(&self) -> bool {
        self.ctrl
            && self.alt
            && self.key.len() == 1
            && self.key.chars().all(|c| c.is_ascii_alphabetic())
    }
}

fn display_key(key: &str) -> String {
    match key {
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "escape" => "Esc".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        "enter" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "Space".to_string(),
        "insert" => "Insert".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        k if k.len() == 1 => k.to_ascii_uppercase(),
        k => k.to_ascii_uppercase(),
    }
}

/// Why a chord cannot be used for a command in a context, or `None` when it can.
pub fn validate(when: When, chord: &Chord) -> Option<&'static str> {
    if chord.key == "escape" {
        return Some("Esc 保留给取消操作");
    }
    let fkey =
        chord.key.len() >= 2 && chord.key.starts_with('f') && chord.key[1..].parse::<u8>().is_ok();
    if chord.is_bare() && !fkey && when != When::Sidebar {
        return Some("需要至少一个修饰键（Ctrl、Alt、Shift 或 Win）");
    }
    None
}

/// Context overlap: a Global command is reachable wherever any other command is.
fn overlaps(a: When, b: When) -> bool {
    a == b || a == When::Global || b == When::Global
}

/// One clash found by `Keymap::conflicts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Registry id, or `""` for a text-editing key.
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// Keymap: overrides on top of the defaults
// ---------------------------------------------------------------------------

/// The effective shortcuts: defaults, with the user's overrides on top.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    overrides: BTreeMap<String, Vec<Chord>>,
}

fn default_chords(spec: &Spec) -> Vec<Chord> {
    let mac = cfg!(target_os = "macos");
    spec.defaults
        .iter()
        .filter_map(|c| Chord::parse(&platform_spelling(c, mac)).ok())
        .collect()
}

/// The default spelling of a shortcut on one platform. On macOS the Command key takes the place of
/// Ctrl (`ctrl-n` becomes `cmd-n`), except for tab paging and pane focus, which keep Ctrl as on
/// other Mac apps. Find and replace uses Cmd+Option+F, because Cmd+H is the system "Hide" command.
/// Windows and Linux spellings are returned unchanged.
pub fn platform_spelling(spelling: &str, mac: bool) -> String {
    if !mac {
        return spelling.to_string();
    }
    if spelling == "ctrl-h" {
        return "cmd-alt-f".to_string();
    }
    let keeps_ctrl =
        spelling.contains("pageup") || spelling.contains("pagedown") || spelling.contains("alt-");
    match spelling.strip_prefix("ctrl-") {
        Some(rest) if !keeps_ctrl => format!("cmd-{rest}"),
        _ => spelling.to_string(),
    }
}

/// The spelling of a text-editing or app key for `KeyBinding::new`: `ctrl-c` on Windows and Linux,
/// `cmd-c` on macOS. Only a leading `ctrl-` is changed.
pub fn primary(spelling: &str) -> String {
    match spelling.strip_prefix("ctrl-") {
        Some(rest) if cfg!(target_os = "macos") => format!("cmd-{rest}"),
        _ => spelling.to_string(),
    }
}

/// Whether the primary shortcut modifier (Ctrl, or Command on macOS) is held.
pub fn primary_held(m: &gpui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.platform
    } else {
        m.control
    }
}

impl Keymap {
    /// Effective chords of a command (empty when unbound).
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        if let Some(over) = self.overrides.get(id) {
            return over.clone();
        }
        spec(id).map(default_chords).unwrap_or_default()
    }

    /// Default chords of a command.
    pub fn default_of(id: &str) -> Vec<Chord> {
        spec(id).map(default_chords).unwrap_or_default()
    }

    /// True when the command still has its default chords.
    pub fn is_default(&self, id: &str) -> bool {
        !self.overrides.contains_key(id)
    }

    /// Sets the chords of a command. The defaults are not stored: setting them removes the override.
    pub fn set(&mut self, id: &str, chords: Vec<Chord>) {
        let mut unique: Vec<Chord> = Vec::new();
        for c in chords {
            if !unique.contains(&c) {
                unique.push(c);
            }
        }
        if unique == Self::default_of(id) {
            self.overrides.remove(id);
        } else {
            self.overrides.insert(id.to_string(), unique);
        }
    }

    /// Restores the defaults of one command.
    pub fn reset(&mut self, id: &str) {
        self.overrides.remove(id);
    }

    /// Restores every default.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    /// The command bound to `chord` in context `when`, if any.
    pub fn lookup(&self, chord: &Chord, when: When) -> Option<&'static str> {
        SPECS
            .iter()
            .filter(|s| s.when == when)
            .find(|s| self.chords(s.id).contains(chord))
            .map(|s| s.id)
    }

    /// Other commands (and text-editing keys) that already use `chord` in an overlapping context.
    pub fn conflicts(&self, id: &str, chord: &Chord) -> Vec<Conflict> {
        let my_when = spec(id).map(|s| s.when).unwrap_or(When::Global);
        let mut out = Vec::new();
        for s in SPECS {
            if s.id != id && overlaps(s.when, my_when) && self.chords(s.id).contains(chord) {
                out.push(Conflict {
                    id: s.id.to_string(),
                    label: s.label.to_string(),
                });
            }
        }
        if overlaps(When::Editor, my_when) {
            for (keys, meaning) in RESERVED {
                // The table is written with Ctrl; on macOS the same keys are Cmd (see `primary`).
                if Chord::parse(&primary(keys)).ok().as_ref() == Some(chord) {
                    out.push(Conflict {
                        id: String::new(),
                        label: format!(
                            "{prefix}：{localized}",
                            prefix = crate::t!("keymap-reserved-conflict-prefix"),
                            localized = reserved_meaning(keys, meaning)
                        ),
                    });
                }
            }
        }
        out
    }

    /// Reads the overrides of the `[keys]` section (command id to chord texts). Unknown ids, bad
    /// chords and forbidden keys are skipped and reported in the returned warnings.
    pub fn from_entries(map: &BTreeMap<String, Vec<String>>) -> (Keymap, Vec<String>) {
        let mut km = Keymap::default();
        let mut warnings = Vec::new();
        for (id, items) in map {
            let Some(spec) = spec(id) else {
                warnings.push(format!("未知命令，已忽略：{id}"));
                continue;
            };
            let mut chords = Vec::new();
            for text in items {
                match Chord::parse(text) {
                    Ok(chord) => match validate(spec.when, &chord) {
                        None => chords.push(chord),
                        Some(why) => warnings.push(format!("{id}：{text} 不可用（{why}）")),
                    },
                    Err(e) => warnings.push(format!("{id}：{e}")),
                }
            }
            if !items.is_empty() && chords.is_empty() {
                // Every chord was bad: keep the defaults rather than silently unbinding.
                continue;
            }
            km.set(id, chords);
        }
        (km, warnings)
    }

    /// The overrides as the `[keys]` section. Chords are written in the text form that
    /// `Chord::parse` reads on every platform.
    pub fn to_entries(&self) -> BTreeMap<String, Vec<String>> {
        self.overrides
            .iter()
            .map(|(id, chords)| {
                (
                    id.clone(),
                    chords.iter().map(|c| c.format_for(false)).collect(),
                )
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// The live keymap, the file, and the GPUI bindings
// ---------------------------------------------------------------------------

static CURRENT: RwLock<Keymap> = RwLock::new(Keymap {
    overrides: BTreeMap::new(),
});
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A copy of the live keymap.
pub fn current() -> Keymap {
    CURRENT.read().map(|k| k.clone()).unwrap_or_default()
}

/// Replaces the live keymap (after a settings change or at start-up).
pub fn set_current(km: Keymap) {
    if let Ok(mut guard) = CURRENT.write() {
        *guard = km;
    }
}

/// Display text of the first chord of a command, or empty when unbound.
pub fn shortcut_text(id: &str) -> String {
    current()
        .chords(id)
        .first()
        .map(Chord::format)
        .unwrap_or_default()
}

/// Display text of the shortcut of a palette command, looked up from its `AppCommand`.
pub fn shortcut_for(command: &AppCommand) -> String {
    command_key(command).map(shortcut_text).unwrap_or_default()
}

/// Id of the Global command bound to this key press, if any.
pub fn lookup_key(ks: &gpui::Keystroke, when: When) -> Option<&'static str> {
    current().lookup(&Chord::from_keystroke(ks), when)
}

/// The user's `config.toml`. Keyboard shortcuts are its `[keys]` section.
pub fn config_path() -> Option<PathBuf> {
    Some(jiyue_core::config::user_file(
        jiyue_core::config::USER_CONFIG_FILE,
    ))
}

/// Reads the `[keys]` section and makes it live. A missing section means defaults; a broken file
/// is ignored whole and its problem is queued for `take_warnings`.
pub fn init_from_disk() {
    let mut warnings = Vec::new();
    let km = match config_path() {
        Some(path) => {
            let loaded: jiyue_core::config::Loaded<jiyue_core::config::UserConfig> =
                jiyue_core::config::load_file_with(&path, &["keys"]);
            match loaded.error {
                Some(issue) => {
                    warnings.push(format!("快捷键配置无效，已使用默认值：{issue}"));
                    Keymap::default()
                }
                None => {
                    let (km, w) = Keymap::from_entries(&loaded.value.keys);
                    warnings.extend(w);
                    km
                }
            }
        }
        None => Keymap::default(),
    };
    set_current(km);
    if let Ok(mut q) = WARNINGS.lock() {
        q.extend(warnings);
    }
}

/// Warnings queued by `init_from_disk` or a failed save; drained by the app.
pub fn take_warnings() -> Vec<String> {
    WARNINGS
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Writes the overrides to the `[keys]` section of `config.toml`, keeping the rest of the file.
pub fn save_to(path: &Path, km: &Keymap) -> Result<(), String> {
    jiyue_core::config::save_section(
        path,
        "keys",
        &km.to_entries(),
        jiyue_core::config::templates::USER_CONFIG,
        &["keys"],
    )
}

/// Makes `km` live, saves it, and rebinds the GPUI keys. A failed save is reported in the toasts.
pub fn apply(cx: &mut App, km: Keymap) {
    set_current(km.clone());
    if let Some(path) = config_path()
        && let Err(e) = save_to(&path, &km)
        && let Ok(mut q) = WARNINGS.lock()
    {
        q.push(format!("快捷键未能保存：{e}"));
    }
    rebind(cx);
}

/// Clears every GPUI key binding and binds them again from the registry. Call at start-up and after
/// each change. The editor keys of `text_input` and the widgets keep their own bindings.
pub fn rebind(cx: &mut App) {
    cx.clear_key_bindings();
    text_input::bind_keys(cx);
    crate::ui::widgets::bind_keys(cx);
    let km = current();
    let mut bindings = Vec::new();
    for chord in km.chords("edit.footnote") {
        bindings.push(KeyBinding::new(
            &chord.to_gpui(),
            text_input::InsertFootnote,
            Some(text_input::CONTEXT),
        ));
    }
    cx.bind_keys(bindings);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The app command of a registry id. `None` for `LOCAL_IDS` and unknown ids.
pub fn action_for(id: &str) -> Option<AppCommand> {
    let ext = |c: ExtCommand| Some(AppCommand::Ext(c));
    match id {
        "file.new_note" => Some(AppCommand::NewNote),
        "file.save" => Some(AppCommand::SaveNote),
        "file.today" => Some(AppCommand::OpenToday),
        "file.daily_prev" => Some(AppCommand::DailyPrev),
        "file.daily_next" => Some(AppCommand::DailyNext),
        "file.new_folder" => ext(ExtCommand::Files(files_ui::Command::NewFolder)),
        "file.copy_path" => ext(ExtCommand::Crumb(crumb::Command::CopyPath)),
        "file.reveal" => ext(ExtCommand::Crumb(crumb::Command::Reveal)),
        "edit.insert_template" => ext(ExtCommand::Template(template_ui::Command::Insert)),
        "edit.template_note" => ext(ExtCommand::Template(template_ui::Command::NewNote)),
        "edit.insert_embed" => ext(ExtCommand::Attach(attach_ui::Command::InsertEmbed)),
        "edit.paste_image" => ext(ExtCommand::Attach(attach_ui::Command::PasteImage)),
        "edit.footnote" => ext(ExtCommand::InsertFootnote),
        "view.quick_switch" => Some(AppCommand::OpenQuickSwitcher),
        "view.command_palette" => Some(AppCommand::OpenCommandPalette),
        "view.settings" => Some(AppCommand::OpenSettings),
        "view.toggle_sidebar" => Some(AppCommand::ToggleSidebar),
        "view.toggle_right" => Some(AppCommand::ToggleRightPanel),
        "view.graph" => Some(AppCommand::OpenGraph),
        "view.theme" => Some(AppCommand::CycleTheme),
        "view.history" => ext(ExtCommand::History(history_ui::Command::Open)),
        "pane.split_right" => Some(AppCommand::SplitRight),
        "pane.split_down" => Some(AppCommand::SplitDown),
        "pane.close_split" => Some(AppCommand::CloseSplit),
        "nav.back" => Some(AppCommand::NavBack),
        "nav.forward" => Some(AppCommand::NavForward),
        "search.global" => ext(ExtCommand::Search(search_ui::Command::OpenGlobalSearch)),
        "search.find" => ext(ExtCommand::Search(search_ui::Command::OpenFind)),
        "search.replace" => ext(ExtCommand::Search(search_ui::Command::OpenReplace)),
        "ai.toggle" => ext(ExtCommand::Ai(ai_ui::Command::TogglePanel)),
        "ai.action_menu" => ext(ExtCommand::Ai(ai_ui::Command::ActionMenu)),
        "sync.now" => ext(ExtCommand::Sync(sync_ui::Command::SyncNow)),
        "sync.settings" => ext(ExtCommand::Sync(sync_ui::Command::OpenSyncSettings)),
        _ => None,
    }
}

/// The registry id of an app command: the inverse of `action_for`.
pub fn command_key(command: &AppCommand) -> Option<&'static str> {
    let ext = |c: &ExtCommand| match c {
        ExtCommand::Files(files_ui::Command::NewFolder) => Some("file.new_folder"),
        ExtCommand::Crumb(crumb::Command::CopyPath) => Some("file.copy_path"),
        ExtCommand::Crumb(crumb::Command::Reveal) => Some("file.reveal"),
        ExtCommand::Template(template_ui::Command::Insert) => Some("edit.insert_template"),
        ExtCommand::Template(template_ui::Command::NewNote) => Some("edit.template_note"),
        ExtCommand::Attach(attach_ui::Command::InsertEmbed) => Some("edit.insert_embed"),
        ExtCommand::Attach(attach_ui::Command::PasteImage) => Some("edit.paste_image"),
        ExtCommand::InsertFootnote => Some("edit.footnote"),
        ExtCommand::History(history_ui::Command::Open) => Some("view.history"),
        ExtCommand::Search(search_ui::Command::OpenGlobalSearch) => Some("search.global"),
        ExtCommand::Search(search_ui::Command::OpenFind) => Some("search.find"),
        ExtCommand::Search(search_ui::Command::OpenReplace) => Some("search.replace"),
        ExtCommand::Ai(ai_ui::Command::TogglePanel) => Some("ai.toggle"),
        ExtCommand::Ai(ai_ui::Command::ActionMenu) => Some("ai.action_menu"),
        ExtCommand::Sync(sync_ui::Command::SyncNow) => Some("sync.now"),
        ExtCommand::Sync(sync_ui::Command::OpenSyncSettings) => Some("sync.settings"),
        _ => None,
    };
    match command {
        AppCommand::NewNote => Some("file.new_note"),
        AppCommand::SaveNote => Some("file.save"),
        AppCommand::OpenToday => Some("file.today"),
        AppCommand::DailyPrev => Some("file.daily_prev"),
        AppCommand::DailyNext => Some("file.daily_next"),
        AppCommand::OpenQuickSwitcher => Some("view.quick_switch"),
        AppCommand::OpenCommandPalette => Some("view.command_palette"),
        AppCommand::OpenSettings => Some("view.settings"),
        AppCommand::ToggleSidebar => Some("view.toggle_sidebar"),
        AppCommand::ToggleRightPanel => Some("view.toggle_right"),
        AppCommand::OpenGraph => Some("view.graph"),
        AppCommand::CycleTheme => Some("view.theme"),
        AppCommand::SplitRight => Some("pane.split_right"),
        AppCommand::SplitDown => Some("pane.split_down"),
        AppCommand::CloseSplit => Some("pane.close_split"),
        AppCommand::NavBack => Some("nav.back"),
        AppCommand::NavForward => Some("nav.forward"),
        AppCommand::Ext(c) => ext(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(text: &str) -> Chord {
        Chord::parse(text).expect("valid test chord")
    }

    /// The default spelling of a Windows/Linux chord on this platform (Cmd on macOS). Use it for
    /// any assertion about a shortcut that the defaults bind, never a literal `ctrl-` chord.
    fn dflt(spelling: &str) -> Chord {
        ch(&platform_spelling(spelling, cfg!(target_os = "macos")))
    }

    #[test]
    fn parse_ignores_case_and_modifier_order() {
        assert_eq!(ch("Ctrl+Shift+A"), ch("shift-ctrl-a"));
        assert_eq!(ch("CTRL-SHIFT-A"), ch("ctrl-shift-a"));
        assert_eq!(ch("Alt+Ctrl+Left"), ch("ctrl-alt-left"));
    }

    #[test]
    fn parse_punctuation_keys() {
        assert_eq!(ch("ctrl-,").key, ",");
        assert_eq!(ch("Ctrl+,"), ch("ctrl-,"));
        assert_eq!(ch("ctrl-\\").key, "\\");
        assert_eq!(ch("Ctrl+\\"), ch("ctrl-\\"));
    }

    #[test]
    fn parse_named_keys_and_aliases() {
        assert_eq!(ch("ctrl-pageup").key, "pageup");
        assert_eq!(ch("Ctrl+PgUp"), ch("ctrl-pageup"));
        assert_eq!(ch("f2").key, "f2");
        assert_eq!(ch("F12").key, "f12");
        assert_eq!(ch("del").key, "delete");
        assert_eq!(ch("Esc").key, "escape");
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("ctrl-").is_err());
        assert!(Chord::parse("ctrl-ctrl-a").is_err());
        assert!(Chord::parse("hyper-a").is_err());
        assert!(Chord::parse("ctrl-f13").is_err());
        assert!(Chord::parse("ctrl-nonsense").is_err());
    }

    #[test]
    fn format_display_form() {
        assert_eq!(ch("ctrl-shift-a").format_for(false), "Ctrl+Shift+A");
        assert_eq!(ch("ctrl-alt-left").format_for(false), "Ctrl+Alt+Left");
        assert_eq!(ch("ctrl-,").format_for(false), "Ctrl+,");
        assert_eq!(ch("ctrl-\\").format_for(false), "Ctrl+\\");
        assert_eq!(ch("f2").format_for(false), "F2");
        assert_eq!(ch("ctrl-pageup").format_for(false), "Ctrl+PageUp");
        assert_eq!(ch("escape").format_for(false), "Esc");
    }

    #[test]
    fn gpui_spelling_uses_fixed_modifier_order() {
        assert_eq!(ch("Shift+Ctrl+A").to_gpui(), "ctrl-shift-a");
        assert_eq!(ch("ctrl-alt-f").to_gpui(), "ctrl-alt-f");
        assert_eq!(ch("F2").to_gpui(), "f2");
    }

    #[test]
    fn every_default_round_trips_through_format() {
        for s in SPECS {
            for d in s.defaults {
                let chord = ch(d);
                assert_eq!(
                    Chord::parse(&chord.format_for(false)).expect("reparse"),
                    chord,
                    "{d}"
                );
            }
        }
    }

    #[test]
    fn registry_ids_are_unique_and_dotted() {
        let mut ids: Vec<&str> = SPECS.iter().map(|s| s.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate id");
        assert!(SPECS.iter().all(|s| s.id.contains('.')));
    }

    #[test]
    fn every_command_has_a_handler() {
        for s in SPECS {
            let handled = action_for(s.id).is_some() || LOCAL_IDS.contains(&s.id);
            assert!(handled, "no handler for {}", s.id);
        }
        for id in LOCAL_IDS {
            assert!(spec(id).is_some(), "local id without spec: {id}");
        }
    }

    #[test]
    fn command_key_inverts_action_for() {
        for s in SPECS {
            if let Some(cmd) = action_for(s.id) {
                assert_eq!(command_key(&cmd), Some(s.id));
            }
        }
    }

    #[test]
    fn footnote_palette_command_maps_to_its_registry_id() {
        let cmd = action_for("edit.footnote").expect("edit.footnote has a command");
        assert_eq!(command_key(&cmd), Some("edit.footnote"));
        assert!(matches!(cmd, AppCommand::Ext(ExtCommand::InsertFootnote)));
        assert_eq!(spec("edit.footnote").map(|s| s.when), Some(When::Editor));
        let expected = platform_spelling("ctrl-shift-e", cfg!(target_os = "macos"));
        assert_eq!(Keymap::default_of("edit.footnote"), vec![ch(&expected)]);
    }

    #[test]
    fn defaults_are_valid_and_avoid_altgr_letters() {
        for s in SPECS {
            for d in s.defaults {
                let chord = ch(d);
                assert_eq!(validate(s.when, &chord), None, "{} {d}", s.id);
                assert!(!chord.altgr_risk(), "{} uses Ctrl+Alt+letter {d}", s.id);
            }
        }
    }

    #[test]
    fn defaults_have_no_clashes() {
        let km = Keymap::default();
        for s in SPECS {
            for chord in km.chords(s.id) {
                let clashes: Vec<Conflict> = km.conflicts(s.id, &chord);
                assert!(
                    clashes.is_empty(),
                    "{} {} clashes: {clashes:?}",
                    s.id,
                    chord.format()
                );
            }
        }
    }

    #[test]
    fn overrides_replace_defaults() {
        let mut km = Keymap::default();
        km.set("file.new_note", vec![ch("ctrl-shift-y")]);
        assert_eq!(km.chords("file.new_note"), vec![ch("ctrl-shift-y")]);
        assert_eq!(km.lookup(&dflt("ctrl-n"), When::Global), None);
        assert_eq!(
            km.lookup(&ch("ctrl-shift-y"), When::Global),
            Some("file.new_note")
        );
        assert!(!km.is_default("file.new_note"));
    }

    #[test]
    fn empty_override_unbinds() {
        let mut km = Keymap::default();
        km.set("view.theme", vec![]);
        assert!(km.chords("view.theme").is_empty());
        assert_eq!(km.lookup(&dflt("ctrl-t"), When::Global), None);
    }

    #[test]
    fn setting_defaults_removes_override_and_reset_works() {
        let mut km = Keymap::default();
        km.set("view.theme", vec![ch("ctrl-y")]);
        km.set("view.theme", Keymap::default_of("view.theme"));
        assert!(km.is_default("view.theme"));
        km.set("view.theme", vec![ch("ctrl-y")]);
        km.reset("view.theme");
        assert!(km.is_default("view.theme"));
        km.set("view.theme", vec![ch("ctrl-y")]);
        km.reset_all();
        assert_eq!(km, Keymap::default());
    }

    #[test]
    fn context_decides_lookup() {
        let km = Keymap::default();
        assert_eq!(km.lookup(&ch("f2"), When::Sidebar), Some("file.rename"));
        assert_eq!(km.lookup(&ch("f2"), When::Global), None);
        assert_eq!(
            km.lookup(&dflt("ctrl-shift-e"), When::Editor),
            Some("edit.footnote")
        );
    }

    #[test]
    fn overlapping_contexts_conflict() {
        let km = Keymap::default();
        // Global vs Sidebar overlap, so a Global command may not take F2.
        let c = km.conflicts("view.theme", &ch("f2"));
        assert!(c.iter().any(|x| x.id == "file.rename"));
        // Editor and Sidebar never overlap.
        assert!(km.conflicts("edit.footnote", &ch("f2")).is_empty());
    }

    #[test]
    fn text_editing_keys_are_reserved() {
        let km = Keymap::default();
        let c = km.conflicts("view.theme", &ch(&primary("ctrl-z")));
        assert!(c.iter().any(|x| x.label.contains("撤销")));
    }

    #[test]
    fn plain_keys_need_a_modifier_except_in_sidebar() {
        assert!(validate(When::Global, &ch("a")).is_some());
        assert!(validate(When::Editor, &ch("a")).is_some());
        assert_eq!(validate(When::Global, &ch("f5")), None);
        assert_eq!(validate(When::Sidebar, &ch("delete")), None);
        assert!(validate(When::Global, &ch("escape")).is_some());
    }

    #[test]
    fn altgr_risk_flags_ctrl_alt_letters_only() {
        assert!(ch("ctrl-alt-f").altgr_risk());
        assert!(!ch("ctrl-alt-left").altgr_risk());
        assert!(!ch("ctrl-shift-f").altgr_risk());
    }

    #[test]
    fn entries_round_trip_keeps_overrides() {
        let mut km = Keymap::default();
        km.set("file.new_note", vec![ch("ctrl-shift-y")]);
        km.set("view.theme", vec![]);
        let (back, warnings) = Keymap::from_entries(&km.to_entries());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, km);
    }

    #[test]
    fn no_overrides_means_an_empty_section() {
        assert!(Keymap::default().to_entries().is_empty());
    }

    #[test]
    fn broken_file_is_an_error() {}

    #[test]
    fn bad_entries_are_skipped_with_warnings() {
        let map: BTreeMap<String, Vec<String>> = [
            ("file.new_note", vec!["ctrl-"]),
            ("nope.cmd", vec!["ctrl-x"]),
            ("file.save", vec!["ctrl-"]),
            ("view.theme", vec!["ctrl-shift-y", "@"]),
            ("view.graph", vec!["a"]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into_iter().map(str::to_string).collect()))
        .collect();
        let (km, warnings) = Keymap::from_entries(&map);
        assert_eq!(km.chords("file.new_note"), default_of_test("file.new_note"));
        assert_eq!(km.chords("file.save"), default_of_test("file.save"));
        assert_eq!(km.chords("view.theme"), vec![ch("ctrl-shift-y")]);
        assert_eq!(km.chords("view.graph"), default_of_test("view.graph"));
        assert!(warnings.len() >= 5, "{warnings:?}");
    }

    fn default_of_test(id: &str) -> Vec<Chord> {
        Keymap::default_of(id)
    }

    #[test]
    fn shortcut_text_follows_live_keymap() {
        let mut km = Keymap::default();
        km.set("file.new_note", vec![ch("ctrl-shift-y")]);
        set_current(km);
        assert_eq!(shortcut_text("file.new_note"), ch("ctrl-shift-y").format());
        set_current(Keymap::default());
        assert_eq!(
            shortcut_text("file.new_note"),
            Keymap::default_of("file.new_note")[0].format()
        );
    }

    #[test]
    fn macos_defaults_use_command_for_ctrl_chords() {
        assert_eq!(platform_spelling("ctrl-n", true), "cmd-n");
        assert_eq!(platform_spelling("ctrl-shift-a", true), "cmd-shift-a");
        assert_eq!(platform_spelling("ctrl-,", true), "cmd-,");
        assert_eq!(platform_spelling("ctrl-\\", true), "cmd-\\");
        // Tab paging and pane focus keep Ctrl.
        assert_eq!(platform_spelling("ctrl-pageup", true), "ctrl-pageup");
        assert_eq!(platform_spelling("ctrl-alt-left", true), "ctrl-alt-left");
        // Cmd+H is the system Hide command.
        assert_eq!(platform_spelling("ctrl-h", true), "cmd-alt-f");
    }

    #[test]
    fn windows_and_linux_defaults_are_unchanged() {
        for s in ["ctrl-n", "ctrl-h", "ctrl-pageup", "ctrl-alt-left", "f2"] {
            assert_eq!(platform_spelling(s, false), s);
        }
    }

    #[test]
    fn macos_defaults_stay_valid_and_clash_free() {
        for s in SPECS {
            for d in s.defaults {
                let spelling = platform_spelling(d, true);
                let chord = ch(&spelling);
                assert_eq!(validate(s.when, &chord), None, "{} {spelling}", s.id);
            }
        }
    }

    #[test]
    fn primary_maps_only_the_leading_ctrl() {
        assert_eq!(
            primary("ctrl-c"),
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            }
        );
        assert_eq!(primary("alt-up"), "alt-up");
    }

    #[test]
    fn macos_display_uses_symbols() {
        assert_eq!(ch("cmd-n").format_for(true), "⌘N");
        assert_eq!(ch("ctrl-shift-a").format_for(true), "⌃⇧A");
        assert_eq!(ch("ctrl-alt-left").format_for(true), "⌃⌥Left");
    }

    #[test]
    fn entries_keep_the_parseable_text_form() {
        let mut km = Keymap::default();
        km.set("file.new_note", vec![ch("cmd-shift-y")]);
        let (back, warnings) = Keymap::from_entries(&km.to_entries());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.chords("file.new_note"), vec![ch("cmd-shift-y")]);
    }
}
