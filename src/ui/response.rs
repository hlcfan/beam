pub(super) mod history;
pub(super) mod html_preview;
pub(super) mod persistence;
pub(super) mod render;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResponseTab {
    Body,
    Headers,
}

pub(super) const MACOS_COMMAND_ICON_PATH: &str = "icons/command.svg";
pub(super) const NON_MACOS_COMMAND_ICON_PATH: &str = "icons/chevron-up.svg";

impl BeamView {
    /// Rebuilds the cached headers table from raw header text. The only writer of
    /// `response_headers_table`, so the cache cannot drift from the response.
    pub(in crate::ui) fn set_response_headers(&mut self, raw: &str) {
        self.response_headers_table = render::response_headers_table_markup(raw);
    }

    pub(in crate::ui) fn clear_response_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.response_status = "—".to_string();
        self.response_status_code = None;
        self.response_time = "—".to_string();
        self.response_size = "—".to_string();
        self.set_response_headers("");
        self.response_content_type = None;
        self.set_response_image(None, window, cx);
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| input.set_value(String::new(), window, cx),
        );
    }

    pub(in crate::ui) fn apply_response_snapshot(
        &mut self,
        snapshot: &StoredResponseSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content_type = snapshot.content_type.clone();
        let formatted_body =
            self.response_body_for_display(&snapshot.body, content_type.as_deref());
        let language = response_body_editor_language(content_type.as_deref());
        self.response_status = snapshot.status.clone();
        self.response_status_code = snapshot.status_code;
        self.response_time = snapshot.time.clone();
        self.response_size = snapshot.size.clone();
        self.set_response_headers(&snapshot.headers_raw);
        self.set_response_image(snapshot.image.clone(), window, cx);
        self.response_content_type = content_type;
        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| {
                input.set_highlighter(language, cx);
                input.set_value(formatted_body.clone(), window, cx);
            },
        );
        self.response_body_language = language;
    }

    pub(in crate::ui) fn sync_response_pane_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.shell.workspace_tree.selected_request_id() else {
            self.response_history_entries.clear();
            self.selected_response_history_index = None;
            self.clear_response_pane(window, cx);
            self.script_result = None;
            return;
        };

        self.response_history_entries =
            load_response_history_entries(&self.current_workspace_paths, request_id);
        self.selected_response_history_index =
            (!self.response_history_entries.is_empty()).then_some(0);
        if let Some(snapshot) = self.response_history_entries.first().map(|entry| {
            load_response_snapshot_for_history_entry(&self.current_workspace_paths, entry)
        }) {
            self.apply_response_snapshot(&snapshot, window, cx);
            self.restore_selected_request_response_scroll_offset(window, cx);
        } else {
            self.clear_response_pane(window, cx);
        }
        self.script_result = load_script_result(&self.current_workspace_paths, request_id);

        let (status, status_code, time, size) = response_summary_for_selected_request(
            Some(request_id),
            &self.request_execution_states,
            &self.response_status,
            self.response_status_code,
            &self.response_time,
            &self.response_size,
        );
        self.response_status = status;
        self.response_status_code = status_code;
        self.response_time = time;
        self.response_size = size;
    }

    pub(in crate::ui) fn status_code_in_color(status: Option<u16>, cx: &App) -> Hsla {
        match status {
            Some(200..=299) => cx.theme().success,
            Some(300..=399) => cx.theme().warning,
            Some(400..=599) => cx.theme().danger,
            Some(100..=199) => cx.theme().info,
            _ => cx.theme().muted_foreground,
        }
    }

    pub(in crate::ui) fn response_body_for_display(
        &self,
        body: &str,
        content_type: Option<&str>,
    ) -> String {
        if !self.shell.theme.auto_format_response {
            return body.to_string();
        }
        format_body_text(body, BodyFormatHint::FromContentType(content_type))
            .unwrap_or_else(|_| body.to_string())
    }

    pub(in crate::ui) fn format_response_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.response_body_editor.read(cx).value().to_string();
        if current_text.trim().is_empty() {
            return;
        }

        let formatted = format_body_text(
            &current_text,
            BodyFormatHint::FromContentType(self.response_content_type.as_deref()),
        )
        .unwrap_or_else(|_| current_text.clone());

        if formatted == current_text {
            return;
        }

        self.update_response_body_editor_with_scroll_persistence_suppressed(
            window,
            cx,
            |input, window, cx| Self::replace_editor_text(input, formatted, window, cx),
        );
        cx.notify();
    }

    /// Swaps the displayed image and frees the outgoing one. gpui keeps decoded
    /// frames in its asset cache and sprite atlas until told to drop them, so a
    /// large GIF would otherwise stay resident after the user moves on.
    pub(in crate::ui) fn set_response_image(
        &mut self,
        image: Option<Arc<gpui::Image>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = std::mem::replace(&mut self.response_image, image);
        let Some(previous) = previous else {
            return;
        };
        if self
            .response_image
            .as_ref()
            .is_some_and(|current| current.id == previous.id)
        {
            return;
        }
        if let Some(render_image) = previous.clone().get_render_image(window, cx) {
            let _ = window.drop_image(render_image);
        }
        previous.remove_asset(cx);
    }
}

const MAX_IMAGE_PREVIEW_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_PREVIEW_SIDE: u32 = 8192;
const MAX_IMAGE_PREVIEW_PIXELS: u64 = 64_000_000;
/// gpui decodes every GIF frame to RGBA, so the budget is width x height x frames.
const MAX_GIF_PREVIEW_TOTAL_PIXELS: u64 = 128_000_000;

pub(super) enum ImagePreview {
    /// Not an image type the pane previews; show the body as text.
    NotImage,
    /// Safe to decode and display.
    Ready,
    /// An image type, but not previewable; carries the text to show instead.
    Unavailable(String),
}

/// Decides whether a response body is an image the pane can safely preview.
/// Reads only the image header, so oversized images are never decoded.
pub(super) fn classify_image_preview(content_type: Option<&str>, bytes: &[u8]) -> ImagePreview {
    let Some(format) = response_image_format(content_type) else {
        return ImagePreview::NotImage;
    };
    let size = format_bytes(bytes.len());
    if bytes.len() > MAX_IMAGE_PREVIEW_BYTES {
        return ImagePreview::Unavailable(format!("[Image too large to preview: {size}]"));
    }
    match image_dimensions(format, bytes) {
        Some((width, height))
            if width > MAX_IMAGE_PREVIEW_SIDE
                || height > MAX_IMAGE_PREVIEW_SIDE
                || u64::from(width) * u64::from(height) > MAX_IMAGE_PREVIEW_PIXELS =>
        {
            ImagePreview::Unavailable(format!(
                "[Image dimensions too large to preview: {width}x{height} px, {size}]"
            ))
        }
        Some((width, height)) => match gif_frame_budget_exceeded(
            format,
            bytes,
            width,
            height,
            MAX_GIF_PREVIEW_TOTAL_PIXELS,
        ) {
            Some(frames) => ImagePreview::Unavailable(format!(
                "[Animated GIF too large to preview: {width}x{height} px, {frames}+ frames, {size}]"
            )),
            None => ImagePreview::Ready,
        },
        None => ImagePreview::Unavailable(format!("[Image could not be read: {size}]")),
    }
}

/// For GIFs, returns the frame count at which the decoded size passes the
/// budget. Walks frame headers only, so pixel data is never decompressed.
pub(super) fn gif_frame_budget_exceeded(
    format: gpui::ImageFormat,
    bytes: &[u8],
    width: u32,
    height: u32,
    budget_pixels: u64,
) -> Option<u64> {
    if format != gpui::ImageFormat::Gif {
        return None;
    }
    let frame_pixels = (u64::from(width) * u64::from(height)).max(1);
    let max_frames = budget_pixels / frame_pixels;
    let mut decoder = gif::DecodeOptions::new()
        .read_info(std::io::Cursor::new(bytes))
        .ok()?;
    let mut frames = 0u64;
    while let Ok(Some(_)) = decoder.next_frame_info() {
        frames += 1;
        if frames > max_frames {
            return Some(frames);
        }
    }
    None
}

fn image_dimensions(format: gpui::ImageFormat, bytes: &[u8]) -> Option<(u32, u32)> {
    let format = match format {
        gpui::ImageFormat::Png => image::ImageFormat::Png,
        gpui::ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        gpui::ImageFormat::Gif => image::ImageFormat::Gif,
        gpui::ImageFormat::Webp => image::ImageFormat::WebP,
        gpui::ImageFormat::Bmp => image::ImageFormat::Bmp,
        gpui::ImageFormat::Tiff => image::ImageFormat::Tiff,
        _ => return None,
    };
    image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .ok()
}

/// Maps a response content type to an image format GPUI can decode.
/// SVG stays in the text view because it is XML.
pub(super) fn response_image_format(content_type: Option<&str>) -> Option<gpui::ImageFormat> {
    let media_type = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    match media_type.as_str() {
        "image/png" => Some(gpui::ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(gpui::ImageFormat::Jpeg),
        "image/gif" => Some(gpui::ImageFormat::Gif),
        "image/webp" => Some(gpui::ImageFormat::Webp),
        "image/bmp" => Some(gpui::ImageFormat::Bmp),
        "image/tiff" => Some(gpui::ImageFormat::Tiff),
        _ => None,
    }
}

pub(super) fn response_image_from_bytes(
    content_type: Option<&str>,
    bytes: Vec<u8>,
) -> Option<Arc<gpui::Image>> {
    let format = response_image_format(content_type)?;
    if bytes.is_empty() {
        return None;
    }
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

/// Text shown in the body editor in place of image bytes.
pub(super) fn image_placeholder_body(len: usize) -> String {
    format!("[Image, {}]", format_bytes(len))
}

/// Body shown for an image type that cannot be previewed. Text served under an
/// image content type (an error page, for example) stays readable; binary data
/// falls back to the explanatory note.
pub(super) fn unavailable_image_body(bytes: &[u8], note: String) -> String {
    std::str::from_utf8(bytes).map_or(note, str::to_owned)
}
