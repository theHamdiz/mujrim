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

/// Short board sound palette.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SoundTheme {
    #[default]
    Wood,
    Crystal,
    Soft,
    Marble,
    Digital,
    Arena,
    Glass,
}

impl SoundTheme {
    pub const ALL: [Self; 7] = [
        Self::Wood,
        Self::Crystal,
        Self::Soft,
        Self::Marble,
        Self::Digital,
        Self::Arena,
        Self::Glass,
    ];
}

impl std::fmt::Display for SoundTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wood => f.write_str("Wood"),
            Self::Crystal => f.write_str("Crystal"),
            Self::Soft => f.write_str("Soft"),
            Self::Marble => f.write_str("Marble"),
            Self::Digital => f.write_str("Digital"),
            Self::Arena => f.write_str("Arena"),
            Self::Glass => f.write_str("Glass"),
        }
    }
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
    /// Spare fifths, slow pulse — concentration.
    Focus,
    /// Dotted martial rhythm.
    March,
    /// Low strings and distant bells.
    Nocturne,
}

impl GameMood {
    pub const ALL: [Self; 6] = [
        Self::Playful,
        Self::Joyful,
        Self::Mystique,
        Self::Focus,
        Self::March,
        Self::Nocturne,
    ];
}

impl std::fmt::Display for GameMood {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Playful => write!(f, "Playful"),
            Self::Joyful => write!(f, "Joyful"),
            Self::Mystique => write!(f, "Mystique"),
            Self::Focus => write!(f, "Focus"),
            Self::March => write!(f, "March"),
            Self::Nocturne => write!(f, "Nocturne"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxKind {
    Move,
    Capture,
    Check,
    Castle,
    Promote,
    Checkmate,
    Resign,
    Forfeit,
    TournamentOver,
}

impl SfxKind {
    pub fn from_move(mv: types::Move, captured: bool, gives_check: bool) -> Self {
        if gives_check {
            Self::Check
        } else if mv.is_castling() {
            Self::Castle
        } else if mv.is_promotion() {
            Self::Promote
        } else if captured || mv.is_capture() {
            Self::Capture
        } else {
            Self::Move
        }
    }
}

/// Holds the audio output stream and provides methods to play sound effects.
pub struct SoundEngine {
    _stream: rodio::OutputStream,
    stream_handle: rodio::OutputStreamHandle,
    move_sound: Arc<[u8]>,
    capture_sound: Arc<[u8]>,
    menu_bgm: Arc<[u8]>,
    game_bgm_playful: Arc<[u8]>,
    game_bgm_joyful: Arc<[u8]>,
    game_bgm_mystique: Arc<[u8]>,
    game_bgm_focus: Arc<[u8]>,
    game_bgm_march: Arc<[u8]>,
    game_bgm_nocturne: Arc<[u8]>,
    check_sound: Arc<[u8]>,
    castle_sound: Arc<[u8]>,
    promote_sound: Arc<[u8]>,
    checkmate_sound: Arc<[u8]>,
    resign_sound: Arc<[u8]>,
    forfeit_sound: Arc<[u8]>,
    tournament_over_sound: Arc<[u8]>,
    bgm_sink: Option<rodio::Sink>,
    current_track: Option<BgmTrack>,
    current_mood: GameMood,
    volume: f32,
}

impl SoundEngine {
    pub fn new() -> Option<Self> {
        let (stream, stream_handle) = rodio::OutputStream::try_default().ok()?;
        let (move_sound, capture_sound) = generate_sound_theme(SoundTheme::Wood);
        Some(Self {
            _stream: stream,
            stream_handle,
            move_sound,
            capture_sound,
            menu_bgm: generate_menu_bgm().into(),
            game_bgm_playful: generate_game_playful().into(),
            game_bgm_joyful: generate_game_joyful().into(),
            game_bgm_mystique: generate_game_mystique().into(),
            game_bgm_focus: generate_game_focus().into(),
            game_bgm_march: generate_game_march().into(),
            game_bgm_nocturne: generate_game_nocturne().into(),
            check_sound: generate_check_wav().into(),
            castle_sound: generate_tone_wav(330.0, 495.0, 160, 0.38).into(),
            promote_sound: generate_tone_wav(523.0, 784.0, 180, 0.48).into(),
            checkmate_sound: generate_motif_wav(&[(392.0, 140), (311.1, 160), (246.9, 280)], 0.48)
                .into(),
            resign_sound: generate_motif_wav(&[(196.0, 180), (146.8, 320)], 0.42).into(),
            forfeit_sound: generate_motif_wav(&[(164.8, 160), (130.8, 140), (98.0, 340)], 0.46)
                .into(),
            tournament_over_sound: generate_motif_wav(
                &[(523.3, 90), (659.3, 90), (783.9, 110), (1046.5, 280)],
                0.44,
            )
            .into(),
            bgm_sink: None,
            current_track: None,
            current_mood: GameMood::Mystique,
            volume: 0.15,
        })
    }

    pub fn play_move(&self) {
        self.play_buffer(&self.move_sound);
    }

    pub fn play_capture(&self) {
        self.play_buffer(&self.capture_sound);
    }

    pub fn play_sfx(&self, sfx_on: bool, kind: SfxKind) {
        if !sfx_on {
            return;
        }
        match kind {
            SfxKind::Move => self.play_move(),
            SfxKind::Capture => self.play_capture(),
            SfxKind::Check => self.play_buffer(&self.check_sound),
            SfxKind::Castle => self.play_buffer(&self.castle_sound),
            SfxKind::Promote => self.play_buffer(&self.promote_sound),
            SfxKind::Checkmate => self.play_buffer(&self.checkmate_sound),
            SfxKind::Resign => self.play_buffer(&self.resign_sound),
            SfxKind::Forfeit => self.play_buffer(&self.forfeit_sound),
            SfxKind::TournamentOver => self.play_buffer(&self.tournament_over_sound),
        }
    }

    fn play_buffer(&self, data: &Arc<[u8]>) {
        if let Ok(source) = rodio::Decoder::new(Cursor::new(Arc::clone(data))) {
            let _ = self
                .stream_handle
                .play_raw(rodio::source::Source::convert_samples(source));
        }
    }

    /// Play the specified BGM track with the current mood.
    pub fn play_bgm(&mut self, track: BgmTrack) {
        self.play_bgm_gated(true, track);
    }

    pub fn play_bgm_gated(&mut self, enabled: bool, track: BgmTrack) {
        self.stop_bgm();
        if !enabled {
            return;
        }

        let (data, vol) = match track {
            BgmTrack::Menu => (Arc::clone(&self.menu_bgm), self.volume * 1.2),
            BgmTrack::Game => {
                let d = match self.current_mood {
                    GameMood::Playful => Arc::clone(&self.game_bgm_playful),
                    GameMood::Joyful => Arc::clone(&self.game_bgm_joyful),
                    GameMood::Mystique => Arc::clone(&self.game_bgm_mystique),
                    GameMood::Focus => Arc::clone(&self.game_bgm_focus),
                    GameMood::March => Arc::clone(&self.game_bgm_march),
                    GameMood::Nocturne => Arc::clone(&self.game_bgm_nocturne),
                };
                (d, self.volume * 0.6)
            }
        };

        if let Ok(sink) = rodio::Sink::try_new(&self.stream_handle) {
            sink.set_volume(vol);
            if let Ok(source) = rodio::Decoder::new(Cursor::new(data)) {
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

    pub fn set_sound_theme(&mut self, theme: SoundTheme) {
        (self.move_sound, self.capture_sound) = generate_sound_theme(theme);
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

fn generate_sound_theme(theme: SoundTheme) -> (Arc<[u8]>, Arc<[u8]>) {
    let (move_sound, capture_sound) = match theme {
        SoundTheme::Wood => (generate_move_wav(), generate_capture_wav()),
        SoundTheme::Crystal => (
            generate_tone_wav(720.0, 1_440.0, 65, 0.45),
            generate_tone_wav(520.0, 1_560.0, 95, 0.55),
        ),
        SoundTheme::Soft => (
            generate_tone_wav(260.0, 390.0, 70, 0.22),
            generate_tone_wav(190.0, 310.0, 100, 0.28),
        ),
        SoundTheme::Marble => (
            generate_tone_wav(210.0, 420.0, 90, 0.34),
            generate_tone_wav(160.0, 640.0, 120, 0.4),
        ),
        SoundTheme::Digital => (
            generate_tone_wav(980.0, 1_960.0, 55, 0.3),
            generate_tone_wav(740.0, 1_480.0, 80, 0.36),
        ),
        SoundTheme::Arena => (
            generate_tone_wav(140.0, 280.0, 110, 0.5),
            generate_tone_wav(90.0, 360.0, 140, 0.55),
        ),
        SoundTheme::Glass => (
            generate_tone_wav(1_176.0, 2_352.0, 70, 0.28),
            generate_tone_wav(988.0, 1_976.0, 95, 0.32),
        ),
    };
    (move_sound.into(), capture_sound.into())
}

fn generate_check_wav() -> Vec<u8> {
    generate_motif_wav(&[(880.0, 70), (1_174.7, 110)], 0.46)
}

fn generate_motif_wav(notes: &[(f64, usize)], gain: f64) -> Vec<u8> {
    let sample_rate = 44_100u32;
    let total_ms: usize = notes.iter().map(|(_, ms)| *ms).sum();
    let sample_count = sample_rate as usize * total_ms.max(1) / 1_000;
    let mut samples = vec![0i16; sample_count];
    let mut cursor = 0usize;
    for &(hz, duration_ms) in notes {
        let count = sample_rate as usize * duration_ms / 1_000;
        for index in 0..count {
            let time = index as f64 / f64::from(sample_rate);
            let envelope = (-time / (duration_ms as f64 / 3_200.0)).exp();
            let sample = (std::f64::consts::TAU * hz * time).sin()
                + 0.22 * (std::f64::consts::TAU * hz * 2.0 * time).sin();
            if let Some(slot) = samples.get_mut(cursor + index) {
                *slot = (sample * envelope * gain * 32_767.0) as i16;
            }
        }
        cursor = cursor.saturating_add(count);
    }
    encode_wav(&samples, sample_rate)
}

pub fn game_end_sfx(board: &mut types::Board, white_score: f64) -> Option<SfxKind> {
    if board.is_checkmate() {
        Some(SfxKind::Checkmate)
    } else if board.is_stalemate() || board.is_draw() {
        None
    } else if (white_score - 0.0).abs() < f64::EPSILON || (white_score - 1.0).abs() < f64::EPSILON {
        Some(SfxKind::Forfeit)
    } else {
        None
    }
}

fn generate_tone_wav(base_hz: f64, overtone_hz: f64, duration_ms: usize, gain: f64) -> Vec<u8> {
    let sample_rate = 44_100u32;
    let sample_count = sample_rate as usize * duration_ms / 1_000;
    let mut samples = Vec::with_capacity(sample_count);
    for index in 0..sample_count {
        let time = index as f64 / f64::from(sample_rate);
        let envelope = (-time / (duration_ms as f64 / 4_000.0)).exp();
        let sample = (std::f64::consts::TAU * base_hz * time).sin()
            + 0.25 * (std::f64::consts::TAU * overtone_hz * time).sin();
        samples.push((sample * envelope * gain * 32_767.0) as i16);
    }
    encode_wav(&samples, sample_rate)
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

fn generate_game_focus() -> Vec<u8> {
    generate_drone_track(22.0, 98.0, 147.0, 196.0, 0.08)
}

fn generate_game_march() -> Vec<u8> {
    generate_pulse_track(20.0, 130.81, 196.0, 0.55)
}

fn generate_game_nocturne() -> Vec<u8> {
    generate_drone_track(26.0, 73.4, 110.0, 220.0, 0.06)
}

fn generate_drone_track(dur: f64, root: f64, fifth: f64, octave: f64, gain: f64) -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let vib = (std::f64::consts::TAU * 0.12 * t).sin() * 0.8;
        let drone = (std::f64::consts::TAU * (root + vib) * t).sin() * gain;
        let pad = (std::f64::consts::TAU * fifth * t).sin() * gain * 0.45;
        let bell_t = t % 6.0;
        let bell = (std::f64::consts::TAU * octave * t).sin() * (-bell_t / 1.8).exp() * gain * 0.5;
        let mut mix = drone + pad + bell;
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

fn generate_pulse_track(dur: f64, bass: f64, treble: f64, beat: f64) -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as f64 * dur) as usize;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let phase = (t * beat).fract();
        let env = (-phase * 5.0).exp();
        let low = (std::f64::consts::TAU * bass * t).sin() * 0.09 * env;
        let high = (std::f64::consts::TAU * treble * t).sin() * 0.04 * env;
        let mut mix = low + high;
        let fd = 0.6;
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
    fn wood_is_the_default_sound_theme() {
        assert_eq!(SoundTheme::default(), SoundTheme::Wood);
    }

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
    fn sound_themes_produce_distinct_allocation_free_playback_buffers() {
        let wood = generate_sound_theme(SoundTheme::Wood);
        let crystal = generate_sound_theme(SoundTheme::Crystal);
        let soft = generate_sound_theme(SoundTheme::Soft);

        assert_ne!(wood.0.as_ref(), crystal.0.as_ref());
        assert_ne!(crystal.0.as_ref(), soft.0.as_ref());
        assert!(wood.0.starts_with(b"RIFF"));
        assert!(soft.1.starts_with(b"RIFF"));
        for theme in SoundTheme::ALL {
            let (mv, cap) = generate_sound_theme(theme);
            assert!(mv.starts_with(b"RIFF"), "{theme}");
            assert!(cap.starts_with(b"RIFF"), "{theme}");
        }
    }

    #[test]
    fn extra_moods_emit_wav_headers() {
        assert!(generate_game_focus().starts_with(b"RIFF"));
        assert!(generate_game_march().starts_with(b"RIFF"));
        assert!(generate_game_nocturne().starts_with(b"RIFF"));
        assert_eq!(GameMood::ALL.len(), 6);
        assert_eq!(SoundTheme::ALL.len(), 7);
    }

    #[test]
    fn sfx_kind_prefers_check_then_specials() {
        let castle = types::Move::king_castle(types::Square::E1, types::Square::G1);
        assert_eq!(SfxKind::from_move(castle, false, false), SfxKind::Castle);
        assert_eq!(SfxKind::from_move(castle, false, true), SfxKind::Check);
        let capture = types::Move::capture(types::Square::E4, types::Square::D5);
        assert_eq!(SfxKind::from_move(capture, true, false), SfxKind::Capture);
        let quiet = types::Move::quiet(types::Square::E2, types::Square::E4);
        assert_eq!(SfxKind::from_move(quiet, false, false), SfxKind::Move);
    }

    #[test]
    fn outcome_motifs_are_distinct_wavs() {
        let check = generate_check_wav();
        let mate = generate_motif_wav(&[(392.0, 140), (311.1, 160), (246.9, 280)], 0.48);
        let resign = generate_motif_wav(&[(196.0, 180), (146.8, 320)], 0.42);
        let forfeit = generate_motif_wav(&[(164.8, 160), (130.8, 140), (98.0, 340)], 0.46);
        let over = generate_motif_wav(
            &[(523.3, 90), (659.3, 90), (783.9, 110), (1046.5, 280)],
            0.44,
        );
        for wav in [&check, &mate, &resign, &forfeit, &over] {
            assert!(wav.starts_with(b"RIFF"));
            assert!(wav.len() > 44);
        }
        assert_ne!(check, mate);
        assert_ne!(mate, resign);
        assert_ne!(resign, forfeit);
        assert_ne!(forfeit, over);
    }

    #[test]
    fn game_end_sfx_classifies_mate_and_forfeit() {
        types::init();
        let mut mate = types::Board::from_fen(
            "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4",
        )
        .expect("fen");
        assert!(mate.is_checkmate());
        assert_eq!(game_end_sfx(&mut mate, 1.0), Some(SfxKind::Checkmate));
        let mut start = types::Board::new();
        assert_eq!(game_end_sfx(&mut start, 1.0), Some(SfxKind::Forfeit));
        assert_eq!(game_end_sfx(&mut start, 0.5), None);
    }

    #[test]
    fn sfx_gate_skips_when_disabled() {
        assert_eq!(
            SfxKind::from_move(
                types::Move::quiet(types::Square::A2, types::Square::A3),
                false,
                false
            ),
            SfxKind::Move
        );
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
