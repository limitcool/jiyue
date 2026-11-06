//! Command line launch: which vault to use and which notes to open (pure logic, no gpui).
//!
//! `jiyue.exe [--vault DIR] [--theme NAME] [--open SCREEN] [--wait-parent PID] [PATH ...]`. A PATH
//! that is an existing note file (`.md`, `.markdown`, `.txt`) opens that note. A PATH that is an
//! existing folder becomes the vault. `resolve_launch` holds the rules, and `choose_startup` decides
//! between the command line, the last vault opened and the first-run chooser. The disk is behind
//! `LaunchFs`, so the rules are unit tested with a fake disk.

use std::path::Path;

/// Extensions of files that can be given on the command line.
const NOTE_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];
/// Folders whose presence marks a vault root.
const VAULT_MARKERS: &[&str] = &[".jiyue", ".obsidian"];
/// The path separator of this platform. Paths stay strings with this separator: `\` on Windows
/// (with drive letters and UNC paths), `/` on macOS and Linux.
const SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// What a path is on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

/// The disk queries of `resolve_launch`. `RealFs` asks the file system; tests use a fake.
pub trait LaunchFs {
    /// `Some` when the path exists, as a file or a folder.
    fn kind(&self, path: &str) -> Option<Kind>;
}

/// The real file system.
pub struct RealFs;

impl LaunchFs for RealFs {
    fn kind(&self, path: &str) -> Option<Kind> {
        let p = Path::new(path);
        if p.is_file() {
            Some(Kind::File)
        } else if p.is_dir() {
            Some(Kind::Dir)
        } else {
            None
        }
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The vault from the command line: the folder argument or `--vault`, or the vault of the first
    /// note file. `None` when the command line names no vault.
    pub vault: Option<String>,
    /// Notes to open, as vault-relative paths with forward slashes, in command line order.
    pub notes: Vec<String>,
    /// The `--theme` value, unchecked.
    pub theme: Option<String>,
    /// The `--open` value, unchecked.
    pub open: Option<String>,
}

/// Where the app starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Startup {
    /// Open the vault at this path.
    Open(String),
    /// No vault is known: show the first-run chooser.
    Chooser,
}

/// Works out the vault and the notes to open from the command line arguments (without the program
/// name). `cwd` is the working directory, used for relative paths.
///
/// Rules:
/// - A folder argument is the vault. It wins over `--vault`.
/// - A note file argument (existing, `.md`/`.markdown`/`.txt`) is opened. Other arguments are ignored.
/// - The vault of the notes: the explicit vault when the first note is inside it; otherwise the
///   note's own folder as a temporary vault when a vault was given; otherwise the nearest ancestor
///   folder with `.jiyue` or `.obsidian`; otherwise the note's folder.
/// - Notes outside the chosen vault are dropped (the first note decides the vault).
/// - Without a folder, `--vault` and without a note file there is no vault (`vault` is `None`).
pub fn resolve_launch(args: &[String], cwd: &str, fs: &dyn LaunchFs) -> LaunchPlan {
    let mut vault_arg: Option<String> = None;
    let mut theme: Option<String> = None;
    let mut open: Option<String> = None;
    let mut candidates: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--vault" | "--theme" | "--open" | "--wait-parent" if i + 1 < args.len() => {
                let value = args[i + 1].clone();
                match arg {
                    "--vault" => vault_arg = Some(value),
                    "--theme" => theme = Some(value),
                    "--open" => open = Some(value),
                    // Read by `vaults::parse_wait_parent`; it is not a path.
                    _ => {}
                }
                i += 2;
                continue;
            }
            // Unknown flags, and a flag with no value, are ignored.
            _ if arg.starts_with("--") => {}
            _ => candidates.push(arg),
        }
        i += 1;
    }

    let cwd = normalize(cwd);
    let mut folder: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    for raw in candidates {
        let path = absolute(raw, &cwd);
        match fs.kind(&path) {
            Some(Kind::File) if is_note_file(&path) => files.push(path),
            Some(Kind::Dir) if folder.is_none() => folder = Some(path),
            _ => {}
        }
    }
    let explicit = folder.or_else(|| vault_arg.as_deref().map(|v| absolute(v, &cwd)));

    let vault = match (files.first(), explicit) {
        (None, vault) => vault,
        (Some(first), Some(vault)) if relative_to(first, &vault).is_some() => Some(vault),
        (Some(first), Some(_)) => Some(parent_of(first)),
        (Some(first), None) => {
            Some(nearest_vault(&parent_of(first), fs).unwrap_or_else(|| parent_of(first)))
        }
    };
    let notes = match &vault {
        Some(root) => files
            .iter()
            .filter_map(|file| relative_to(file, root))
            .collect(),
        None => Vec::new(),
    };
    LaunchPlan {
        vault,
        notes,
        theme,
        open,
    }
}

/// Decides where the app starts. The order: the vault of the command line (`plan.vault`), then the
/// last vault opened when its folder still exists, then the first-run chooser.
pub fn choose_startup(plan: &LaunchPlan, last_opened: Option<&str>, fs: &dyn LaunchFs) -> Startup {
    if let Some(vault) = &plan.vault {
        return Startup::Open(vault.clone());
    }
    match last_opened {
        Some(last) if fs.kind(last) == Some(Kind::Dir) => Startup::Open(last.to_string()),
        _ => Startup::Chooser,
    }
}

/// The nearest folder from `start` upwards that contains a vault marker.
fn nearest_vault(start: &str, fs: &dyn LaunchFs) -> Option<String> {
    let mut dir = start.to_string();
    loop {
        let base = dir.trim_end_matches(SEP);
        if VAULT_MARKERS
            .iter()
            .any(|marker| fs.kind(&format!("{base}{SEP}{marker}")).is_some())
        {
            return Some(dir);
        }
        let up = parent_of(&dir);
        if up.len() >= dir.len() {
            return None;
        }
        dir = up;
    }
}

/// Path of `path` relative to `vault` with forward slashes, or `None` when `path` is not inside it.
/// The comparison ignores ASCII case, as Windows does.
fn relative_to(path: &str, vault: &str) -> Option<String> {
    let root = vault.trim_end_matches(SEP);
    let head = path.get(..root.len())?;
    let tail = path.get(root.len()..)?;
    let same = if cfg!(windows) {
        head.eq_ignore_ascii_case(root)
    } else {
        head == root
    };
    if !same || !tail.starts_with(SEP) || tail.len() < 2 {
        return None;
    }
    Some(tail[1..].replace(SEP, "/"))
}

/// The folder of a path. `C:\a.md` gives `C:\`.
fn parent_of(path: &str) -> String {
    match path.rfind(SEP) {
        Some(i) => {
            let dir = &path[..i];
            if dir.is_empty() || dir.ends_with(':') {
                format!("{dir}{SEP}")
            } else {
                dir.to_string()
            }
        }
        None => path.to_string(),
    }
}

/// True for `.md`, `.markdown` and `.txt` names.
fn is_note_file(path: &str) -> bool {
    let name = path.rsplit(SEP).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) => NOTE_EXTENSIONS
            .iter()
            .any(|ext| name[i + 1..].eq_ignore_ascii_case(ext)),
        None => false,
    }
}

/// Turns an argument into a path. On Windows: strips the `\\?\` verbatim prefix and uses
/// backslashes. Elsewhere the argument is kept as it is.
fn normalize(raw: &str) -> String {
    if !cfg!(windows) {
        return raw.to_string();
    }
    let s = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.to_string()
    };
    s.replace('/', "\\")
}

/// True for rooted paths, and on Windows also drive-letter and UNC paths.
fn is_absolute(n: &str) -> bool {
    let b = n.as_bytes();
    let drive = cfg!(windows)
        && b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'\\';
    n.starts_with(SEP) || drive
}

/// The absolute, normalised form of an argument, relative to `cwd`.
fn absolute(raw: &str, cwd: &str) -> String {
    let n = normalize(raw);
    if is_absolute(&n) {
        lexical(&n)
    } else {
        lexical(&format!("{}{SEP}{}", cwd.trim_end_matches(SEP), n))
    }
}

/// Removes `.` and `..` segments without touching the disk.
fn lexical(n: &str) -> String {
    let drive = if cfg!(windows)
        && n.as_bytes().get(1) == Some(&b':')
        && n.as_bytes().get(2) == Some(&b'\\')
    {
        n.get(..3)
    } else {
        None
    };
    let (prefix, rest): (&str, &str) =
        if let (true, Some(r)) = (cfg!(windows), n.strip_prefix(r"\\")) {
            (r"\\", r)
        } else if let (Some(drive), Some(r)) = (drive, n.get(3..)) {
            (drive, r)
        } else if let Some(r) = n.strip_prefix(SEP) {
            (if SEP == '\\' { "\\" } else { "/" }, r)
        } else {
            ("", n)
        };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split(SEP) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let joined = parts.join(if SEP == '\\' { "\\" } else { "/" });
    format!("{prefix}{joined}")
}

/// The Windows path rules, tested with backslash paths and drive letters. Windows only.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A fake disk: the listed paths exist with the given kind (ASCII case ignored).
    struct Disk(Vec<(&'static str, Kind)>);

    impl LaunchFs for Disk {
        fn kind(&self, path: &str) -> Option<Kind> {
            self.0
                .iter()
                .find(|(p, _)| p.eq_ignore_ascii_case(path))
                .map(|(_, k)| *k)
        }
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn explicit_vault_contains_the_file() {
        let disk = Disk(vec![
            ("D:\\notes", Kind::Dir),
            ("D:\\notes\\a.md", Kind::File),
        ]);
        let plan = resolve_launch(
            &args(&["--vault", "D:\\notes", "D:\\notes\\a.md"]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.vault.as_deref(), Some("D:\\notes"));
        assert_eq!(plan.notes, vec!["a.md"]);
    }

    #[test]
    fn file_outside_explicit_vault_uses_its_folder() {
        let disk = Disk(vec![
            ("D:\\notes", Kind::Dir),
            ("D:\\other\\b.md", Kind::File),
        ]);
        let plan = resolve_launch(
            &args(&["--vault", "D:\\notes", "D:\\other\\b.md"]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.vault.as_deref(), Some("D:\\other"));
        assert_eq!(plan.notes, vec!["b.md"]);
    }

    #[test]
    fn nearest_ancestor_with_a_marker_wins() {
        let disk = Disk(vec![
            ("D:\\v\\.obsidian", Kind::Dir),
            ("D:\\v\\a", Kind::Dir),
            ("D:\\v\\a\\.jiyue", Kind::Dir),
            ("D:\\v\\a\\b", Kind::Dir),
            ("D:\\v\\a\\b\\c.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["D:\\v\\a\\b\\c.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\v\\a"));
        assert_eq!(plan.notes, vec!["b/c.md"]);
    }

    #[test]
    fn without_a_marker_the_file_folder_is_the_vault() {
        let disk = Disk(vec![("D:\\loose\\x.md", Kind::File)]);
        let plan = resolve_launch(&args(&["D:\\loose\\x.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\loose"));
        assert_eq!(plan.notes, vec!["x.md"]);
    }

    #[test]
    fn nonexistent_path_is_ignored() {
        let disk = Disk(vec![]);
        let plan = resolve_launch(&args(&["D:\\missing\\n.md"]), "C:\\", &disk);
        assert_eq!(plan.vault, None);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn folder_argument_is_the_vault_and_wins_over_vault_flag() {
        let disk = Disk(vec![
            ("D:\\books", Kind::Dir),
            ("D:\\books\\n.md", Kind::File),
            ("D:\\x", Kind::Dir),
        ]);
        let plan = resolve_launch(&args(&["D:\\books"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\books"));
        assert!(plan.notes.is_empty());
        let plan = resolve_launch(&args(&["--vault", "D:\\x", "D:\\books"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\books"));
        let plan = resolve_launch(&args(&["D:\\books", "D:\\books\\n.md"]), "C:\\", &disk);
        assert_eq!(plan.notes, vec!["n.md"]);
    }

    #[test]
    fn relative_paths_resolve_against_the_working_directory() {
        let disk = Disk(vec![
            ("D:\\work", Kind::Dir),
            ("D:\\work\\note.md", Kind::File),
            ("D:\\v", Kind::Dir),
            ("D:\\v\\n.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["./note.md"]), "D:\\work", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\work"));
        assert_eq!(plan.notes, vec!["note.md"]);
        let plan = resolve_launch(&args(&["..\\v\\n.md"]), "D:\\work", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\v"));
        assert_eq!(plan.notes, vec!["n.md"]);
        let plan = resolve_launch(
            &args(&["--vault", "..\\v", "..\\v\\n.md"]),
            "D:\\work",
            &disk,
        );
        assert_eq!(plan.vault.as_deref(), Some("D:\\v"));
        assert_eq!(plan.notes, vec!["n.md"]);
    }

    #[test]
    fn paths_with_spaces_and_cjk_characters() {
        let disk = Disk(vec![
            ("D:\\我的 笔记", Kind::Dir),
            ("D:\\我的 笔记\\日记 2026.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["D:\\我的 笔记\\日记 2026.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\我的 笔记"));
        assert_eq!(plan.notes, vec!["日记 2026.md"]);
        let plan = resolve_launch(
            &args(&["--vault", "D:\\我的 笔记", "D:\\我的 笔记\\日记 2026.md"]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.notes, vec!["日记 2026.md"]);
    }

    #[test]
    fn unc_paths_and_drive_letter_case() {
        let disk = Disk(vec![
            ("\\\\server\\share\\notes\\.jiyue", Kind::Dir),
            ("\\\\server\\share\\notes\\a.md", Kind::File),
            ("D:\\notes\\a.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["\\\\server\\share\\notes\\a.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("\\\\server\\share\\notes"));
        assert_eq!(plan.notes, vec!["a.md"]);
        // Forward slashes and a different drive-letter case still match the explicit vault.
        let plan = resolve_launch(
            &args(&["--vault", "d:/NOTES", "D:\\notes\\a.md"]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.vault.as_deref(), Some("d:\\NOTES"));
        assert_eq!(plan.notes, vec!["a.md"]);
    }

    #[test]
    fn verbatim_prefix_is_normalised() {
        let disk = Disk(vec![
            ("D:\\notes\\a.md", Kind::File),
            ("\\\\server\\share\\notes\\b.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["\\\\?\\D:\\notes\\a.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\notes"));
        assert_eq!(plan.notes, vec!["a.md"]);
        let plan = resolve_launch(
            &args(&["\\\\?\\UNC\\server\\share\\notes\\b.md"]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.vault.as_deref(), Some("\\\\server\\share\\notes"));
        assert_eq!(plan.notes, vec!["b.md"]);
    }

    #[test]
    fn flags_are_read_and_only_note_extensions_open() {
        let disk = Disk(vec![
            ("D:\\n", Kind::Dir),
            ("D:\\n\\a.exe", Kind::File),
            ("D:\\n\\b.txt", Kind::File),
            ("D:\\n\\c.markdown", Kind::File),
        ]);
        let plan = resolve_launch(
            &args(&[
                "--theme",
                "paper",
                "--open",
                "editor",
                "D:\\n\\a.exe",
                "D:\\n\\b.txt",
                "D:\\n\\c.markdown",
            ]),
            "C:\\",
            &disk,
        );
        assert_eq!(plan.theme.as_deref(), Some("paper"));
        assert_eq!(plan.open.as_deref(), Some("editor"));
        assert_eq!(plan.vault.as_deref(), Some("D:\\n"));
        assert_eq!(plan.notes, vec!["b.txt", "c.markdown"]);
    }

    #[test]
    fn several_files_in_one_folder_open_together() {
        let disk = Disk(vec![
            ("D:\\a\\x.md", Kind::File),
            ("D:\\a\\y.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["D:\\a\\x.md", "D:\\a\\y.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\a"));
        assert_eq!(plan.notes, vec!["x.md", "y.md"]);
    }

    #[test]
    fn files_outside_the_first_files_vault_are_dropped() {
        let disk = Disk(vec![
            ("D:\\a\\x.md", Kind::File),
            ("D:\\b\\y.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["D:\\a\\x.md", "D:\\b\\y.md"]), "C:\\", &disk);
        assert_eq!(plan.vault.as_deref(), Some("D:\\a"));
        assert_eq!(plan.notes, vec!["x.md"]);
    }

    #[test]
    fn no_arguments_name_no_vault() {
        let disk = Disk(vec![]);
        let plan = resolve_launch(&args(&[]), "C:\\", &disk);
        assert_eq!(plan.vault, None);
        assert!(plan.notes.is_empty());
        assert_eq!(plan.theme, None);
    }

    #[test]
    fn lexical_removes_dot_segments() {
        assert_eq!(lexical("D:\\a\\.\\b\\..\\c.md"), "D:\\a\\c.md");
        assert_eq!(lexical("D:\\"), "D:\\");
        assert_eq!(lexical("\\\\server\\share\\x"), "\\\\server\\share\\x");
    }

    #[test]
    fn startup_order_is_command_line_then_last_then_chooser() {
        let disk = Disk(vec![
            ("D:\\last", Kind::Dir),
            ("D:\\gone-file", Kind::File),
            ("D:\\x", Kind::Dir),
            ("D:\\a\\n.md", Kind::File),
        ]);
        // An explicit folder wins over the remembered vault.
        let plan = resolve_launch(&args(&["--vault", "D:\\x"]), "C:\\", &disk);
        assert_eq!(
            choose_startup(&plan, Some("D:\\last"), &disk),
            Startup::Open("D:\\x".into())
        );
        // A note file decides the vault before the remembered one.
        let plan = resolve_launch(&args(&["D:\\a\\n.md"]), "C:\\", &disk);
        assert_eq!(
            choose_startup(&plan, Some("D:\\last"), &disk),
            Startup::Open("D:\\a".into())
        );
        // Nothing on the command line: the last vault, if its folder exists.
        let plan = resolve_launch(&args(&[]), "C:\\", &disk);
        assert_eq!(
            choose_startup(&plan, Some("D:\\last"), &disk),
            Startup::Open("D:\\last".into())
        );
        // A missing or non-folder last vault gives the chooser.
        assert_eq!(
            choose_startup(&plan, Some("D:\\nope"), &disk),
            Startup::Chooser
        );
        assert_eq!(
            choose_startup(&plan, Some("D:\\gone-file"), &disk),
            Startup::Chooser
        );
        assert_eq!(choose_startup(&plan, None, &disk), Startup::Chooser);
    }
}

/// The same rules with `/` paths, for macOS and Linux.
#[cfg(all(test, not(windows)))]
mod tests_unix {
    use super::*;

    struct Disk(Vec<(&'static str, Kind)>);

    impl LaunchFs for Disk {
        fn kind(&self, path: &str) -> Option<Kind> {
            self.0.iter().find(|(p, _)| *p == path).map(|(_, k)| *k)
        }
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn note_opens_in_the_vault_marked_by_obsidian_folder() {
        let disk = Disk(vec![
            ("/home/ann/notes/.obsidian", Kind::Dir),
            ("/home/ann/notes/a.md", Kind::File),
        ]);
        let plan = resolve_launch(&args(&["/home/ann/notes/a.md"]), "/", &disk);
        assert_eq!(plan.vault.as_deref(), Some("/home/ann/notes"));
        assert_eq!(plan.notes, vec!["a.md"]);
    }

    #[test]
    fn relative_note_resolves_against_the_working_directory() {
        let disk = Disk(vec![("/work/n/a.md", Kind::File)]);
        let plan = resolve_launch(&args(&["a.md"]), "/work/n", &disk);
        assert_eq!(plan.vault.as_deref(), Some("/work/n"));
        assert_eq!(plan.notes, vec!["a.md"]);
    }

    #[test]
    fn folder_argument_is_the_vault() {
        let disk = Disk(vec![("/data/vault", Kind::Dir)]);
        let plan = resolve_launch(&args(&["/data/vault"]), "/", &disk);
        assert_eq!(plan.vault.as_deref(), Some("/data/vault"));
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn other_files_are_ignored() {
        let disk = Disk(vec![("/x/a.png", Kind::File)]);
        let plan = resolve_launch(&args(&["/x/a.png"]), "/", &disk);
        assert_eq!(plan.vault, None);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn lexical_removes_dot_segments() {
        assert_eq!(lexical("/a/./b/../c.md"), "/a/c.md");
        assert_eq!(lexical("/"), "/");
    }
}
