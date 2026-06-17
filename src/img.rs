use crate::api;
use image::imageops::FilterType;
use image::DynamicImage;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Fetch and decode an image from a URL into a `DynamicImage`.
/// Used by the graphics-protocol preview path (kitty/iterm2/sixel).
pub fn fetch_dynamic_image(url: &str) -> Result<DynamicImage, String> {
    let bytes = api::get_bytes(url)?;
    image::load_from_memory(&bytes).map_err(|e| e.to_string())
}

/// Fetch an image from a URL and render it as colored half-block lines
/// that fit within the given width/height (in terminal cells).
/// Each cell = 1 char wide, 2 pixels tall (using ▀ with fg=top, bg=bottom).
pub fn fetch_and_render(url: &str, max_w: u16, max_h: u16) -> Result<Vec<Line<'static>>, String> {
    let bytes = api::get_bytes(url)?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    Ok(render_image_to_lines(&img, max_w, max_h))
}

/// Render an already-decoded image into colored half-block lines that fit
/// within the given width/height (in terminal cells). Used by the text
/// fallback for both still images and video frames.
pub fn render_image_to_lines(img: &DynamicImage, max_w: u16, max_h: u16) -> Vec<Line<'static>> {
    let pixel_w = max_w as u32;
    let pixel_h = (max_h as u32) * 2; // 2 pixels per cell row

    let img = img.resize(pixel_w, pixel_h, FilterType::Triangle);
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    let mut lines = Vec::new();
    let mut y = 0u32;
    while y < h {
        let mut spans = Vec::new();
        for x in 0..w {
            let top = rgba.get_pixel(x, y);
            let bot = if y + 1 < h {
                rgba.get_pixel(x, y + 1)
            } else {
                top
            };

            let fg = Color::Rgb(top[0], top[1], top[2]);
            let bg = Color::Rgb(bot[0], bot[1], bot[2]);

            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }
        lines.push(Line::from(spans));
        y += 2;
    }

    lines
}

fn preview_block() -> Block<'static> {
    Block::default()
        .title(" Preview (i to toggle) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
}

/// Draw a cached text (half-block) image preview into a ratatui frame area.
/// This is the fallback used when the terminal has no graphics protocol.
pub fn draw_preview(f: &mut ratatui::Frame, area: ratatui::layout::Rect, image_lines: &[Line<'static>]) {
    let block = preview_block();
    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible: Vec<Line> = image_lines.iter()
        .take(inner.height as usize)
        .cloned()
        .collect();
    let img_widget = Paragraph::new(visible);
    f.render_widget(img_widget, inner);
}

/// Draw a centered message inside the preview block (e.g. while a video loads).
pub fn draw_preview_message(f: &mut ratatui::Frame, area: ratatui::layout::Rect, msg: &str) {
    let block = preview_block();
    let inner = block.inner(area);
    f.render_widget(block, area);
    let p = Paragraph::new(msg)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, inner);
}

/// Draw a real image into a ratatui frame area using the terminal's graphics
/// protocol (kitty / iTerm2 / sixel) via ratatui-image.
pub fn draw_graphic_preview(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &mut ratatui_image::protocol::StatefulProtocol,
) {
    let block = preview_block();
    let inner = block.inner(area);
    f.render_widget(block, area);
    // `Scale` (unlike the default `Fit`) upscales the image to fill the pane
    // when the source is smaller than the area, keeping aspect ratio.
    let widget = ratatui_image::StatefulImage::default()
        .resize(ratatui_image::Resize::Scale(None));
    f.render_stateful_widget(widget, inner, state);
}

/// Inner drawing area of the preview pane (inside the block border). Frames are
/// pre-encoded to this size so playback is a plain blit.
pub fn preview_inner(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    preview_block().inner(area)
}

/// Draw a pre-encoded video frame. Unlike [`draw_graphic_preview`], the frame
/// is already encoded for this area, so this just blits it — no per-frame
/// resize/encode work on the UI thread. Used by immediate-mode protocols.
pub fn draw_video_frame(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    frame: &ratatui_image::protocol::Protocol,
) {
    let block = preview_block();
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(ratatui_image::Image::new(frame), inner);
}

