//! Internationalization (i18n) module for Jiyue based on Project Fluent.
//!
//! Provides compile-time embedded Fluent translations, zero-cost fallback,
//! runtime dynamic locale switching, and ergonomic `t!` macros.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

pub const LOCALE_ZH_CN: &str = "zh-CN";
pub const LOCALE_EN_US: &str = "en-US";
pub const LOCALE_AUTO: &str = "auto";

const FTL_ZH_CN: &str = include_str!("../resources/locales/zh-CN/app.ftl");
const FTL_EN_US: &str = include_str!("../resources/locales/en-US/app.ftl");

/// Supported locales and their human-readable labels.
pub const SUPPORTED_LOCALES: [(&str, &str); 3] = [
    (LOCALE_AUTO, "跟随系统 / System"),
    (LOCALE_ZH_CN, "简体中文 (zh-CN)"),
    (LOCALE_EN_US, "English (en-US)"),
];

/// Global i18n manager.
pub struct I18nManager {
    configured_locale: RwLock<String>,
    current_locale: RwLock<String>,
    bundles: HashMap<String, FluentBundle<FluentResource>>,
}

impl I18nManager {
    pub fn new() -> Self {
        let mut bundles = HashMap::new();

        // 1. zh-CN bundle
        let lang_zh: LanguageIdentifier = LOCALE_ZH_CN.parse().expect("valid zh-CN langid");
        let mut bundle_zh = FluentBundle::new_concurrent(vec![lang_zh]);
        bundle_zh.set_use_isolating(false);
        let res_zh = FluentResource::try_new(FTL_ZH_CN.to_string())
            .expect("Failed to parse embedded zh-CN.ftl");
        bundle_zh
            .add_resource(res_zh)
            .expect("Failed to add zh-CN resource");
        bundles.insert(LOCALE_ZH_CN.to_string(), bundle_zh);

        // 2. en-US bundle
        let lang_en: LanguageIdentifier = LOCALE_EN_US.parse().expect("valid en-US langid");
        let mut bundle_en = FluentBundle::new_concurrent(vec![lang_en]);
        bundle_en.set_use_isolating(false);
        let res_en = FluentResource::try_new(FTL_EN_US.to_string())
            .expect("Failed to parse embedded en-US.ftl");
        bundle_en
            .add_resource(res_en)
            .expect("Failed to add en-US resource");
        bundles.insert(LOCALE_EN_US.to_string(), bundle_en);

        Self {
            configured_locale: RwLock::new(LOCALE_AUTO.to_string()),
            current_locale: RwLock::new(LOCALE_ZH_CN.to_string()),
            bundles,
        }
    }

    /// Sets the current active locale. If "auto", inspects system locale.
    pub fn set_locale(&self, locale: &str) {
        let conf = if locale.is_empty() {
            LOCALE_AUTO
        } else {
            locale
        };
        let target = if conf == LOCALE_AUTO {
            detect_system_locale()
        } else {
            conf.to_string()
        };

        if let Ok(mut lock) = self.configured_locale.write() {
            *lock = conf.to_string();
        }
        if let Ok(mut lock) = self.current_locale.write() {
            *lock = target;
        }
    }

    /// Gets user-configured locale string ("auto", "zh-CN", "en-US").
    pub fn configured_locale(&self) -> String {
        self.configured_locale
            .read()
            .as_deref()
            .cloned()
            .unwrap_or_else(|_| LOCALE_AUTO.to_string())
    }

    /// Gets current effective locale string ("zh-CN", "en-US").
    pub fn current_locale(&self) -> String {
        self.current_locale
            .read()
            .as_deref()
            .cloned()
            .unwrap_or_else(|_| LOCALE_ZH_CN.to_string())
    }

    /// Translates a message key with optional arguments.
    /// Falls back to zh-CN if translation is missing in the current locale.
    pub fn translate(&self, key: &str, args: Option<&FluentArgs>) -> String {
        let cur = self.current_locale();

        // Try current bundle
        if let Some(bundle) = self.bundles.get(&cur)
            && let Some(msg) = bundle.get_message(key)
            && let Some(pattern) = msg.value()
        {
            let mut errors = vec![];
            let val = bundle.format_pattern(pattern, args, &mut errors);
            return val.to_string();
        }

        // Fallback to zh-CN
        if cur != LOCALE_ZH_CN
            && let Some(bundle) = self.bundles.get(LOCALE_ZH_CN)
            && let Some(msg) = bundle.get_message(key)
            && let Some(pattern) = msg.value()
        {
            let mut errors = vec![];
            let val = bundle.format_pattern(pattern, args, &mut errors);
            return val.to_string();
        }

        // Ultimate fallback: key itself
        key.to_string()
    }
}

impl Default for I18nManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Detects the system language on Windows/macOS/Linux.
fn detect_system_locale() -> String {
    // Check standard env vars first
    for var in &["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_ascii_lowercase();
            if lower.starts_with("zh") {
                return LOCALE_ZH_CN.to_string();
            } else if lower.starts_with("en") {
                return LOCALE_EN_US.to_string();
            }
        }
    }

    // Default to zh-CN
    LOCALE_ZH_CN.to_string()
}

static GLOBAL_I18N: OnceLock<I18nManager> = OnceLock::new();

pub fn manager() -> &'static I18nManager {
    GLOBAL_I18N.get_or_init(I18nManager::new)
}

pub fn set_locale(locale: &str) {
    manager().set_locale(locale);
}

pub fn configured_locale() -> String {
    manager().configured_locale()
}

pub fn current_locale() -> String {
    manager().current_locale()
}

pub fn translate(key: &str, args: Option<&FluentArgs>) -> String {
    manager().translate(key, args)
}

/// Shorthand macro for translation.
///
/// # Examples
/// ```rust
/// let text = t!("sidebar-new-note");
/// let formatted = t!("update-available", version = "0.2.0");
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::translate($key, None)
    };
    ($key:expr, $($arg_name:ident = $arg_val:expr),* $(,)?) => {{
        let mut args = ::fluent_bundle::FluentArgs::new();
        $(
            args.set(stringify!($arg_name), $arg_val);
        )*
        $crate::i18n::translate($key, Some(&args))
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i18n_translation() {
        let mgr = I18nManager::new();
        mgr.set_locale(LOCALE_ZH_CN);
        assert_eq!(mgr.translate("sidebar-new-note", None), "新建笔记");

        mgr.set_locale(LOCALE_EN_US);
        assert_eq!(mgr.translate("sidebar-new-note", None), "New Note");

        // Test arguments
        let mut args = FluentArgs::new();
        args.set("version", "1.0.0");
        assert_eq!(
            mgr.translate("update-available", Some(&args)),
            "New version available: v1.0.0"
        );

        // Fallback test
        assert_eq!(mgr.translate("non-existent-key", None), "non-existent-key");
    }
}
