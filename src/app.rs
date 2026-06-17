use crate::api::{self, Post};
use crate::config::Config;
use crate::img;
use crate::video;
use image::DynamicImage;
use ratatui::prelude::*;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::{Protocol, StatefulProtocol};
use ratatui_image::Resize;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// A loaded preview for a post. Graphics-capable terminals get a real image
/// rendered via a terminal graphics protocol; everything else falls back to
/// the colored half-block text rendering. Videos/animations carry a sequence
/// of frames plus the currently displayed index.
pub enum Preview {
    Text(Vec<Line<'static>>),
    Graphic(Box<StatefulProtocol>),
    /// Video frames pre-encoded to the preview pane. Drawing is a plain blit.
    /// Kitty frames carry unique random image IDs so the terminal caches them
    /// after the first transmission and reuses them on subsequent loop cycles.
    Animation { frames: Vec<Protocol>, idx: usize },
    TextAnimation { frames: Vec<Vec<Line<'static>>>, idx: usize },
}

impl Preview {
    /// True if this preview has more than one frame (i.e. actually animates).
    pub fn is_animated(&self) -> bool {
        match self {
            Preview::Animation { frames, .. } => frames.len() > 1,
            Preview::TextAnimation { frames, .. } => frames.len() > 1,
            _ => false,
        }
    }

    /// True when a multi-frame preview has shown its last frame and should stop.
    pub fn is_ended(&self) -> bool {
        match self {
            Preview::Animation { frames, idx } if frames.len() > 1 => *idx >= frames.len() - 1,
            Preview::TextAnimation { frames, idx } if frames.len() > 1 => *idx >= frames.len() - 1,
            _ => false,
        }
    }

    /// Advance to the next frame, stopping at the last one. No-op for still
    /// previews. This is the *only* thing that moves video playback forward;
    /// it's paced by [`App::tick`] and never consults the audio.
    pub fn advance(&mut self) {
        match self {
            Preview::Animation { frames, idx } if frames.len() > 1 => {
                *idx = (*idx + 1).min(frames.len() - 1);
            }
            Preview::TextAnimation { frames, idx } if frames.len() > 1 => {
                *idx = (*idx + 1).min(frames.len() - 1);
            }
            _ => {}
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum Screen {
    Search,
    Results,
    Detail,
    Help,
    Settings,
}

#[derive(Clone, PartialEq)]
pub enum InputTarget {
    Tags,
    Sort,
    Rating,
}

pub struct App {
    pub screen: Screen,
    pub prev_screen: Screen,
    pub should_quit: bool,

    // search
    pub tag_input: String,
    pub input_target: InputTarget,
    pub sort_options: Vec<&'static str>,
    pub sort_idx: usize,
    pub rating_options: Vec<&'static str>,
    pub rating_idx: usize,

    // results
    pub posts: Vec<Post>,
    pub selected: usize,
    pub page: u32,
    pub detail_scroll: u16,

    // image preview
    pub show_image: bool,
    pub image_cache: Option<(u64, Preview)>, // (post_id, rendered preview)
    /// Some(picker) if the terminal supports a graphics protocol; None means
    /// we fall back to the text (half-block) preview.
    pub picker: Option<Picker>,
    /// In-flight background video decode: (post_id, channel of decoded data).
    pub video_loader: Option<(u64, Receiver<Result<video::VideoData, String>>)>,
    /// In-flight background frame encode: (post_id, channel of encoded frames).
    /// Frames are resized/encoded off the UI thread so playback never stalls
    /// doing that work mid-loop.
    pub encode_loader: Option<(u64, Receiver<Vec<Protocol>>)>,
    /// Inner area of the preview pane from the last draw; frames are encoded to
    /// fit this so the blit matches the on-screen rect.
    pub preview_area: Option<Rect>,
    /// Audio waiting for its frames to finish encoding, so picture and sound
    /// start together.
    pending_audio: Option<Vec<u8>>,

    // audio playback for video previews
    /// Kept alive for the lifetime of the app; dropping it silences output.
    /// `None` if no audio device is available.
    audio_stream: Option<(OutputStream, OutputStreamHandle)>,
    /// Current looping audio track for the active video preview, if any. Runs
    /// on rodio's own playback thread, completely independent of video frames.
    audio_sink: Option<Sink>,
    /// When the on-screen frame was last advanced, used purely to pace the
    /// video at the configured fps. This is the video clock and nothing else.
    last_tick: Option<Instant>,

    // settings
    pub config: Config,
    pub fps_input: u32, // working value while on the Settings screen

    // status
    pub status_msg: String,
    pub loading: bool,
}

impl App {
    pub fn new(picker: Option<Picker>, config: Config) -> Self {
        let fps_input = config.fps;
        // Best-effort audio: if no output device is available, previews just
        // play silently.
        let audio_stream = OutputStream::try_default().ok();
        Self {
            screen: Screen::Search,
            prev_screen: Screen::Search,
            should_quit: false,
            tag_input: String::new(),
            input_target: InputTarget::Tags,
            sort_options: vec!["default", "score", "favcount", "new", "old"],
            sort_idx: 0,
            rating_options: vec!["all", "s", "q", "e"],
            rating_idx: 0,
            posts: Vec::new(),
            selected: 0,
            page: 1,
            detail_scroll: 0,
            show_image: false,
            image_cache: None,
            picker,
            video_loader: None,
            encode_loader: None,
            preview_area: None,
            pending_audio: None,
            audio_stream,
            audio_sink: None,
            last_tick: None,
            config,
            fps_input,
            status_msg: String::new(),
            loading: false,
        }
    }

    pub fn current_sort(&self) -> &str {
        self.sort_options[self.sort_idx]
    }

    pub fn current_rating(&self) -> &str {
        self.rating_options[self.rating_idx]
    }

    pub fn cycle_sort(&mut self) {
        self.sort_idx = (self.sort_idx + 1) % self.sort_options.len();
    }

    pub fn cycle_rating(&mut self) {
        self.rating_idx = (self.rating_idx + 1) % self.rating_options.len();
    }

    pub fn search(&mut self) {
        self.loading = true;
        self.status_msg = "Searching...".to_string();
        match api::search_posts(&self.tag_input, self.page, self.current_sort(), self.current_rating()) {
            Ok(posts) => {
                let count = posts.len();
                self.posts = posts;
                self.selected = 0;
                self.detail_scroll = 0;
                self.status_msg = format!("{count} results (page {})", self.page);
                if count > 0 {
                    self.screen = Screen::Results;
                } else {
                    self.status_msg = "No results found".to_string();
                }
            }
            Err(e) => {
                self.status_msg = format!("Error: {e}");
            }
        }
        self.loading = false;
    }

    pub fn next_post(&mut self) {
        if !self.posts.is_empty() && self.selected < self.posts.len() - 1 {
            self.selected += 1;
        }
    }

    pub fn prev_post(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn next_page(&mut self) {
        self.page += 1;
        self.search();
    }

    pub fn prev_page(&mut self) {
        if self.page > 1 {
            self.page -= 1;
            self.search();
        }
    }

    pub fn current_post(&self) -> Option<&Post> {
        self.posts.get(self.selected)
    }

    pub fn open_in_browser(&mut self) {
        if let Some(post) = self.current_post() {
            let url = format!("https://e621.net/posts/{}", post.id);
            if let Err(e) = open::that(&url) {
                self.status_msg = format!("Failed to open browser: {e}");
            } else {
                self.status_msg = format!("Opened post #{}", post.id);
            }
        }
    }

    pub fn toggle_image(&mut self) {
        self.show_image = !self.show_image;
        if self.show_image {
            self.load_image_for_current();
        } else {
            self.stop_audio();
        }
    }

    pub fn load_image_for_current(&mut self) {
        // Gather what we need from the post, then drop the immutable borrow so
        // we can mutate self below.
        let Some(post) = self.current_post() else { return };
        let post_id = post.id;
        // Already cached or loading for this post? Nothing to do.
        if matches!(&self.image_cache, Some((id, _)) if *id == post_id)
            || matches!(&self.video_loader, Some((id, _)) if *id == post_id)
        {
            return;
        }
        let is_video = post.is_video();
        let video_url = post.file.url.clone();
        let video_ext = post.file.ext.clone().unwrap_or_default();
        let still_url = post.still_url();

        // Switching to a different post: silence any audio from the old one
        // and drop any in-flight encode so it can't install over the new post.
        self.stop_audio();
        self.encode_loader = None;
        self.pending_audio = None;

        if is_video {
            match video_url {
                Some(url) => self.start_video_load(post_id, url, video_ext),
                None => self.status_msg = "No video URL available".to_string(),
            }
            return;
        }

        let Some(url) = still_url else {
            self.status_msg = "No preview URL available".to_string();
            return;
        };
        self.status_msg = "Loading preview...".to_string();

        if let Some(picker) = self.picker.clone() {
            // Graphics-capable terminal: download, decode, and hand off to the
            // graphics protocol. Resizing/encoding happens lazily at draw time.
            match img::fetch_dynamic_image(&url) {
                Ok(dyn_img) => {
                    let proto = picker.new_resize_protocol(dyn_img);
                    self.image_cache = Some((post_id, Preview::Graphic(Box::new(proto))));
                    self.status_msg = format!("Preview loaded for #{post_id}");
                }
                Err(e) => {
                    self.status_msg = format!("Preview failed: {e}");
                }
            }
        } else {
            // Fallback: colored half-block text rendering.
            match img::fetch_and_render(&url, 80, 30) {
                Ok(lines) => {
                    self.image_cache = Some((post_id, Preview::Text(lines)));
                    self.status_msg = format!("Preview loaded for #{post_id}");
                }
                Err(e) => {
                    self.status_msg = format!("Preview failed: {e}");
                }
            }
        }
    }

    /// Kick off decoding a video on a background thread so the UI stays
    /// responsive. Frames are delivered via a channel and picked up by
    /// [`App::poll_video`].
    fn start_video_load(&mut self, post_id: u64, url: String, ext: String) {
        let fps = self.config.fps;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(video::fetch_video_frames(&url, &ext, post_id, fps));
        });
        self.video_loader = Some((post_id, rx));
        self.status_msg = "Loading video…".to_string();
    }

    /// Check for a finished background video decode and install it as the
    /// current preview. Called once per event-loop tick.
    pub fn poll_video(&mut self) {
        let Some((id, rx)) = self.video_loader.as_ref() else { return };
        let id = *id;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.video_loader = None;
                return;
            }
        };
        self.video_loader = None;

        match result {
            Ok(data) if !data.frames.is_empty() => {
                let n = data.frames.len();
                let has_audio = data.audio.is_some();
                let sound = if has_audio { ", sound" } else { "" };

                if let Some(picker) = self.picker.clone() {
                    // Resize/encode every frame to the preview-pane size on a
                    // background thread so playback never stalls doing it.
                    // Audio waits for this to finish (see poll_encode) so
                    // picture and sound start together.
                    self.pending_audio = data.audio;
                    let area = self.preview_area.unwrap_or(Rect::new(0, 0, 60, 30));
                    let frames = data.frames;
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(encode_frames(&picker, frames, area));
                    });
                    self.encode_loader = Some((id, rx));
                    self.status_msg = format!("Encoding {n} frames…{sound}");
                } else {
                    // Half-block fallback: render frames to colored text lines.
                    let lines: Vec<Vec<Line<'static>>> = data
                        .frames
                        .iter()
                        .map(|f| img::render_image_to_lines(f, 80, 30))
                        .collect();
                    self.image_cache =
                        Some((id, Preview::TextAnimation { frames: lines, idx: 0 }));
                    self.last_tick = None;
                    if let Some(bytes) = data.audio {
                        self.start_audio(bytes);
                    }
                    self.status_msg =
                        format!("Video ready ({n} frames @ {}fps{sound})", self.config.fps);
                }
            }
            Ok(_) => self.status_msg = "No video frames decoded".to_string(),
            Err(e) => self.status_msg = format!("Video failed: {e}"),
        }
    }

    /// Install pre-encoded frames once the background encode finishes. Called
    /// once per event-loop tick.
    pub fn poll_encode(&mut self) {
        let Some((id, rx)) = self.encode_loader.as_ref() else { return };
        let id = *id;
        let encoded = match rx.try_recv() {
            Ok(f) => f,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.encode_loader = None;
                return;
            }
        };
        self.encode_loader = None;
        let frames = encoded;
        let n = frames.len();
        if n == 0 {
            self.pending_audio = None;
            self.status_msg = "Frame encoding failed".to_string();
            return;
        }
        self.image_cache = Some((id, Preview::Animation { frames, idx: 0 }));
        // Picture is ready: fire up the audio thread now so sound and video
        // start at the same moment, then each runs on its own clock.
        self.last_tick = None;
        if let Some(bytes) = self.pending_audio.take() {
            self.start_audio(bytes);
        }
        self.status_msg = format!("Video ready ({n} frames @ {}fps)", self.config.fps);
    }

    /// Begin looping the given WAV audio for the active video preview on
    /// rodio's own playback thread. Plays immediately and independently of the
    /// video frames; [`App::sync_audio`] only pauses it when the preview leaves
    /// the screen. No-op if no audio device was available.
    fn start_audio(&mut self, wav: Vec<u8>) {
        self.stop_audio();
        let Some((_, handle)) = self.audio_stream.as_ref() else { return };
        let Ok(sink) = Sink::try_new(handle) else { return };
        match rodio::Decoder::new(Cursor::new(wav)) {
            Ok(source) => {
                sink.append(source.repeat_infinite());
                self.audio_sink = Some(sink);
            }
            Err(_) => self.audio_sink = None,
        }
    }

    /// Drop the current audio track, stopping playback immediately.
    pub fn stop_audio(&mut self) {
        if let Some(sink) = self.audio_sink.take() {
            sink.stop();
        }
    }

    /// Pause the audio thread when the video preview isn't on screen (e.g. you
    /// backed out to the results list) and resume it when it is. This is the
    /// only coupling between audio and video, and it's just an on/off gate — no
    /// position syncing. Called each event loop tick.
    pub fn sync_audio(&mut self) {
        let Some(sink) = self.audio_sink.as_ref() else { return };
        if self.is_animating() {
            if sink.is_paused() {
                sink.play();
            }
        } else if !sink.is_paused() {
            sink.pause();
        }
    }

    /// Whether a multi-frame preview should be ticking right now.
    pub fn is_animating(&self) -> bool {
        self.show_image
            && matches!(self.screen, Screen::Detail)
            && matches!(&self.image_cache, Some((_, p)) if p.is_animated() && !p.is_ended())
    }

    /// Advance the video preview by one frame if its fps interval has elapsed.
    /// This is the entire video clock: a plain frame-by-frame pacer that never
    /// looks at the audio. Audio plays on rodio's thread; video ticks here;
    /// the two run side by side. For a short looping clip that's all the sync a
    /// preview needs, and it keeps playback smooth and consistent because every
    /// frame is just a blit of an already-encoded `Protocol`.
    pub fn tick(&mut self) {
        match self.last_tick {
            // First call after install: start the clock so frame 0 shows for a
            // full frame interval before we begin cycling.
            None => {
                self.last_tick = Some(Instant::now());
            }
            // Not yet time for the next frame.
            Some(t) if t.elapsed().as_secs_f64() < self.frame_interval() => {}
            // Interval elapsed: advance and reset the clock.
            Some(_) => {
                self.last_tick = Some(Instant::now());
                if let Some((_, preview)) = &mut self.image_cache {
                    preview.advance();
                }
                // Stop audio the moment the last frame is reached so picture
                // and sound end together instead of audio looping on alone.
                let ended = matches!(&self.image_cache, Some((_, p)) if p.is_ended());
                if ended {
                    self.stop_audio();
                }
            }
        }
    }

    /// Seconds each frame should be shown at the configured fps.
    pub fn frame_interval(&self) -> f64 {
        1.0 / self.config.fps.max(1) as f64
    }

    /// Time since the current frame was shown. Used to size the event-loop
    /// poll so it wakes up right when the *next* frame is due, instead of
    /// waiting a whole frame interval on top of the time spent drawing (which
    /// silently halved the effective frame rate).
    pub fn time_since_tick(&self) -> Duration {
        self.last_tick.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
    }

    pub fn open_settings(&mut self) {
        self.fps_input = self.config.fps;
        self.prev_screen = self.screen.clone();
        self.screen = Screen::Settings;
    }

    pub fn adjust_fps(&mut self, delta: i32) {
        let next = (self.fps_input as i32 + delta).clamp(1, 60);
        self.fps_input = next as u32;
    }

    /// Persist the working FPS value and leave the Settings screen.
    pub fn save_settings(&mut self) {
        self.config.set_fps(self.fps_input);
        match self.config.save() {
            Ok(()) => self.status_msg = format!("Saved: {} fps", self.config.fps),
            Err(e) => self.status_msg = format!("Could not save settings: {e}"),
        }
        self.screen = self.prev_screen.clone();
    }

    pub fn download_current(&mut self) {
        if let Some(post) = self.current_post().cloned() {
            self.status_msg = "Downloading...".to_string();
            match api::download_post(&post) {
                Ok(path) => self.status_msg = format!("Saved: {path}"),
                Err(e) => self.status_msg = format!("Download failed: {e}"),
            }
        }
    }
}

/// Resize and encode decoded video frames to [`Protocol`] objects off the UI thread.
///
/// For kitty: transmit each frame at its decoded pixel dimensions and fill the
/// entire `area` with unicode placeholders. The kitty terminal GPU-scales the
/// image to fill the placeholder grid, so the base64 payload stays small
/// (bounded by the decoded frame size) while the display fills the pane.
///
/// For other protocols (sixel/iterm2): pixel data = display size, so we scale
/// to fill the pane in CPU. This is slower but those protocols have no GPU path.
fn encode_frames(picker: &Picker, frames: Vec<DynamicImage>, area: Rect) -> Vec<Protocol> {
    use ratatui_image::protocol::kitty::Kitty;

    if picker.protocol_type() == ProtocolType::Kitty {
        // Detect tmux to wrap escape sequences correctly (mirrors ratatui-image's
        // own detection logic from picker/cap_parser.rs).
        let is_tmux = std::env::var("TERM").is_ok_and(|t| t.starts_with("tmux"))
            || std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "tmux");

        return frames
            .into_iter()
            .filter_map(|f| {
                // Pass `area` (the full inner pane) as the placeholder grid so
                // kitty GPU-scales the small frame to fill the pane. The pixel
                // data is transmitted at the decoded resolution — no CPU upscale.
                Kitty::new(f, area, rand::random(), is_tmux)
                    .ok()
                    .map(Protocol::Kitty)
            })
            .collect();
    }

    // Non-kitty protocols have no GPU scaling path; upscale in CPU to fill pane.
    frames
        .into_iter()
        .filter_map(|f| picker.new_protocol(f, area, Resize::Scale(None)).ok())
        .collect()
}
