//! Dialogs of the AI features: consent before the first use, the rewrite instruction, the summary
//! confirmation, and the confirmation before a provider is removed.

use gpui::{AnyElement, App, Context, MouseDownEvent, Window, div, prelude::*, px};

use super::{Modal, effective_config, host_of, notes, selection, settings};
use crate::RustdianApp;
use crate::ui::widgets::{ButtonKind, button, modal};

/// A close handler for a dialog (fresh copy for each use).
fn closer(
    cx: &mut Context<RustdianApp>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(|this, _: &MouseDownEvent, _, cx| {
        this.ext.ai.modal = Modal::None;
        this.ext.ai.target = None;
        cx.notify();
    })
}

fn paragraph(text: &str) -> gpui::Div {
    div().pb(px(8.0)).child(text.to_string())
}

/// Renders the open dialog. Called only while `modal` is not `Modal::None`.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    match &app.ext.ai.modal {
        Modal::None => div().into_any_element(),
        Modal::Consent => {
            // The provider that would be used once AI is on (the master switch is still off here).
            let cfg = effective_config(&app.ext.ai);
            let who = match cfg
                .active_chat
                .as_deref()
                .and_then(|id| cfg.providers.iter().find(|p| p.id == id))
            {
                Some(p) => format!("{}（{}）", p.name, host_of(&p.base_url)),
                None => "尚未选择服务商".to_string(),
            };
            let body = div()
                .flex()
                .flex_col()
                .text_color(t.tx2)
                .child(paragraph(&format!(
                    "开启后，AI 功能会把你主动选择的内容发送给你配置的服务商：{who}。"
                )))
                .child(paragraph("· 提问：发送前会显示预览，你确认后才发送，包括所选笔记和选中文字。"))
                .child(paragraph("· 选区操作与总结：你选中的文字，或整篇笔记。"))
                .child(paragraph(
                    "· 向量索引：未排除的笔记会被切块后发送给嵌入服务（需要选择嵌入服务）。",
                ))
                .child(paragraph(
                    "排除的文件夹和标签下的笔记永远不会发送。API 密钥以明文保存在 credentials.toml（用户目录）中，不写入笔记库。",
                ));
            let footer = vec![
                button(&t, "取消", ButtonKind::Secondary, None, closer(cx)).into_any_element(),
                button(
                    &t,
                    "我已了解，开启",
                    ButtonKind::Primary,
                    None,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        settings::confirm_enable(this, cx);
                    }),
                )
                .into_any_element(),
            ];
            modal(&t, "开启 AI 功能", body, footer, 480.0, closer(cx))
        }
        Modal::Rewrite => {
            let body = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(div().text_color(t.tx2).child(
                    "输入改写要求。选中的文字会发送给 AI，结果以对比形式显示，接受后才会替换。",
                ))
                .child(
                    div()
                        .w(px(436.0))
                        .h(px(32.0))
                        .children(app.ext.ai.inputs.rewrite.clone()),
                );
            let footer = vec![
                button(&t, "取消", ButtonKind::Secondary, None, closer(cx)).into_any_element(),
                button(
                    &t,
                    "改写",
                    ButtonKind::Primary,
                    Some("pencil"),
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        selection::confirm_rewrite_from_modal(this, cx);
                    }),
                )
                .into_any_element(),
            ];
            modal(&t, "改写选区", body, footer, 480.0, closer(cx))
        }
        Modal::Summary => {
            let (provider, target_len) = {
                let cfg = effective_config(&app.ext.ai);
                let provider = cfg
                    .chat_provider()
                    .map(|p| format!("{}（{}）", p.name, host_of(&p.base_url)))
                    .unwrap_or_else(|| "未配置服务商".into());
                let len = app
                    .ext
                    .ai
                    .target
                    .as_ref()
                    .map(|t| t.text.chars().count())
                    .unwrap_or(0);
                (provider, len)
            };
            let body = div()
                .flex()
                .flex_col()
                .text_color(t.tx2)
                .child(paragraph(&format!(
                    "将把整篇笔记（约 {target_len} 字）发送给 {provider} 生成摘要。"
                )))
                .child(paragraph(
                    "摘要会显示在右侧助手中，你可以再选择插入到笔记顶部。",
                ));
            let footer = vec![
                button(&t, "取消", ButtonKind::Secondary, None, closer(cx)).into_any_element(),
                button(
                    &t,
                    "发送并总结",
                    ButtonKind::Primary,
                    Some("file-text"),
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        notes::confirm_summary(this, cx);
                    }),
                )
                .into_any_element(),
            ];
            modal(&t, "总结当前笔记", body, footer, 460.0, closer(cx))
        }
        Modal::DeleteProvider(id) => {
            let name = app
                .ext
                .ai
                .config
                .providers
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let body = div()
                .flex()
                .flex_col()
                .text_color(t.tx2)
                .child(paragraph(&format!("确定删除服务「{name}」吗？")))
                .child(paragraph(
                    "它保存的 API 密钥也会从 credentials.toml 中删除。",
                ));
            let id = id.clone();
            let footer = vec![
                button(&t, "取消", ButtonKind::Secondary, None, closer(cx)).into_any_element(),
                button(
                    &t,
                    "删除",
                    ButtonKind::Danger,
                    None,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        settings::delete_provider(this, &id, cx);
                    }),
                )
                .into_any_element(),
            ];
            modal(&t, "删除服务", body, footer, 420.0, closer(cx))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dialog_is_none() {
        assert_eq!(Modal::default(), Modal::None);
    }
}
