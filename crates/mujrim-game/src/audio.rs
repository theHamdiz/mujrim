use bevy::prelude::*;

/// Sound effect messages.
#[derive(Message, Clone, Copy, Debug)]
pub enum SoundMessage {
    Move,
    Capture,
    Check,
    Castle,
    GameOver,
    Click,
}

/// Holds loaded audio asset handles.
#[derive(Resource)]
pub struct AudioAssets {
    pub move_sound: Handle<AudioSource>,
    pub capture_sound: Handle<AudioSource>,
    pub check_sound: Handle<AudioSource>,
    pub click_sound: Handle<AudioSource>,
    pub bgm: Handle<AudioSource>,
    pub bgm_playing: bool,
}

/// Generate a procedural WAV buffer.
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
    let n = (sr as usize * 120) / 1000;
    let mut s = Vec::with_capacity(n);
    let mut rng: u32 = 12345;
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let tone = (2.0 * std::f64::consts::PI * 600.0 * t).sin();
        let tone2 = (2.0 * std::f64::consts::PI * 1200.0 * t).sin() * 0.4;
        let noise = if t < 0.015 {
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

fn generate_check_wav() -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as usize * 200) / 1000;
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let freq = 800.0 + 400.0 * (t * 30.0).sin();
        let tone = (2.0 * std::f64::consts::PI * freq * t).sin();
        let env = (-t / 0.05).exp();
        s.push((tone * env * 0.5 * 32767.0) as i16);
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

/// Load/generate audio assets at startup.
pub fn load_audio_assets(
    mut commands: Commands,
    mut audio_sources: ResMut<Assets<AudioSource>>,
    asset_server: Res<AssetServer>,
) {
    let move_wav = generate_move_wav();
    let capture_wav = generate_capture_wav();
    let check_wav = generate_check_wav();
    let click_wav = generate_click_wav();

    let move_handle = audio_sources.add(AudioSource {
        bytes: move_wav.into(),
    });
    let capture_handle = audio_sources.add(AudioSource {
        bytes: capture_wav.into(),
    });
    let check_handle = audio_sources.add(AudioSource {
        bytes: check_wav.into(),
    });
    let click_handle = audio_sources.add(AudioSource {
        bytes: click_wav.into(),
    });

    let bgm = asset_server.load("embedded://music/Caves.mp3");

    commands.insert_resource(AudioAssets {
        move_sound: move_handle,
        capture_sound: capture_handle,
        check_sound: check_handle,
        click_sound: click_handle,
        bgm,
        bgm_playing: false,
    });
}

/// Play the background music on loop when in-game.
pub fn play_background_music(mut commands: Commands, audio_assets: Option<ResMut<AudioAssets>>) {
    let Some(mut audio_assets) = audio_assets else {
        return;
    };
    if !audio_assets.bgm_playing {
        commands.spawn((
            AudioPlayer::new(audio_assets.bgm.clone()),
            PlaybackSettings {
                mode: bevy::audio::PlaybackMode::Loop,
                volume: bevy::audio::Volume::Linear(0.15),
                ..default()
            },
        ));
        audio_assets.bgm_playing = true;
    }
}

/// System that reacts to SoundMessages by spawning audio playback entities.
pub fn play_sound_effects(
    mut commands: Commands,
    audio_assets: Option<Res<AudioAssets>>,
    mut messages: MessageReader<SoundMessage>,
) {
    let Some(assets) = audio_assets else { return };

    for msg in messages.read() {
        let handle = match msg {
            SoundMessage::Move => assets.move_sound.clone(),
            SoundMessage::Capture => assets.capture_sound.clone(),
            SoundMessage::Check => assets.check_sound.clone(),
            SoundMessage::Castle => assets.move_sound.clone(),
            SoundMessage::GameOver => assets.check_sound.clone(),
            SoundMessage::Click => assets.click_sound.clone(),
        };
        commands.spawn((
            AudioPlayer::new(handle),
            PlaybackSettings {
                mode: bevy::audio::PlaybackMode::Despawn,
                ..default()
            },
        ));
    }
}

fn generate_click_wav() -> Vec<u8> {
    let sr = 44100u32;
    let n = (sr as usize * 30) / 1000; // 30ms
    let mut s = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let tone = (2.0 * std::f64::consts::PI * 1800.0 * t).sin();
        let env = (-t / 0.005).exp();
        s.push((tone * env * 0.35 * 32767.0) as i16);
    }
    encode_wav(&s, sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_wav_valid_header() {
        let wav = generate_move_wav();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_capture_wav_valid_header() {
        let wav = generate_capture_wav();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
    }

    #[test]
    fn test_check_wav_valid_header() {
        let wav = generate_check_wav();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
    }
}
