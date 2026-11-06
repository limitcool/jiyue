//! Per-OS folders of the app: configuration (keymap, crash log) and cache.
//!
//! Windows keeps the layout it has always used: everything under `%LOCALAPPDATA%\Jiyue`, with the
//! cache in `cache`. macOS uses `~/Library/Application Support/Jiyue` and `~/Library/Caches/Jiyue`.
//! Linux and other Unix systems follow the XDG base directories: `$XDG_CONFIG_HOME/jiyue` (or
//! `~/.config/jiyue`) and `$XDG_CACHE_HOME/jiyue` (or `~/.cache/jiyue`).
//!
//! The mapping is a pure function of an [`Env`] snapshot so that every OS can be unit-tested on
//! every machine. When no home folder can be found, the temp folder is used, as the app did before.

use std::path::PathBuf;

/// Operating system families the mapping knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    /// Linux and the other Unix systems (XDG rules).
    Unix,
}

/// The OS this binary was built for.
pub fn current_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Unix
    }
}

/// The environment values the mapping reads. Empty or relative XDG values are ignored, as the
/// XDG specification asks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    pub temp: PathBuf,
}

impl Env {
    /// Reads the snapshot from the process environment.
    pub fn from_process() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        Self {
            home,
            local_app_data: non_empty_var("LOCALAPPDATA"),
            xdg_config_home: absolute_var("XDG_CONFIG_HOME"),
            xdg_cache_home: absolute_var("XDG_CACHE_HOME"),
            temp: std::env::temp_dir(),
        }
    }
}

fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn absolute_var(name: &str) -> Option<PathBuf> {
    non_empty_var(name).filter(|p| p.is_absolute())
}

/// Folder for configuration files (`config.toml`, `state.toml`, `credentials.toml`, `crash.log`).
pub fn config_dir_for(os: Os, env: &Env) -> PathBuf {
    match os {
        Os::Windows => env
            .local_app_data
            .clone()
            .unwrap_or_else(|| env.temp.clone())
            .join("Jiyue"),
        Os::MacOs => match &env.home {
            Some(home) => home
                .join("Library")
                .join("Application Support")
                .join("Jiyue"),
            None => env.temp.join("Jiyue"),
        },
        Os::Unix => match env
            .xdg_config_home
            .clone()
            .or_else(|| env.home.as_ref().map(|h| h.join(".config")))
        {
            Some(base) => base.join("jiyue"),
            None => env.temp.join("Jiyue"),
        },
    }
}

/// Folder for the app's cache. On Windows it is `<config>\cache`, as before.
pub fn cache_dir_for(os: Os, env: &Env) -> PathBuf {
    match os {
        Os::Windows => config_dir_for(os, env).join("cache"),
        Os::MacOs => match &env.home {
            Some(home) => home.join("Library").join("Caches").join("Jiyue"),
            None => env.temp.join("Jiyue").join("cache"),
        },
        Os::Unix => match env
            .xdg_cache_home
            .clone()
            .or_else(|| env.home.as_ref().map(|h| h.join(".cache")))
        {
            Some(base) => base.join("jiyue"),
            None => env.temp.join("Jiyue").join("cache"),
        },
    }
}

/// Configuration folder of this machine.
pub fn config_dir() -> PathBuf {
    config_dir_for(current_os(), &Env::from_process())
}

/// Cache folder of this machine.
pub fn cache_dir() -> PathBuf {
    cache_dir_for(current_os(), &Env::from_process())
}

/// The user's Documents folder: `%USERPROFILE%\Documents` on Windows, `~/Documents` elsewhere.
/// The temp folder stands in when no home folder can be found.
pub fn documents_dir_for(env: &Env) -> PathBuf {
    match &env.home {
        Some(home) => home.join("Documents"),
        None => env.temp.join("Documents"),
    }
}

/// The user's Documents folder of this machine (see `documents_dir_for`).
pub fn documents_dir() -> PathBuf {
    documents_dir_for(&Env::from_process())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            home: Some(PathBuf::from("/home/ann")),
            local_app_data: Some(PathBuf::from("C:/Users/ann/AppData/Local")),
            xdg_config_home: None,
            xdg_cache_home: None,
            temp: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn documents_folder_sits_in_the_home_folder() {
        assert_eq!(
            documents_dir_for(&env()),
            PathBuf::from("/home/ann").join("Documents")
        );
        let no_home = Env {
            home: None,
            temp: PathBuf::from("/tmp"),
            ..env()
        };
        assert_eq!(
            documents_dir_for(&no_home),
            PathBuf::from("/tmp").join("Documents")
        );
    }

    #[test]
    fn windows_layout_is_unchanged() {
        let e = env();
        assert_eq!(
            config_dir_for(Os::Windows, &e),
            PathBuf::from("C:/Users/ann/AppData/Local").join("Jiyue")
        );
        assert_eq!(
            cache_dir_for(Os::Windows, &e),
            PathBuf::from("C:/Users/ann/AppData/Local")
                .join("Jiyue")
                .join("cache")
        );
    }

    #[test]
    fn windows_without_local_app_data_uses_temp() {
        let e = Env {
            local_app_data: None,
            ..env()
        };
        assert_eq!(
            config_dir_for(Os::Windows, &e),
            PathBuf::from("/tmp").join("Jiyue")
        );
    }

    #[test]
    fn macos_uses_library_folders() {
        let e = env();
        assert_eq!(
            config_dir_for(Os::MacOs, &e),
            PathBuf::from("/home/ann/Library/Application Support/Jiyue")
        );
        assert_eq!(
            cache_dir_for(Os::MacOs, &e),
            PathBuf::from("/home/ann/Library/Caches/Jiyue")
        );
    }

    #[test]
    fn unix_defaults_to_home_dot_folders() {
        let e = env();
        assert_eq!(
            config_dir_for(Os::Unix, &e),
            PathBuf::from("/home/ann/.config/jiyue")
        );
        assert_eq!(
            cache_dir_for(Os::Unix, &e),
            PathBuf::from("/home/ann/.cache/jiyue")
        );
    }

    #[test]
    fn unix_honours_absolute_xdg_variables() {
        let e = Env {
            xdg_config_home: Some(PathBuf::from("/cfg")),
            xdg_cache_home: Some(PathBuf::from("/cache")),
            ..env()
        };
        assert_eq!(config_dir_for(Os::Unix, &e), PathBuf::from("/cfg/jiyue"));
        assert_eq!(cache_dir_for(Os::Unix, &e), PathBuf::from("/cache/jiyue"));
    }

    #[test]
    fn no_home_falls_back_to_temp() {
        let e = Env {
            home: None,
            ..env()
        };
        assert_eq!(
            config_dir_for(Os::Unix, &e),
            PathBuf::from("/tmp").join("Jiyue")
        );
        assert_eq!(
            config_dir_for(Os::MacOs, &e),
            PathBuf::from("/tmp").join("Jiyue")
        );
    }
}
