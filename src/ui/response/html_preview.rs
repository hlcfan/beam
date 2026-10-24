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

    /// Inserts the CSP inside `<head>` (else after `<html>` or the doctype) so the
    /// doctype stays first and the document keeps standards mode.
    fn sanitized_document(html: &str) -> String {
        let insert_at = Self::open_tag_end(html, "<head")
            .or_else(|| Self::open_tag_end(html, "<html"))
            .or_else(|| Self::open_tag_end(html, "<!doctype"))
            .unwrap_or(0);
        let (before, after) = html.split_at(insert_at);
        format!("{before}{PREVIEW_CSP}{after}")
    }

    /// Byte offset just past the `>` of the first tag starting with `prefix`.
    fn open_tag_end(html: &str, prefix: &str) -> Option<usize> {
        let lowered = html.to_ascii_lowercase();
        let start = lowered.find(prefix)?;
        let after_prefix = start + prefix.len();
        // Reject longer names such as `<header` for `<head`.
        if lowered[after_prefix..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            return None;
        }
        lowered[after_prefix..]
            .find('>')
            .map(|end| after_prefix + end + 1)
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

#[cfg(test)]
mod tests {
    use super::{HtmlResponsePreview, PREVIEW_CSP};

    #[test]
    fn csp_goes_inside_head_after_doctype() {
        let out = HtmlResponsePreview::sanitized_document(
            "<!DOCTYPE html><html lang=\"en\"><HEAD><title>x</title></HEAD><body></body></html>",
        );
        assert_eq!(
            out,
            format!(
                "<!DOCTYPE html><html lang=\"en\"><HEAD>{PREVIEW_CSP}<title>x</title></HEAD><body></body></html>"
            )
        );
    }

    #[test]
    fn csp_falls_back_to_html_then_doctype_then_start() {
        let no_head = HtmlResponsePreview::sanitized_document("<html><body>x</body></html>");
        assert!(no_head.starts_with(&format!("<html>{PREVIEW_CSP}")));
        let doctype_only = HtmlResponsePreview::sanitized_document("<!doctype html><p>x</p>");
        assert!(doctype_only.starts_with(&format!("<!doctype html>{PREVIEW_CSP}")));
        let fragment = HtmlResponsePreview::sanitized_document("<p>x</p>");
        assert!(fragment.starts_with(PREVIEW_CSP));
        let header_tag = HtmlResponsePreview::sanitized_document("<header>x</header>");
        assert!(header_tag.starts_with(PREVIEW_CSP));
    }
}
