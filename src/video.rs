use crate::api;
use ffmpeg_sidecar::command::{ffmpeg_is_installed, FfmpegCommand};
use ffmpeg_sidecar::download::auto_download;
use image::{DynamicImage, RgbImage};

/// Width (in pixels) that decoded frames are scaled to. Height keeps aspect
/// ratio. Kitty GPU-scales this to fill the pane, so a modest source size keeps
/// base64 payloads small while the display fills the preview regardless of
/// terminal size. Non-kitty protocols upscale in CPU during encode.
const SCALE_W: u32 = 360;
/// Maximum seconds of video to decode (longer clips are truncated).
const MAX_SECONDS: u32 = 10;
/// Hard cap on decoded frames regardless of fps/duration.
const MAX_FRAMES: u32 = 150;

/// Decoded preview payload: the sampled video frames plus, when the source has
/// an audio track, the extracted audio as in-memory WAV bytes.
pub struct VideoData {
    pub frames: Vec<DynamicImage>,
    pub audio: Option<Vec<u8>>,
}

/// Download a video (webm/mp4/gif) and decode it into a sequence of frames
/// sampled at `fps`, scaled to a preview size, plus the audio track (as WAV
/// bytes) when present. Uses a bundled ffmpeg (auto-downloaded on first use).
///
/// This is blocking and intended to run on a background thread.
pub fn fetch_video_frames(
    url: &str,
    ext: &str,
    post_id: u64,
    fps: u32,
) -> Result<VideoData, String> {
    // Ensure an ffmpeg binary is available (no-op if already installed).
    if !ffmpeg_is_installed() {
        auto_download().map_err(|e| format!("ffmpeg download failed: {e}"))?;
    }

    // e621 requires a User-Agent, so download to a temp file rather than
    // letting ffmpeg fetch the URL itself.
    let bytes = api::get_bytes(url)?;

    let ext = if ext.is_empty() { "bin" } else { ext };
    let tmp = std::env::temp_dir().join(format!("con621_{post_id}.{ext}"));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;

    let fps = fps.clamp(1, 60);
    let max_frames = (fps * MAX_SECONDS).clamp(1, MAX_FRAMES);

    // Decode frames first; on failure still clean up the temp file.
    let frames = match decode(&tmp, fps, max_frames) {
        Ok(frames) => frames,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if frames.is_empty() {
        let _ = std::fs::remove_file(&tmp);
        return Err("no frames decoded".to_string());
    }

    // Extract audio for the same duration as the decoded frames so picture and
    // sound stay aligned across loops. Best-effort: silent clips yield None.
    let duration = frames.len() as f64 / fps as f64;
    let audio = extract_audio(&tmp, duration);

    let _ = std::fs::remove_file(&tmp);
    Ok(VideoData { frames, audio })
}

/// Extract up to `duration` seconds of audio from `path` as WAV (PCM s16le,
/// 44.1 kHz stereo) into memory. Returns None if the source has no audio track
/// or extraction fails for any reason — sound is a best-effort extra.
fn extract_audio(path: &std::path::Path, duration: f64) -> Option<Vec<u8>> {
    let wav = std::env::temp_dir().join(format!(
        "con621_audio_{}.wav",
        std::process::id()
    ));
    let dur = format!("{:.3}", duration.max(0.1));

    let mut child = FfmpegCommand::new()
        .input(path.to_string_lossy())
        .args([
            "-vn", // drop video
            "-t", &dur,
            "-ac", "2",
            "-ar", "44100",
            "-f", "wav",
            "-y",
            &wav.to_string_lossy(),
        ])
        .spawn()
        .ok()?;

    // Drain ffmpeg's event/log stream so it runs to completion.
    if let Ok(iter) = child.iter() {
        iter.for_each(|_| {});
    }
    let _ = child.wait();

    let bytes = std::fs::read(&wav).ok();
    let _ = std::fs::remove_file(&wav);
    // A bare WAV header with no samples is ~44 bytes; treat that as "no audio".
    bytes.filter(|b| b.len() > 64)
}

fn decode(path: &std::path::Path, fps: u32, max_frames: u32) -> Result<Vec<DynamicImage>, String> {
    let mut child = FfmpegCommand::new()
        .input(path.to_string_lossy())
        .args([
            "-vf",
            &format!("fps={fps},scale={SCALE_W}:-2:flags=lanczos"),
        ])
        .frames(max_frames)
        .rawvideo() // -f rawvideo -pix_fmt rgb24 -> stdout
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut frames = Vec::new();
    for frame in child.iter().map_err(|e| e.to_string())?.filter_frames() {
        if let Some(buf) = RgbImage::from_raw(frame.width, frame.height, frame.data) {
            frames.push(DynamicImage::ImageRgb8(buf));
        }
        if frames.len() >= max_frames as usize {
            break;
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(frames)
}
