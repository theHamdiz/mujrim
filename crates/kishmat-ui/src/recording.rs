//! Screen recording — cross-platform screen capture and video export.
//!
//! Uses `xcap` for frame capture on macOS and Windows.
//! Frames are collected in memory and exported via ffmpeg (if available)
//! or as a sequence of PNG images.

use image::RgbaImage;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

impl RecordingEngine {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingInner {
                state: RecordState::Idle,
                frames: Vec::new(),
                start_time: None,
                capture_fps: 10,
            })),
        }
    }

    pub fn state(&self) -> RecordState {
        self.inner.lock().unwrap().state
    }

    /// Start recording — begins capturing frames.
    pub fn start(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = RecordState::Recording;
        inner.frames.clear();
        inner.start_time = Some(Instant::now());
    }

    /// Capture a single frame (called from a timer tick).
    /// Uses xcap to capture the entire screen and crops to the window area.
    pub fn capture_frame(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.state != RecordState::Recording {
            return;
        }

        let timestamp_ms = inner.start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        // Use xcap to capture the primary monitor
        if let Some(frame) = capture_screen() {
            inner.frames.push(CapturedFrame {
                image: frame,
                _timestamp_ms: timestamp_ms,
            });
        }
    }

    /// Stop recording and save to the given path.
    /// Returns the number of frames captured.
    pub fn stop_and_save(&self, output_path: PathBuf) -> Result<usize, String> {
        let (frames, fps) = {
            let mut inner = self.inner.lock().unwrap();
            inner.state = RecordState::Saving;
            let frames = std::mem::take(&mut inner.frames);
            let fps = inner.capture_fps;
            (frames, fps)
        };

        let frame_count = frames.len();
        if frame_count == 0 {
            let mut inner = self.inner.lock().unwrap();
            inner.state = RecordState::Idle;
            return Err("No frames captured".to_string());
        }

        // Try to use ffmpeg for video encoding
        let result = if has_ffmpeg() {
            save_with_ffmpeg(&frames, fps, &output_path)
        } else {
            // Fallback: save as animated GIF
            let gif_path = output_path.with_extension("gif");
            save_as_gif(&frames, &gif_path)
        };

        let mut inner = self.inner.lock().unwrap();
        inner.state = RecordState::Idle;

        result.map(|_| frame_count)
    }
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
fn save_with_ffmpeg(frames: &[CapturedFrame], fps: u32, output_path: &PathBuf) -> Result<(), String> {
    use std::io::Write;

    // Create a temp directory for frame images
    let temp_dir = std::env::temp_dir().join("kishmat_recording");
    let _ = std::fs::create_dir_all(&temp_dir);

    // Save frames as individual PNGs
    for (i, frame) in frames.iter().enumerate() {
        let path = temp_dir.join(format!("frame_{:06}.png", i));
        frame.image.save(&path)
            .map_err(|e| format!("Failed to save frame {i}: {e}"))?;
    }

    // Build ffmpeg command
    let pattern = temp_dir.join("frame_%06d.png");
    let output_str = output_path.to_string_lossy();

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",                                    // overwrite
            "-framerate", &fps.to_string(),          // input fps
            "-i", &pattern.to_string_lossy(),        // input pattern
            "-c:v", "libx264",                       // H.264 codec
            "-pix_fmt", "yuv420p",                   // compatibility
            "-crf", "18",                            // quality (lower = better)
            "-preset", "fast",                       // encoding speed
            &output_str,                             // output file
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;

    // Clean up temp frames
    let _ = std::fs::remove_dir_all(&temp_dir);

    if status.success() {
        // If user wanted audio too, write a note
        let _ = writeln!(
            std::io::stderr(),
            "Video saved to {}. Note: audio recording requires additional setup.",
            output_str
        );
        Ok(())
    } else {
        Err(format!("ffmpeg exited with status: {status}"))
    }
}

/// Save frames as an animated GIF (fallback when ffmpeg unavailable).
fn save_as_gif(frames: &[CapturedFrame], output_path: &PathBuf) -> Result<(), String> {
    let first = frames.first().ok_or("No frames")?;
    let width = first.image.width() as u16;
    let height = first.image.height() as u16;

    // Scale down for reasonable GIF size
    let scale = if width > 640 { 640.0 / width as f32 } else { 1.0 };
    let new_w = (width as f32 * scale) as u32;
    let new_h = (height as f32 * scale) as u32;

    let file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create file: {e}"))?;

    let mut encoder = gif::Encoder::new(file, new_w as u16, new_h as u16, &[])
        .map_err(|e| format!("GIF encoder error: {e}"))?;

    encoder.set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("GIF repeat error: {e}"))?;

    for capture in frames {
        let resized = if scale < 1.0 {
            image::imageops::resize(&capture.image, new_w, new_h, image::imageops::FilterType::Nearest)
        } else {
            capture.image.clone()
        };

        // Simple quantization: build a palette from the image
        let (palette, indices) = crate::gif_export::quantize_frame(&resized);

        let mut frame = gif::Frame::default();
        frame.width = new_w as u16;
        frame.height = new_h as u16;
        frame.delay = 10; // 100ms per frame = 10fps
        frame.palette = Some(palette);
        frame.buffer = std::borrow::Cow::Owned(indices);

        encoder.write_frame(&frame)
            .map_err(|e| format!("Failed to write GIF frame: {e}"))?;
    }

    Ok(())
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
    }

    #[test]
    fn test_has_ffmpeg_does_not_panic() {
        // Just verify it doesn't panic; result depends on system
        let _ = has_ffmpeg();
    }
}
