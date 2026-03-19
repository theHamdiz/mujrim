//! Audio engine for chess sound effects and background music.
//!
//! Two distinct BGM tracks:
//! - **Menu BGM**: Rich Arabian/Hijaz ambient with arpeggios
//! - **Game BGM**: One of three moods — Playful, Joyful, Mystique
//!
//! All sounds are procedurally generated — no external audio files needed.

use std::io::Cursor;
use std::sync::Arc;

/// Which BGM track is playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmTrack {
    Menu,
    Game,
}

/// Game music mood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GameMood {
    /// Bouncy rhythm, major scale feel.
    Playful,
    /// Bright chords, warm harmonics, uplifting.
    Joyful,
    /// Deep drone, Hijaz scale, ethereal shimmer.
    Mystique,
}

impl std::fmt::Display for GameMood {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Playful => write!(f, "Playful"),
            Self::Joyful => write!(f, "Joyful"),
            Self::Mystique => write!(f, "Mystique"),
        }
    }
}

/// Holds the audio output stream and provides methods to play sound effects.
pub struct SoundEngine {
    _stream: rodio::OutputStream,
    stream_handle: rodio::OutputStreamHandle,
    move_sound: Vec<u8>,
    capture_sound: Vec<u8>,
    menu_bgm: Arc<Vec<u8>>,
    game_bgm_playful: Arc<Vec<u8>>,
    game_bgm_joyful: Arc<Vec<u8>>,
    game_bgm_mystique: Arc<Vec<u8>>,
    bgm_sink: Option<rodio::Sink>,
    current_track: Option<BgmTrack>,
    current_mood: GameMood,
    volume: f32,
}

impl SoundEngine {
    pub fn new() -> Option<Self> {
        let (stream, stream_handle) = rodio::OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            stream_handle,
            move_sound: generate_move_wav(),
            capture_sound: generate_capture_wav(),
            menu_bgm: Arc::new(generate_menu_bgm()),
            game_bgm_playful: Arc::new(generate_game_playful()),
            game_bgm_joyful: Arc::new(generate_game_joyful()),
            game_bgm_mystique: Arc::new(generate_game_mystique()),
            bgm_sink: None,
            current_track: None,
            current_mood: GameMood::Mystique,
            volume: 0.15,
        })
    }

    pub fn play_move(&self) {
        if let Ok(source) = rodio::Decoder::new(Cursor::new(self.move_sound.clone())) {
            let _ = self
                .stream_handle
                .play_raw(rodio::source::Source::convert_samples(source));
        }
    }

    pub fn play_capture(&self) {
        if let Ok(source) = rodio::Decoder::new(Cursor::new(self.capture_sound.clone())) {
            let _ = self
                .stream_handle
                .play_raw(rodio::source::Source::convert_samples(source));
        }
    }

    /// Play the specified BGM track with the current mood.
    pub fn play_bgm(&mut self, track: BgmTrack) {
        self.stop_bgm();

        let (data, vol) = match track {
            BgmTrack::Menu => (self.menu_bgm.clone(), self.volume * 1.2),
            BgmTrack::Game => {
                let d = match self.current_mood {
                    GameMood::Playful => self.game_bgm_playful.clone(),
                    GameMood::Joyful => self.game_bgm_joyful.clone(),
                    GameMood::Mystique => self.game_bgm_mystique.clone(),
                };
                (d, self.volume * 0.6)
            }
        };

        if let Ok(sink) = rodio::Sink::try_new(&self.stream_handle) {
            sink.set_volume(vol);
            if let Ok(source) = rodio::Decoder::new(Cursor::new((*data).clone())) {
                use rodio::Source;
                sink.append(source.repeat_infinite());
            }
            self.bgm_sink = Some(sink);
            self.current_track = Some(track);
        }
    }

    pub fn stop_bgm(&mut self) {
        if let Some(sink) = self.bgm_sink.take() {
            sink.stop();
        }
        self.current_track = None;
    }

    pub fn is_bgm_playing(&self) -> bool {
        self.bgm_sink
            .as_ref()
            .is_some_and(|s| !s.is_paused() && !s.empty())
    }

    pub fn toggle_bgm(&mut self, preferred_track: BgmTrack) -> bool {
        if self.is_bgm_playing() {
            self.stop_bgm();
            false
        } else {
            self.play_bgm(preferred_track);
            true
        }
    }

    /// Set the game music mood. If game BGM is playing, restarts with new mood.
    pub fn set_mood(&mut self, mood: GameMood) {
        self.current_mood = mood;
        if self.current_track == Some(BgmTrack::Game) {
            self.play_bgm(BgmTrack::Game);
        }
    }

    /// Set master volume (0.0 – 1.0).
    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        if let Some(ref sink) = self.bgm_sink {
            let effective = match self.current_track {
                Some(BgmTrack::Menu) => self.volume * 1.2,
                Some(BgmTrack::Game) => self.volume * 0.6,
                None => self.volume,
            };
            sink.set_volume(effective);
        }
    }
}

// ── Procedural WAV generation ───────────────────────────────────

fn generate_move_wav() -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as usize * 80) / 1000;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let tone = (2.0 * std::f64::consts::PI * 400.0 * t).sin();
        let tone2 = (2.0 * std::f64::consts::PI * 800.0 * t).sin() * 0.3;
        let env = (-t / 0.015).exp();
        s.push(((tone + tone2) * env * 0.6 * 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

fn generate_capture_wav() -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as usize * 100) / 1000;
    let mut s = Vec::with_capacity(n);
    let mut rng: u32 = 12345;
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let tone = (2.0 * std::f64::consts::PI * 600.0 * t).sin();
        let tone2 = (2.0 * std::f64::consts::PI * 1200.0 * t).sin() * 0.4;
        let noise = if t < 0.010 {
            rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
            ((rng >> 16) as f64 / 32768.0 - 1.0) * 0.5
        } else {
            0.0
        };
        let env = (-t / 0.012).exp();
        s.push(((tone + tone2 + noise) * env * 0.7 * 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

/// Menu BGM: Rich Arabian arpeggio, inviting atmosphere.
fn generate_menu_bgm() -> Vec<u8> {
    let sr = 44100u32;
    let dur = 20.0_f64;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);

    let scale: [f64; 7] = [110.0, 116.54, 138.59, 146.83, 164.81, 174.61, 207.65];
    let arp: [usize; 8] = [0, 2, 3, 4, 2, 5, 4, 3];
    let note_dur = 2.5_f64;
    let total = arp.len() as f64 * note_dur;

    for i in 0..n {
        let t = i as f64 / sr as f64;
        let vib = (2.0 * std::f64::consts::PI * 0.3 * t).sin() * 2.0;
        let drone = (2.0 * std::f64::consts::PI * (110.0 + vib) * t).sin() * 0.15;

        let at = t % total;
        let ai = (at / note_dur) as usize;
        let nt = at - (ai as f64 * note_dur);
        let freq = scale[arp[ai.min(arp.len() - 1)]];
        let att = (nt / 0.05).min(1.0);
        let dec = (-nt / 1.5).exp();
        let env = att * dec;
        let note = (2.0 * std::f64::consts::PI * freq * t).sin() * 0.12 * env;
        let shimmer = (2.0 * std::f64::consts::PI * freq * 2.0 * t).sin() * 0.04 * env;

        let pad_lfo = (2.0 * std::f64::consts::PI * 0.1 * t).sin() * 0.5 + 0.5;
        let pad = (2.0 * std::f64::consts::PI * 165.0 * t).sin() * 0.06 * pad_lfo;

        let mut mix = drone + note + shimmer + pad;
        let fd = 0.5;
        if t < fd {
            mix *= t / fd;
        } else if t > dur - fd {
            mix *= (dur - t) / fd;
        }

        s.push((mix * 32767.0).clamp(-32767.0, 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

/// Playful: Bouncy major-scale arpeggio with syncopated rhythm.
fn generate_game_playful() -> Vec<u8> {
    let sr = 44100u32;
    let dur = 24.0_f64;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);

    // C major pentatonic: C3, D3, E3, G3, A3, C4
    let scale: [f64; 6] = [130.81, 146.83, 164.81, 196.00, 220.00, 261.63];
    let pattern: [usize; 8] = [0, 2, 4, 5, 4, 2, 3, 1];
    let note_dur = 0.6_f64; // Faster for bouncy feel
    let total = pattern.len() as f64 * note_dur;

    for i in 0..n {
        let t = i as f64 / sr as f64;

        // Light bass pulse at root
        let bass_phase = (t / 1.2).fract();
        let bass_env = (-bass_phase * 4.0).exp();
        let bass = (2.0 * std::f64::consts::PI * 130.81 * t).sin() * 0.08 * bass_env;

        // Bouncy arpeggio
        let at = t % total;
        let ai = (at / note_dur) as usize;
        let nt = at - (ai as f64 * note_dur);
        let freq = scale[pattern[ai.min(pattern.len() - 1)]];

        // Snappy attack, moderate decay
        let att = (nt / 0.02).min(1.0);
        let dec = (-nt / 0.4).exp();
        let env = att * dec;
        let note = (2.0 * std::f64::consts::PI * freq * t).sin() * 0.10 * env;
        // Add brightness with octave
        let bright = (2.0 * std::f64::consts::PI * freq * 2.0 * t).sin() * 0.03 * env;

        // Subtle shaker rhythm (noise bursts)
        let beat_phase = (t * 3.33).fract(); // ~200bpm
        let shaker = if beat_phase < 0.05 {
            let pseudo = ((t * 43758.5453).sin() * 2.0).fract();
            pseudo * 0.02 * (1.0 - beat_phase / 0.05)
        } else {
            0.0
        };

        let mut mix = bass + note + bright + shaker;
        let fd = 0.8;
        if t < fd {
            mix *= t / fd;
        } else if t > dur - fd {
            mix *= (dur - t) / fd;
        }

        s.push((mix * 32767.0).clamp(-32767.0, 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

/// Joyful: Warm chords with bright harmonics, uplifting.
fn generate_game_joyful() -> Vec<u8> {
    let sr = 44100u32;
    let dur = 28.0_f64;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);

    // Chord progression: I - vi - IV - V in C major
    let chords: [[f64; 3]; 4] = [
        [130.81, 164.81, 196.00], // C-E-G
        [110.00, 130.81, 164.81], // A-C-E
        [116.54, 146.83, 174.61], // F-A-C (approx)
        [123.47, 155.56, 196.00], // G-B-D (approx)
    ];
    let chord_dur = 3.5_f64;
    let total = chords.len() as f64 * chord_dur;

    for i in 0..n {
        let t = i as f64 / sr as f64;

        // Warm chord pad
        let ct = t % total;
        let ci = (ct / chord_dur) as usize;
        let chord = &chords[ci.min(chords.len() - 1)];

        let cross_t = ct - ci as f64 * chord_dur;
        let pad_att = (cross_t / 0.3).min(1.0);
        let pad_dec = if cross_t > chord_dur - 0.3 {
            (chord_dur - cross_t) / 0.3
        } else {
            1.0
        };
        let pad_env = pad_att * pad_dec;

        let mut chord_sum = 0.0_f64;
        for &freq in chord {
            chord_sum += (2.0 * std::f64::consts::PI * freq * t).sin();
            // Octave shimmer
            chord_sum += (2.0 * std::f64::consts::PI * freq * 2.0 * t).sin() * 0.2;
        }
        let pad = chord_sum * 0.04 * pad_env;

        // Gentle arpeggio picking top note + shimmer
        let arp_phase = (ct / 0.7).fract();
        let arp_env = (-arp_phase * 3.0).exp() * 0.5;
        let arp_idx = ((ct / 0.7) as usize) % 3;
        let arp_freq = chord[arp_idx] * 2.0; // One octave up
        let arp = (2.0 * std::f64::consts::PI * arp_freq * t).sin() * 0.05 * arp_env;

        // Sub bass
        let sub = (2.0 * std::f64::consts::PI * chord[0] * 0.5 * t).sin() * 0.05 * pad_env;

        let mut mix = pad + arp + sub;
        let fd = 1.0;
        if t < fd {
            mix *= t / fd;
        } else if t > dur - fd {
            mix *= (dur - t) / fd;
        }

        s.push((mix * 32767.0).clamp(-32767.0, 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

/// Mystique: Deep Hijaz drone with ethereal shimmer.
fn generate_game_mystique() -> Vec<u8> {
    let sr = 44100u32;
    let dur = 30.0_f64;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);

    for i in 0..n {
        let t = i as f64 / sr as f64;

        // Deep E2 drone with slow vibrato
        let vib = (2.0 * std::f64::consts::PI * 0.15 * t).sin() * 1.0;
        let drone = (2.0 * std::f64::consts::PI * (82.4 + vib) * t).sin() * 0.10;

        // Sub-bass E1
        let sub = (2.0 * std::f64::consts::PI * 41.2 * t).sin() * 0.06;

        // Breathing pad at the fifth (B2 ≈ 123.5Hz)
        let breath = (2.0 * std::f64::consts::PI * 0.05 * t).sin() * 0.5 + 0.5;
        let pad = (2.0 * std::f64::consts::PI * 123.5 * t).sin() * 0.04 * breath;

        // Hijaz scale ghost notes (very sparse, ethereal)
        let ghost_cycle = 8.0_f64; // One ghost note every 8 seconds
        let ghost_t = t % ghost_cycle;
        let ghost_freqs: [f64; 4] = [82.4, 87.31, 103.83, 110.0]; // E-F-Ab-A (Hijaz fragment)
        let ghost_idx = ((t / ghost_cycle) as usize) % ghost_freqs.len();
        let ghost_freq = ghost_freqs[ghost_idx];
        let ghost_att = (ghost_t / 0.1).min(1.0);
        let ghost_dec = (-ghost_t / 2.0).exp();
        let ghost = (2.0 * std::f64::consts::PI * ghost_freq * 2.0 * t).sin()
            * 0.03
            * ghost_att
            * ghost_dec;

        // Distant shimmer
        let shim_lfo = (2.0 * std::f64::consts::PI * 0.03 * t).sin().max(0.0);
        let shimmer = (2.0 * std::f64::consts::PI * 329.6 * t).sin() * 0.015 * shim_lfo;

        let mut mix = drone + sub + pad + ghost + shimmer;
        let fd = 1.0;
        if t < fd {
            mix *= t / fd;
        } else if t > dur - fd {
            mix *= (dur - t) / fd;
        }

        s.push((mix * 32767.0).clamp(-32767.0, 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let file_size = 36 + data_size;
    let mut wav = Vec::with_capacity(44 + data_size as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_wav() {
        let w = generate_move_wav();
        assert!(w.len() > 44);
        assert_eq!(&w[0..4], b"RIFF");
    }

    #[test]
    fn test_capture_wav() {
        let w = generate_capture_wav();
        assert!(w.len() > 44);
    }

    #[test]
    fn test_menu_bgm() {
        let w = generate_menu_bgm();
        assert!(w.len() > 44100 * 20 * 2);
    }

    #[test]
    fn test_game_playful() {
        let w = generate_game_playful();
        assert!(w.len() > 44100 * 24 * 2);
    }

    #[test]
    fn test_game_joyful() {
        let w = generate_game_joyful();
        assert!(w.len() > 44100 * 28 * 2);
    }

    #[test]
    fn test_game_mystique() {
        let w = generate_game_mystique();
        assert!(w.len() > 44100 * 30 * 2);
    }
}
