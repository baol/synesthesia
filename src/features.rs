//! What we hear in the music, in the language of sound and music, and a
//! synthetic source for it.
//!
//! [`AudioFeatures`] comes from the microphone or a file (`analysis.rs`), or
//! from the synthetic source: an imaginary 120 BPM track with kicks on the
//! beat, hats on the off-beats, slowly moving mids and brightness, a held
//! melody note that changes every 4 bars, and a quiet breakdown every 16 bars.
//! Keys let you poke it by hand.
//!
//! These are raw readings. Smoothing and shaping happen later, in the
//! transforms (`tuning.rs`).

use bevy::prelude::*;

/// All values 0..1 unless noted.
#[derive(Resource, Debug, Clone, Default)]
pub struct AudioFeatures {
    /// How loud the music is now, against the loudest of the last minute or so.
    pub loudness: f32,
    /// The four frequency bands (20–150 Hz, 150–500 Hz, 500 Hz–2 kHz, 2–8 kHz),
    /// on one shared scale and weighted by how much each is changing, so a
    /// static background layer reads lower than a moving part.
    pub bass: f32,
    pub low_mids: f32,
    pub high_mids: f32,
    pub treble: f32,
    /// Spectral centroid on a log scale (200 Hz → 0, 5 kHz → 1).
    pub brightness: f32,
    /// Note onsets: jumps to the onset's strength and decays within ~60 ms.
    pub onset: f32,
    /// Onsets since the last frame.
    pub onsets: u32,
    /// Beats: jumps to the beat's strength and decays within ~60 ms.
    pub beat: f32,
    /// Beats since the last frame, and the strongest one's strength.
    pub beats: u32,
    pub beat_strength: f32,
    /// Tempo, 60 BPM → 0 … 180 BPM → 1.
    pub tempo: f32,
    /// How clear and steady the pulse is (0 = none, 1 = a clear beat).
    pub pulse_clarity: f32,
    /// Onsets per second (strength-weighted), 6/s → 1.
    pub note_density: f32,
    /// Where the bass sits: 0 ≈ 40 Hz, 1 ≈ 140 Hz.
    pub bass_pitch: f32,
    /// The strongest steady frequency (spectrum peak averaged over ~2 s):
    /// 0 = 30 Hz … 1 = 12 kHz.
    pub dominant_pitch: f32,
    /// Input level in dBFS (live sources only), for the overlay.
    pub level_db: f32,
    /// Spectrum in [`SPECTRUM_BINS`] log-spaced bands from 30 Hz to 12 kHz,
    /// each 0..1 on the same scale as the four bands above.
    pub spectrum: [f32; SPECTRUM_BINS],
}

/// Number of spectrum bands (about 1/4 octave each).
pub const SPECTRUM_BINS: usize = 32;
pub const SPECTRUM_LO_HZ: f32 = 30.0;
pub const SPECTRUM_HI_HZ: f32 = 12_000.0;

/// Edges of spectrum band `i` in Hz.
pub fn spectrum_edges(i: usize) -> (f32, f32) {
    let ratio = SPECTRUM_HI_HZ / SPECTRUM_LO_HZ;
    let at = |x: usize| SPECTRUM_LO_HZ * ratio.powf(x as f32 / SPECTRUM_BINS as f32);
    (at(i), at(i + 1))
}

/// Centre (geometric) of spectrum band `i` in Hz.
pub fn spectrum_center(i: usize) -> f32 {
    let (lo, hi) = spectrum_edges(i);
    (lo * hi).sqrt()
}

/// Envelopes of onsets and beats decay with this time constant (s).
pub const EVENT_DECAY: f32 = 0.06;

/// Map a tempo in BPM to the 0..1 `tempo` reading.
pub fn tempo01(bpm: f32) -> f32 {
    ((bpm - 60.0) / 120.0).clamp(0.0, 1.0)
}

impl AudioFeatures {
    pub fn bands(&self) -> [f32; 4] {
        [self.bass, self.low_mids, self.high_mids, self.treble]
    }

    /// Clear the per-frame event counts.
    pub fn clear_events(&mut self) {
        self.onsets = 0;
        self.beats = 0;
        self.beat_strength = 0.0;
    }
}

/// Where features come from.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSource {
    /// The microphone or a playing file (see `audio.rs`).
    Live,
    Synthetic,
    /// Everything at zero: shows the bare wind and turbulence.
    Silent,
}

/// Systems that produce `AudioFeatures`; consumers run after this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeatureSet;

pub struct FeaturesPlugin;

impl Plugin for FeaturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioFeatures>()
            .insert_resource(FeatureSource::Synthetic)
            .init_resource::<Synth>()
            .add_systems(Update, (keyboard, synthetic).chain().in_set(FeatureSet));
    }
}

/// One-pole envelope follower with separate attack and release times.
#[inline]
pub fn follow(current: f32, target: f32, dt: f32, attack: f32, release: f32) -> f32 {
    let tau = if target > current { attack } else { release };
    current + (target - current) * (1.0 - (-dt / tau.max(1e-4)).exp())
}

#[derive(Resource, Default)]
struct Synth {
    last_beat: i64,
    last_eighth: i64,
    kick: f32,
    hat: f32,
    /// Strength-weighted onset rate, decaying (for note density).
    density: f32,
    /// Manual pokes from the keyboard, per band, decaying.
    manual: [f32; 4],
    manual_onset: bool,
    manual_beat: bool,
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut synth: ResMut<Synth>,
    mut source: ResMut<FeatureSource>,
    live: Option<Res<crate::audio::LiveAudio>>,
) {
    // M cycles live → synthetic → silent → live (skipping live if there is none).
    if keys.just_pressed(KeyCode::KeyM) {
        *source = match *source {
            FeatureSource::Live => FeatureSource::Synthetic,
            FeatureSource::Synthetic => FeatureSource::Silent,
            FeatureSource::Silent if live.is_some() => FeatureSource::Live,
            FeatureSource::Silent => FeatureSource::Synthetic,
        };
        info!("feature source: {:?}", *source);
    }
    for (i, key) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4]
        .into_iter()
        .enumerate()
    {
        if keys.pressed(key) {
            synth.manual[i] = 1.0;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        synth.manual_onset = true;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        synth.manual_beat = true;
    }
}

fn synthetic(
    time: Res<Time>,
    cfg: Res<crate::config::Config>,
    source: Res<FeatureSource>,
    mut synth: ResMut<Synth>,
    mut f: ResMut<AudioFeatures>,
) {
    if *source == FeatureSource::Live {
        // Live frames come from the analysis thread; keyboard pokes don't apply.
        return;
    }
    let dt = time.delta_secs().clamp(1e-4, 0.1);
    let t = time.elapsed_secs() + cfg.synth_offset;
    // Slow changes (mids, brightness) can run faster for short review clips.
    let ts = t * cfg.synth_speed;
    f.clear_events();
    f.level_db = f32::NEG_INFINITY;
    let decay = (-dt / EVENT_DECAY).exp();
    f.onset *= decay;
    f.beat *= decay;

    let (mut loud, mut bands, mut centroid) = (0.0, [0.0f32; 4], 0.0);
    // A held melody note (spectrum band), changing every 4 bars.
    let mut lead: Option<(f32, f32)> = None;
    let onset = |f: &mut AudioFeatures, synth: &mut Synth, strength: f32| {
        f.onsets += 1;
        f.onset = f.onset.max(strength);
        synth.density += strength;
    };

    if *source == FeatureSource::Synthetic {
        let bpm = 120.0;
        let beats = t * bpm / 60.0;
        let bar = (beats / 4.0) as i64;
        // Bars 12–15 of every 16 are a breakdown: no kick, quieter.
        let breakdown = bar % 16 >= 12;
        f.tempo = tempo01(bpm);
        f.pulse_clarity = follow(f.pulse_clarity, if breakdown { 0.3 } else { 1.0 }, dt, 1.0, 2.0);

        let beat = beats.floor() as i64;
        if beat != synth.last_beat {
            synth.last_beat = beat;
            if !breakdown {
                synth.kick = 1.0;
                // Accent the downbeat of each bar.
                let strength = if beat % 4 == 0 { 1.0 } else { 0.6 };
                onset(&mut f, &mut synth, strength);
                f.beats += 1;
                f.beat_strength = f.beat_strength.max(strength * f.pulse_clarity);
                f.beat = f.beat.max(strength * f.pulse_clarity);
            }
        }
        // Hats on the off-beats (the eighths between kicks).
        let eighth = (beats * 2.0).floor() as i64;
        if eighth != synth.last_eighth {
            synth.last_eighth = eighth;
            if eighth % 2 == 1 {
                synth.hat = 0.8;
                onset(&mut f, &mut synth, 0.4);
            }
        }
        synth.kick *= (-dt / 0.18).exp();
        synth.hat *= (-dt / 0.06).exp();

        let section = if breakdown { 0.35 } else { 1.0 };
        let sway = |freq: f32, phase: f32| 0.5 + 0.5 * (ts * freq * std::f32::consts::TAU + phase).sin();
        bands[0] = section * (0.15 + 0.85 * synth.kick);
        bands[1] = section * (0.25 + 0.55 * sway(0.05, 0.0));
        bands[2] = 0.2 + 0.6 * sway(0.11, 1.3) * sway(0.023, 0.4);
        bands[3] = section * (0.1 + 0.6 * synth.hat + 0.2 * sway(0.31, 2.0));
        centroid = 0.3 + 0.5 * sway(0.017, 0.9);
        // A bass line: a different note every bar (in 40–140 Hz terms).
        const NOTES: [f32; 4] = [0.2, 0.2, 0.55, 0.35];
        f.bass_pitch = follow(f.bass_pitch, NOTES[(bar % 4) as usize], dt, 0.05, 0.05);
        loud = section * (0.55 + 0.25 * synth.kick + 0.2 * sway(0.03, 0.0));
        const LEAD: [f32; 4] = [6.0, 15.0, 10.0, 21.0];
        let lead_bin = LEAD[((bar / 4) % 4) as usize];
        lead = Some((lead_bin, 0.9 * section.max(0.6)));
        f.dominant_pitch = lead_bin / (SPECTRUM_BINS - 1) as f32;
    }

    // Manual pokes add on top of whatever the source says.
    for (b, m) in bands.iter_mut().zip(synth.manual.iter_mut()) {
        *b = b.max(*m);
        *m *= (-dt / 0.4).exp();
    }
    if std::mem::take(&mut synth.manual_onset) {
        onset(&mut f, &mut synth, 1.0);
    }
    if std::mem::take(&mut synth.manual_beat) {
        f.beats += 1;
        f.beat_strength = 1.0;
        f.beat = 1.0;
    }

    // Light smoothing, as the analyser does.
    synth.density *= (-dt / 2.0).exp();
    f.note_density = (synth.density / 2.0 / 6.0).min(1.0);
    f.loudness = follow(f.loudness, loud, dt, 0.02, 0.3);
    f.bass = follow(f.bass, bands[0], dt, 0.01, 0.25);
    f.low_mids = follow(f.low_mids, bands[1], dt, 0.05, 0.4);
    f.high_mids = follow(f.high_mids, bands[2], dt, 0.05, 0.4);
    f.treble = follow(f.treble, bands[3], dt, 0.01, 0.2);
    f.brightness = follow(f.brightness, centroid, dt, 0.1, 0.3);

    // A spectrum to match: the four band levels spread over their frequencies
    // (interpolated on a log scale), with some slowly moving texture.
    let centres = [55f32, 275.0, 1000.0, 4000.0].map(f32::log2);
    let levels = f.bands();
    for (i, v) in f.spectrum.iter_mut().enumerate() {
        let lf = spectrum_center(i).log2();
        let j = centres.iter().rposition(|&c| c <= lf).unwrap_or(0).min(2);
        let a = ((lf - centres[j]) / (centres[j + 1] - centres[j])).clamp(0.0, 1.0);
        let level = levels[j] * (1.0 - a) + levels[j + 1] * a;
        let texture = 0.8 + 0.2 * (i as f32 * 1.7 + t * (0.4 + 0.05 * i as f32)).sin();
        let note = lead.map_or(0.0, |(bin, height)| height * (-((i as f32 - bin) / 1.2).powi(2)).exp());
        *v = (level * texture).max(note).clamp(0.0, 1.0);
    }
}
