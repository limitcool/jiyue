//! Cross-device configuration export for mobile scanning (S3 & AI settings).

use std::path::Path;

use gpui::{AnyElement, Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};
use qrcode::{EcLevel, QrCode};
use serde::{Deserialize, Serialize};

use crate::RustdianApp;
use crate::ui::icons::icon;
use crate::ui::widgets::{ButtonKind, button, modal};

/// Unified configuration export payload for Jiyue mobile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedConfig {
    pub jiyue: String,
    pub version: u32,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ExportedSyncConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<ExportedAiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSyncConfig {
    pub backend: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub root_prefix: String,
    pub force_path_style: bool,
    // WebDAV
    pub username: String,
    pub password: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedAiConfig {
    pub enabled: bool,
    pub active_chat: Option<String>,
    pub providers: Vec<ExportedAiProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedAiProvider {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub chat_model: String,
    pub embedding_model: Option<String>,
    pub api_key: String,
}

/// Generates a boolean 2D matrix representing QR modules (true = dark, false = light).
pub fn generate_qr_matrix(text: &str) -> Result<Vec<Vec<bool>>, String> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .or_else(|_| QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L))
        .map_err(|e| format!("生成二维码失败: {e}"))?;

    let width = code.width();
    let colors = code.to_colors();
    let mut matrix = Vec::with_capacity(width);
    for y in 0..width {
        let mut row = Vec::with_capacity(width);
        for x in 0..width {
            let dark = colors[y * width + x] == qrcode::Color::Dark;
            row.push(dark);
        }
        matrix.push(row);
    }
    Ok(matrix)
}

/// Collects and serializes active configurations into an export struct.
pub fn collect_export_config(
    vault_root: Option<&Path>,
    include_sync: bool,
    include_ai: bool,
) -> ExportedConfig {
    let sync = if include_sync {
        vault_root.and_then(|root| {
            let mut cfg = super::sync_ui::config::SyncConfig::load(root).ok()?;
            super::sync_ui::secrets::fill_identity(&mut cfg);
            let secret = super::sync_ui::secrets::load(&cfg)
                .ok()
                .flatten()
                .unwrap_or_default();
            Some(ExportedSyncConfig {
                backend: cfg.backend.key().to_string(),
                endpoint: if cfg.backend == super::sync_ui::config::Backend::S3 {
                    cfg.s3_endpoint
                } else {
                    cfg.webdav_endpoint
                },
                region: cfg.s3_region,
                bucket: cfg.s3_bucket,
                access_key_id: cfg.s3_access_key_id,
                secret_access_key: if cfg.backend == super::sync_ui::config::Backend::S3 {
                    secret.clone()
                } else {
                    String::new()
                },
                root_prefix: cfg.s3_root_prefix,
                force_path_style: cfg.s3_path_style,
                username: cfg.webdav_username,
                password: if cfg.backend == super::sync_ui::config::Backend::WebDav {
                    secret
                } else {
                    String::new()
                },
                root: cfg.webdav_root,
            })
        })
    } else {
        None
    };

    let ai = if include_ai {
        vault_root.map(|root| {
            let loaded: jiyue_core::config::Loaded<jiyue_ai::AiConfig> =
                jiyue_core::config::load_section(
                    &jiyue_core::config::vault_file(root, jiyue_core::config::VAULT_CONFIG_FILE),
                    "ai",
                );
            let ai_cfg = loaded.value;
            let creds = jiyue_core::config::credentials::load();
            let providers: Vec<ExportedAiProvider> = ai_cfg
                .providers
                .into_iter()
                .map(|p| {
                    let api_key = creds
                        .value
                        .ai
                        .get(&p.id)
                        .map(|c| c.api_key.clone())
                        .and_then(|raw| {
                            jiyue_core::config::secret::resolve_secret(&raw)
                                .ok()
                                .map(|s| s.expose().to_string())
                        })
                        .unwrap_or_default();
                    ExportedAiProvider {
                        id: p.id,
                        name: p.name,
                        kind: format!("{:?}", p.kind).to_lowercase(),
                        base_url: p.base_url,
                        chat_model: p.chat_model,
                        embedding_model: p.embedding_model,
                        api_key,
                    }
                })
                .collect();

            ExportedAiConfig {
                enabled: ai_cfg.enabled,
                active_chat: ai_cfg.active_chat,
                providers,
            }
        })
    } else {
        None
    };

    ExportedConfig {
        jiyue: "config_export".into(),
        version: 1,
        timestamp: 0,
        sync,
        ai,
    }
}

/// Renders the QR code matrix with crisp black & white GPUI divs.
fn render_qr_grid(matrix: &[Vec<bool>]) -> AnyElement {
    let width = matrix.len();
    if width == 0 {
        return div().into_any_element();
    }

    // Size QR code nicely around 240px
    let cell_px = (240.0 / width as f32).max(2.0);
    let cell_size = px(cell_px);

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(px(12.0))
        .bg(gpui::white())
        .rounded(px(12.0))
        .shadow_md()
        .border_1()
        .border_color(gpui::rgb(0xe0e0e0))
        .children(matrix.iter().map(|row| {
            div().flex().flex_row().children(row.iter().map(|&dark| {
                div()
                    .size(cell_size)
                    .bg(if dark { gpui::black() } else { gpui::white() })
            }))
        }))
        .into_any_element()
}

/// Renders the export configuration modal dialog.
pub fn render_modal(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let vault_root = app.vault.root_dir.as_path();
    let include_sync = app.settings.export_include_sync;
    let include_ai = app.settings.export_include_ai;

    let payload = collect_export_config(Some(vault_root), include_sync, include_ai);
    let json_text = serde_json::to_string(&payload).unwrap_or_default();
    let qr_matrix = generate_qr_matrix(&json_text);

    let copy_json = json_text.clone();

    let body = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(16.0))
        .w_full()
        .font_family(crate::ui::fonts::ui_name())
        .child(div().text_size(px(13.0)).text_color(t.tx2).child(
            "使用 霁月 手机端「设置 → 跨端配置导入」扫描下方二维码，自动同步 S3 存储及 AI 设置。",
        ))
        // Checkboxes row for toggling sync and ai
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(20.0))
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .cursor_pointer()
                        .child(icon(
                            if include_sync { "check" } else { "circle" },
                            16.0,
                            if include_sync { t.ac } else { t.tx2 },
                        ))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(if include_sync { t.tx } else { t.tx2 })
                                .child("S3 云端同步配置"),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.settings.export_include_sync =
                                    !this.settings.export_include_sync;
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .cursor_pointer()
                        .child(icon(
                            if include_ai { "check" } else { "circle" },
                            16.0,
                            if include_ai { t.ac } else { t.tx2 },
                        ))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(if include_ai { t.tx } else { t.tx2 })
                                .child("AI 助手配置 (含服务商与密钥)"),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.settings.export_include_ai = !this.settings.export_include_ai;
                                cx.notify();
                            }),
                        ),
                ),
        )
        // QR Code Container
        .child(match qr_matrix {
            Ok(ref matrix) => render_qr_grid(matrix),
            Err(ref err) => div()
                .p(px(20.0))
                .rounded(px(10.0))
                .bg(t.raised)
                .text_color(t.st)
                .text_size(px(12.0))
                .child(SharedString::from(format!("生成失败: {err}")))
                .into_any_element(),
        })
        .child(
            div()
                .text_size(px(11.0))
                .text_color(t.tx2)
                .child("提示：二维码包含敏感连接密钥，请仅在可信私密环境下展示与扫描。"),
        );

    let footer: Vec<AnyElement> = vec![
        button(
            &t,
            "复制配置代码 (JSON)",
            ButtonKind::Secondary,
            None,
            cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_json.clone()));
                this.ext.toasts.push(
                    crate::ui::ext::ToastKind::Success,
                    "配置 JSON 已复制到剪贴板，可在移动端直接粘贴导入".to_string(),
                );
                cx.notify();
            }),
        )
        .into_any_element(),
        button(
            &t,
            "完成",
            ButtonKind::Primary,
            None,
            cx.listener(|this, _, _, cx| {
                this.settings.export_qr_open = false;
                cx.notify();
            }),
        )
        .into_any_element(),
    ];

    modal(
        &t,
        "跨端配置同步 (导出给手机端)",
        body,
        footer,
        480.0,
        cx.listener(|this, _, _, cx| {
            this.settings.export_qr_open = false;
            cx.notify();
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_qr_matrix_success() {
        let text = "{\"jiyue\":\"config_export\",\"version\":1}";
        let res = generate_qr_matrix(text);
        assert!(res.is_ok());
        let matrix = res.unwrap();
        assert!(!matrix.is_empty());
        assert_eq!(matrix.len(), matrix[0].len());
    }

    #[test]
    fn test_collect_export_config_structure() {
        let export = collect_export_config(None, true, true);
        assert_eq!(export.jiyue, "config_export");
        assert_eq!(export.version, 1);
        assert!(export.timestamp > 0);
    }
}
