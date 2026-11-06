//! Chinese messages for provider errors, and validation of the provider form.
//!
//! Pure functions only, so they are unit-tested without a window.

use jiyue_ai::{AiConfig, AiError, ProviderEntry, ProviderKind};

/// Longest server text shown to the user.
const MAX_DETAIL_CHARS: usize = 120;

/// One-line Chinese description of a provider error, shown inline and in toasts.
pub fn describe(err: &AiError) -> String {
    match err {
        AiError::Unauthorized => "未授权：API 密钥无效或没有权限，请在设置中检查密钥".to_string(),
        AiError::RateLimited { retry_after } => match retry_after {
            Some(d) => format!("被限流：请求过于频繁，约 {} 秒后可重试", d.as_secs().max(1)),
            None => "被限流：请求过于频繁，请稍后重试".to_string(),
        },
        AiError::Network(m) => format!("网络错误：无法连接到服务（{}）", detail(m)),
        AiError::Server(code, m) => format!("服务器错误（{code}）：{}", detail(m)),
        AiError::BadRequest(m) => format!("请求被服务拒绝：{}", detail(m)),
        AiError::BadResponse(m) => format!("服务返回的内容无法解析：{}", detail(m)),
        AiError::Cancelled => "已停止".to_string(),
        AiError::Other(m) => detail(m),
    }
}

/// Every error except a deliberate stop can be retried from the UI.
pub fn can_retry(err: &AiError) -> bool {
    !matches!(err, AiError::Cancelled)
}

fn detail(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return "未知原因".to_string();
    }
    if t.chars().count() <= MAX_DETAIL_CHARS {
        t.to_string()
    } else {
        let cut: String = t.chars().take(MAX_DETAIL_CHARS).collect();
        format!("{cut}…")
    }
}

/// Input of the provider editor, before validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderForm {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub chat_model: String,
    pub embedding_model: String,
}

/// Checks the provider form and builds the entry. `id` is the stable id of the entry (new or
/// existing). Errors are Chinese messages for the form.
pub fn validate_provider(id: &str, form: &ProviderForm) -> Result<ProviderEntry, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("请填写服务名称".into());
    }
    if name.chars().count() > 40 {
        return Err("服务名称最多 40 个字符".into());
    }
    let raw = form.base_url.trim();
    if !(raw.starts_with("https://") || raw.starts_with("http://")) {
        return Err("服务地址必须以 https:// 或 http:// 开头".into());
    }
    let base = raw.trim_end_matches('/');
    if base.len() <= "https://".len() {
        return Err("服务地址不完整".into());
    }
    let chat = form.chat_model.trim();
    if chat.is_empty() {
        return Err("请填写对话模型名称".into());
    }
    let embedding = form.embedding_model.trim();
    let entry = ProviderEntry {
        id: id.to_string(),
        name: name.to_string(),
        kind: form.kind,
        base_url: base.to_string(),
        chat_model: chat.to_string(),
        embedding_model: if embedding.is_empty() {
            None
        } else {
            Some(embedding.to_string())
        },
    };
    Ok(entry)
}

/// Picks an unused provider id (`p1`, `p2`, ...).
pub fn new_provider_id(config: &AiConfig) -> String {
    (1..)
        .map(|n| format!("p{n}"))
        .find(|candidate| !config.providers.iter().any(|p| &p.id == candidate))
        .unwrap_or_else(|| "p".to_string())
}

/// Normalises a folder typed in the settings. Returns `None` when nothing is left.
pub fn normalize_folder(input: &str) -> Option<String> {
    let f = input.trim().replace('\\', "/");
    let f = f.trim_matches('/').trim();
    if f.is_empty() {
        None
    } else {
        Some(f.to_string())
    }
}

/// Normalises a tag typed in the settings (without `#`). Returns `None` when nothing is left.
pub fn normalize_tag(input: &str) -> Option<String> {
    let t = input.trim().trim_start_matches('#').trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        None
    } else {
        Some(t.to_string())
    }
}

/// Adds `item` to `list` unless an equal item (case-insensitive) is already there.
pub fn push_unique_ci(list: &mut Vec<String>, item: String) -> bool {
    if list.iter().any(|x| x.to_lowercase() == item.to_lowercase()) {
        false
    } else {
        list.push(item);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn form() -> ProviderForm {
        ProviderForm {
            name: " 本地 ".into(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:8080/v1/".into(),
            chat_model: " gpt-test ".into(),
            embedding_model: "  ".into(),
        }
    }

    #[test]
    fn errors_map_to_chinese_categories() {
        assert!(describe(&AiError::Unauthorized).starts_with("未授权"));
        assert!(describe(&AiError::RateLimited { retry_after: None }).starts_with("被限流"),);
        let with_delay = describe(&AiError::RateLimited {
            retry_after: Some(Duration::from_secs(7)),
        });
        assert!(with_delay.contains("7 秒"), "{with_delay}");
        assert!(describe(&AiError::Network("timeout".into())).starts_with("网络错误"));
        assert!(describe(&AiError::Server(500, "boom".into())).starts_with("服务器错误（500）"));
        assert_eq!(describe(&AiError::Cancelled), "已停止");
    }

    #[test]
    fn long_server_text_is_cut() {
        let long = "x".repeat(500);
        let msg = describe(&AiError::BadRequest(long));
        assert!(msg.chars().count() < 200);
        assert!(msg.ends_with('…'));
        assert_eq!(describe(&AiError::Other("  ".into())), "未知原因");
    }

    #[test]
    fn cancel_is_not_retryable_others_are() {
        assert!(!can_retry(&AiError::Cancelled));
        assert!(can_retry(&AiError::Unauthorized));
        assert!(can_retry(&AiError::Network("x".into())));
    }

    #[test]
    fn valid_form_builds_entry_with_trimmed_fields() {
        let entry = validate_provider("p1", &form()).expect("valid");
        assert_eq!(entry.name, "本地");
        assert_eq!(entry.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(entry.chat_model, "gpt-test");
        assert_eq!(entry.embedding_model, None);
    }

    #[test]
    fn invalid_forms_name_the_problem() {
        let mut f = form();
        f.name = "  ".into();
        assert_eq!(validate_provider("p1", &f), Err("请填写服务名称".into()));
        let mut f = form();
        f.base_url = "ftp://x".into();
        assert!(validate_provider("p1", &f).is_err());
        let mut f = form();
        f.base_url = "https://".into();
        assert_eq!(validate_provider("p1", &f), Err("服务地址不完整".into()));
        let mut f = form();
        f.chat_model = "".into();
        assert_eq!(
            validate_provider("p1", &f),
            Err("请填写对话模型名称".into())
        );
        let mut f = form();
        f.name = "名".repeat(41);
        assert!(validate_provider("p1", &f).is_err());
    }

    #[test]
    fn new_ids_skip_used_ones() {
        let mut cfg = AiConfig::default();
        assert_eq!(new_provider_id(&cfg), "p1");
        let entry = validate_provider("p1", &form()).expect("valid");
        cfg.providers.push(entry);
        assert_eq!(new_provider_id(&cfg), "p2");
    }

    #[test]
    fn folders_and_tags_are_normalised() {
        assert_eq!(
            normalize_folder(" \\私人\\子目录/ "),
            Some("私人/子目录".into())
        );
        assert_eq!(normalize_folder("  /  "), None);
        assert_eq!(normalize_tag(" #secret "), Some("secret".into()));
        assert_eq!(normalize_tag("two words"), None);
        assert_eq!(normalize_tag("#"), None);
    }

    #[test]
    fn unique_push_ignores_case() {
        let mut list = vec!["Secret".to_string()];
        assert!(!push_unique_ci(&mut list, "secret".into()));
        assert!(push_unique_ci(&mut list, "other".into()));
        assert_eq!(list.len(), 2);
    }
}
