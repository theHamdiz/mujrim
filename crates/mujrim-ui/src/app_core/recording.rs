//! Screen recording — cross-platform screen capture and video export.
//!
//! Uses `xcap` for frame capture on macOS and Windows.
//! Frames are collected in memory and exported via ffmpeg (if available)
//! or as a sequence of PNG images.

use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

/// Recording state shared between the UI and capture thread.
#[derive(Clone)]
pub struct RecordingEngine {
    inner: Arc<Mutex<RecordingInner>>,
}

struct RecordingInner {
    state: RecordState,
    frames: Vec<CapturedFrame>,
    start_time: Option<Instant>,
    capture_fps: u32,
    frame_bytes: usize,
}

struct CapturedFrame {
    image: RgbaImage,
    _timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    Idle,
    Recording,
    Saving,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Captured,
    Inactive,
    Unavailable,
    MemoryLimitReached,
}

const MAX_CAPTURE_WIDTH: u32 = 1280;
const MAX_FRAME_MEMORY_BYTES: usize = 256 * 1024 * 1024;

impl RecordingEngine {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingInner {
                state: RecordState::Idle,
                frames: Vec::new(),
                start_time: None,
                capture_fps: 10,
                frame_bytes: 0,
            })),
        }
    }

    pub fn state(&self) -> RecordState {
        self.lock_inner().state
    }

    /// Start recording — begins capturing frames.
    pub fn start(&self) {
        let mut inner = self.lock_inner();
        inner.state = RecordState::Recording;
        inner.frames.clear();
        inner.frame_bytes = 0;
        inner.start_time = Some(Instant::now());
    }

    /// Discard a recording without encoding it.
    pub fn cancel(&self) {
        let mut inner = self.lock_inner();
        inner.state = RecordState::Idle;
        inner.frames.clear();
        inner.frame_bytes = 0;
        inner.start_time = None;
    }

    /// Capture a single frame (called from a timer tick).
    /// Uses xcap to capture the entire screen and crops to the window area.
    pub fn capture_frame(&self) -> CaptureOutcome {
        let mut inner = self.lock_inner();
        if inner.state != RecordState::Recording {
            return CaptureOutcome::Inactive;
        }

        let timestamp_ms = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        // Use xcap to capture the primary monitor
        let Some(frame) = capture_screen() else {
            return CaptureOutcome::Unavailable;
        };
        let frame = downscale_capture(frame);
        let frame_bytes = frame.as_raw().len();
        if inner.frame_bytes.saturating_add(frame_bytes) > MAX_FRAME_MEMORY_BYTES {
            return CaptureOutcome::MemoryLimitReached;
        }
        inner.frame_bytes += frame_bytes;
        inner.frames.push(CapturedFrame {
            image: frame,
            _timestamp_ms: timestamp_ms,
        });
        CaptureOutcome::Captured
    }

    /// Stop recording and save to the given path.
    /// Returns the number of frames captured.
    pub fn stop_and_save(&self, output_path: PathBuf) -> Result<usize, String> {
        let (frames, fps) = {
            let mut inner = self.lock_inner();
            inner.state = RecordState::Saving;
            let frames = std::mem::take(&mut inner.frames);
            inner.frame_bytes = 0;
            let fps = inner.capture_fps;
            drop(inner);
            (frames, fps)
        };

        let frame_count = frames.len();
        if frame_count == 0 {
            let mut inner = self.lock_inner();
            inner.state = RecordState::Idle;
            drop(inner);
            return Err("No frames captured".to_string());
        }

        // Try to use ffmpeg for video encoding
        let result = if has_ffmpeg() {
            save_with_ffmpeg(&frames, fps, &output_path)
        } else {
            // Fallback: save as animated GIF
            let gif_path = output_path.with_extension("gif");
            save_as_gif(&frames, fps, &gif_path)
        };

        let mut inner = self.lock_inner();
        inner.state = RecordState::Idle;
        drop(inner);

        result.map(|_| frame_count)
    }

    fn lock_inner(&self) -> MutexGuard<'_, RecordingInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn downscale_capture(frame: RgbaImage) -> RgbaImage {
    if frame.width() <= MAX_CAPTURE_WIDTH {
        return frame;
    }
    let height = frame
        .height()
        .saturating_mul(MAX_CAPTURE_WIDTH)
        .checked_div(frame.width())
        .unwrap_or(1)
        .max(1);
    image::imageops::resize(
        &frame,
        MAX_CAPTURE_WIDTH,
        height,
        image::imageops::FilterType::Triangle,
    )
}

/// Capture the screen using xcap.
fn capture_screen() -> Option<RgbaImage> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        use xcap::Monitor;

        let monitors = Monitor::all().ok()?;
        let primary = monitors.into_iter().next()?;
        let screenshot = primary.capture_image().ok()?;
        Some(screenshot)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Check if ffmpeg is available on the system.
fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Save frames as video using ffmpeg.
fn save_with_ffmpeg(frames: &[CapturedFrame], fps: u32, output_path: &Path) -> Result<(), String> {
    use std::io::Write;

    // Create a temp directory for frame images
    let temp_dir = std::env::temp_dir().join(format!(
        "mujrim-recording-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis())
    ));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("Failed to create recording directory: {error}"))?;

    // Save frames as individual PNGs
    for (i, frame) in frames.iter().enumerate() {
        let path = temp_dir.join(format!("frame_{:06}.png", i));
        if let Err(error) = frame.image.save(&path) {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(format!("Failed to save frame {i}: {error}"));
        }
    }

    // Build ffmpeg command
    let pattern = temp_dir.join("frame_%06d.png");
    let output_str = output_path.to_string_lossy();

    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y", // overwrite
            "-framerate",
            &fps.to_string(), // input fps
            "-i",
            &pattern.to_string_lossy(), // input pattern
            "-c:v",
            "libx264", // H.264 codec
            "-pix_fmt",
            "yuv420p", // compatibility
            "-crf",
            "18", // quality (lower = better)
            "-preset",
            "fast",      // encoding speed
            &output_str, // output file
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;

    // Clean up temp frames
    let _ = std::fs::remove_dir_all(&temp_dir);

    if output.status.success() {
        // If user wanted audio too, write a note
        let _ = writeln!(
            std::io::stderr(),
            "Video saved to {}. Note: audio recording requires additional setup.",
            output_str
        );
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().last().unwrap_or("no error detail");
        Err(format!("ffmpeg exited with {}: {detail}", output.status))
    }
}

/// Save frames as an animated GIF (fallback when ffmpeg unavailable).
fn save_as_gif(frames: &[CapturedFrame], fps: u32, output_path: &Path) -> Result<(), String> {
    let first = frames.first().ok_or("No frames")?;
    let width = first.image.width() as u16;
    let height = first.image.height() as u16;

    // Scale down for reasonable GIF size
    let scale = if width > 640 {
        640.0 / width as f32
    } else {
        1.0
    };
    let new_w = (width as f32 * scale) as u32;
    let new_h = (height as f32 * scale) as u32;

    let file =
        std::fs::File::create(output_path).map_err(|e| format!("Failed to create file: {e}"))?;

    let mut encoder = gif::Encoder::new(file, new_w as u16, new_h as u16, &[])
        .map_err(|e| format!("GIF encoder error: {e}"))?;

    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("GIF repeat error: {e}"))?;

    for capture in frames {
        let resized = if scale < 1.0 {
            image::imageops::resize(
                &capture.image,
                new_w,
                new_h,
                image::imageops::FilterType::Nearest,
            )
        } else {
            capture.image.clone()
        };

        // Simple quantization: build a palette from the image
        let (palette, indices) = super::gif_export::quantize_frame(&resized);

        let frame = gif::Frame {
            width: new_w as u16,
            height: new_h as u16,
            delay: gif_frame_delay(fps),
            palette: Some(palette),
            buffer: std::borrow::Cow::Owned(indices),
            ..gif::Frame::default()
        };

        encoder
            .write_frame(&frame)
            .map_err(|e| format!("Failed to write GIF frame: {e}"))?;
    }

    Ok(())
}

fn gif_frame_delay(fps: u32) -> u16 {
    u16::try_from(
        100_u32
            .saturating_add(fps / 2)
            .checked_div(fps.max(1))
            .unwrap_or(1),
    )
    .unwrap_or(u16::MAX)
    .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_engine_state() {
        let engine = RecordingEngine::new();
        assert_eq!(engine.state(), RecordState::Idle);
        engine.start();
        assert_eq!(engine.state(), RecordState::Recording);
        engine.cancel();
        assert_eq!(engine.state(), RecordState::Idle);
    }

    #[test]
    fn capture_downscaling_preserves_aspect_ratio() {
        let frame = RgbaImage::new(2560, 1440);
        let resized = downscale_capture(frame);
        assert_eq!(resized.dimensions(), (1280, 720));
    }

    #[test]
    fn gif_delay_tracks_capture_rate() {
        assert_eq!(gif_frame_delay(10), 10);
        assert_eq!(gif_frame_delay(25), 4);
        assert_eq!(gif_frame_delay(0), 100);
    }

    #[test]
    fn test_has_ffmpeg_does_not_panic() {
        // Just verify it doesn't panic; result depends on system
        let _ = has_ffmpeg();
    }
}
