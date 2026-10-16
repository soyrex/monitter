//! Publish only resolved presentation tokens over the private owner IPC.
use crate::{Desktop, design::Palette};
use gpui_kit::{Context, component::ActiveTheme as _};
use monitter_lib::report_styles::ReportTheme;

#[derive(Default)]
pub(super) struct Publisher {
    latest: Option<ReportTheme>,
    pending: Option<ReportTheme>,
    backend_id: Option<String>,
    busy: bool,
    unsupported_backend: Option<String>,
    error: Option<String>,
}

impl Publisher {
    fn unsupported(&self, backend_id: &str) -> bool {
        self.unsupported_backend.as_deref() == Some(backend_id)
    }

    fn finish_publication(
        &mut self,
        backend_id: &str,
        result: Result<(), String>,
        preferences_error: &mut Option<String>,
    ) {
        // Older resident servers do not implement this optional method. Cache
        // that capability per server instance, never a transport/validation error.
        let unavailable = matches!(
            result.as_ref().err().map(String::as_str),
            Some("Unknown private native method.")
                | Some(
                    "Unknown private native method. Server shutdown is available only in the menu-bar server."
                )
        );
        if unavailable {
            self.unsupported_backend = Some(backend_id.to_owned());
        }
        match result {
            Ok(()) => {
                if self.unsupported(backend_id) {
                    self.unsupported_backend = None;
                }
            }
            Err(error) if !unavailable => {
                let message = format!("Report theme could not be shared with agents: {error}");
                preferences_error.get_or_insert_with(|| message.clone());
                self.error = Some(message);
                return;
            }
            Err(_) => {}
        }
        if *preferences_error == self.error {
            *preferences_error = None;
        }
        self.error = None;
    }
}

fn theme_tokens(
    palette: Palette,
    font_family: String,
    mono_font_family: String,
    font_size: f32,
) -> ReportTheme {
    ReportTheme {
        dark: palette.dark,
        accent: palette.accent,
        paper: palette.paper,
        sidebar: palette.sidebar,
        panel: palette.panel(),
        line: palette.line,
        soft: palette.soft,
        code: palette.code,
        ink: palette.ink,
        muted: palette.muted,
        font_family,
        mono_font_family,
        font_size,
    }
}

impl Desktop {
    pub(super) fn publish_report_theme(&mut self, palette: Palette, cx: &mut Context<Self>) {
        let Some(service) = self.service.as_ref() else {
            return;
        };
        let backend_id = service.server_instance_id().to_owned();
        if self.report_theme_publisher.unsupported(&backend_id) {
            return;
        }
        let tokens = theme_tokens(
            palette,
            cx.theme().font_family.to_string(),
            cx.theme().mono_font_family.to_string(),
            f32::from(cx.theme().font_size),
        );
        let publisher = &mut self.report_theme_publisher;
        if publisher.backend_id.as_ref() == Some(&backend_id)
            && publisher.latest.as_ref() == Some(&tokens)
        {
            return;
        }
        publisher.backend_id = Some(backend_id);
        publisher.latest = Some(tokens.clone());
        // A single writer drains only the latest theme, so an older slow IPC
        // response cannot overwrite a newer accent or appearance change.
        publisher.pending = Some(tokens);
        self.drain_report_theme(cx);
    }

    fn drain_report_theme(&mut self, cx: &mut Context<Self>) {
        if self.report_theme_publisher.busy {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        let backend_id = service.server_instance_id().to_owned();
        if self.report_theme_publisher.unsupported(&backend_id) {
            self.report_theme_publisher.pending = None;
            return;
        }
        let Some(tokens) = self.report_theme_publisher.pending.take() else {
            return;
        };
        self.report_theme_publisher.busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { service.publish_report_theme(&tokens) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.report_theme_publisher.busy = false;
                view.report_theme_publisher.finish_publication(
                    &backend_id,
                    result,
                    &mut view.preferences_error,
                );
                view.drain_report_theme(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_method_is_cached_only_for_the_unsupported_server() {
        let mut publisher = Publisher::default();
        let mut error = None;
        publisher.finish_publication("old", Err("Unknown private native method. Server shutdown is available only in the menu-bar server.".into()), &mut error);
        assert!(error.is_none());
        assert!(publisher.unsupported("old"));
        assert!(!publisher.unsupported("replacement"));
        publisher.finish_publication("replacement", Ok(()), &mut error);
        assert!(error.is_none());
    }

    #[test]
    fn genuine_errors_remain_visible_and_unrelated_errors_are_preserved() {
        let mut publisher = Publisher::default();
        let mut error = None;
        publisher.finish_publication("server", Err("Connection closed".into()), &mut error);
        assert!(error.as_deref().unwrap().contains("Connection closed"));
        assert!(!publisher.unsupported("server"));
        publisher.finish_publication(
            "server",
            Err("Unknown private native method.".into()),
            &mut error,
        );
        assert!(error.is_none());
        error = Some("Layout save failed".into());
        publisher.finish_publication(
            "server",
            Err("Unknown private native method.".into()),
            &mut error,
        );
        assert_eq!(error.as_deref(), Some("Layout save failed"));
    }

    #[test]
    fn report_styles_use_the_exact_resolved_desktop_palette() {
        let palette = Palette {
            dark: true,
            accent: 0x123456,
            paper: 0x111111,
            sidebar: 0x222222,
            panel: 0x333333,
            card: 0x333333,
            ink: 0xffffff,
            muted: 0xaaaaaa,
            line: 0x444444,
            soft: 0x555555,
            code: 0x666666,
        };
        let tokens = theme_tokens(palette, "IBM Plex Sans".into(), "IBM Plex Mono".into(), 18.);
        assert_eq!(tokens.accent, palette.accent);
        assert_eq!(tokens.line, palette.line);
        assert_eq!(tokens.panel, palette.panel());
        assert_eq!(tokens.font_size, 18.);
        assert_eq!(tokens.font_family, "IBM Plex Sans");
    }
}
