use super::super::*;
use gpui_webview::WebView;

/// Blocks scripts and form submission; the page is displayed, not run.
const PREVIEW_CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src * data: blob:; style-src * 'unsafe-inline'; font-src * data:; media-src *\">";

/// Renders a server's HTML response in a native webview.
pub(in crate::ui) struct HtmlResponsePreview {
    html: String,
    webview: Option<Entity<WebView>>,
    loaded_html: Option<String>,
}

impl HtmlResponsePreview {
    pub(in crate::ui) fn new() -> Self {
        Self {
            html: String::new(),
            webview: None,
            loaded_html: None,
        }
    }

    pub(in crate::ui) fn set_html(&mut self, html: &str, cx: &mut Context<Self>) {
        if self.html != html {
            self.html = html.to_string();
            cx.notify();
        }
    }

    /// Drops the native view so it stops covering the window.
    pub(in crate::ui) fn release(&mut self, cx: &mut Context<Self>) {
        if self.webview.take().is_some() {
            self.loaded_html = None;
            cx.notify();
        }
    }

    fn sanitized_document(html: &str) -> String {
        format!("{PREVIEW_CSP}{html}")
    }

    fn build_webview(
        document: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<WebView>> {
        let builder = wry::WebViewBuilder::new()
            .with_html(document)
            .with_navigation_handler(|url| url.starts_with("about:") || url.starts_with("data:"))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny);
        match WebView::build(builder, window, cx) {
            Ok(webview) => Some(cx.new(|_| webview)),
            Err(error) => {
                log::warn!("Failed to create HTML response webview: {error}");
                None
            }
        }
    }
}

impl Render for HtmlResponsePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.webview.is_none() {
            let document = Self::sanitized_document(&self.html);
            self.webview = Self::build_webview(document, window, cx);
            self.loaded_html = self.webview.as_ref().map(|_| self.html.clone());
        } else if self.loaded_html.as_deref() != Some(self.html.as_str()) {
            if let Some(webview) = &self.webview {
                let document = Self::sanitized_document(&self.html);
                let _ = webview.read(cx).raw().load_html(&document);
            }
            self.loaded_html = Some(self.html.clone());
        }

        div()
            .size_full()
            .when_some(self.webview.clone(), |this, webview| this.child(webview))
    }
}
