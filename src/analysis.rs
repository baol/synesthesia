//! Turns a stereo (or mono) sample stream into [`AudioFeatures`], at one
//! frame per hop (512 samples ≈ 10.7 ms at 48 kHz). Pure and thread-agnostic:
//! the audio thread pushes samples in, frames come out.
//!
//! Per hop: a 2048-sample Hann-windowed FFT of each channel. Their average is
//! the mono spectrum, which gives
//! - loudness (RMS, dBFS),
//! - energy in four bands and 32 spectrum bands (dB),
//! - spectral centroid (Hz),
//! - spectral flux on log magnitudes: onsets, and the beat tracker's input,
//! - where the bass sits in pitch, and the dominant steady pitch,
//! - the lead line's pitch (`melody.rs`), found in the *lead spectrum*: the
//!   centre of the stereo image (where vocals are almost always panned), with
//!   percussion removed (harmonic / percussive median filtering, causal) and
//!   the bass damped,
//! - whether someone is singing, from the system's sound classifier
//!   (`voice_detect.rs`, macOS only),
//! - the harmony (`harmony.rs`, on its own longer window): the twelve pitch
//!   classes, the chord and its changes, major or minor, the key.
//!
//! Levels are scaled to 0..1 against the loudest of the last minute or so (a
//! common scale, so sections keep their contrast and quiet layers stay quiet).
//! Bands are also weighted by how much each one is changing: what moves is
//! what we hear as the foreground. Steady sounds (hum, fans) fade out.

use std::{collections::VecDeque, sync::Arc};

use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};

use crate::features::{AudioFeatures, SPECTRUM_BINS, follow, spectrum_center, spectrum_edges};

pub const FFT_SIZE: usize = 2048;
pub const HOP: usize = 512;

/// Band edges in Hz: low, low-mid, high-mid, high.
pub const BANDS: [(f32, f32); 4] = [(20.0, 150.0), (150.0, 500.0), (500.0, 2000.0), (2000.0, 8000.0)];

/// Per-band offsets (dB) before the shared gain: divide by bandwidth in octaves
/// (so equal energy per octave reads equal), then tilt +1.5 dB/octave upwards,
/// because music gets quieter towards the treble. Without this the low band
/// would win almost always.
const BAND_TILT_DB: [f32; 4] = [-4.6, 1.1, 3.3, 6.3];

/// dB below the shared band ceiling that maps to 0.
const BAND_RANGE_DB: f32 = 24.0;
/// The spectrum display keeps a wider range, to show more of the music.
const SPECTRUM_RANGE_DB: f32 = 30.0;

/// Below this level (dBFS) the input counts as silence and everything relaxes to 0.
const GATE_DB: f32 = -62.0;
/// With the loudest band under this to the ear (`EarOut::now`: about where
/// the loudness reads 0; hiss at -45 dBFS after a mastered song reads 0.15)
/// there is nothing to hear either, whatever the meter says: unless
/// something was struck in the last `STRUCK` seconds (a soft kick and a bass
/// far under the rest of the song are no louder than hiss, but hiss has no
/// notes).
const QUIET: f32 = 0.2;
const STRUCK: f32 = 1.0;
/// A silence this long (s) is a gap in the music: everything that describes
/// the music (its tempo, its pulse, its tension, where its pitch sits) holds.
/// From here to `NEW_SONG` it lets go.
const GAP: f32 = 1.0;
/// After this long (s) the song is over: what was learned about it (tempo,
/// the scale of its bands) is forgotten, and the next sound starts afresh.
const NEW_SONG: f32 = 3.0;
/// Where a position (brightness, a pitch) rests when there is no music: the
/// middle, which is neutral for the controls that follow it.
const REST: f32 = 0.5;

/// Unnormalised measurements of one hop (used by the tests).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Raw {
    pub level_db: f32,
    pub bands_db: [f32; 4],
    pub centroid_hz: f32,
    pub flux: f32,
    pub onset: bool,
    pub beat: bool,
    /// Strength (0..1) of this hop's onset / beat, 0 if none.
    pub onset_strength: f32,
    pub beat_strength: f32,
    pub tempo_bpm: f32,
    /// Spectrum bands in dB (with the treble tilt), before any normalisation.
    pub spectrum_db: [f32; SPECTRUM_BINS],
    /// The lead pitch this hop (Hz), 0 if none.
    pub melody_hz: f32,
    /// Spectral flatness of 500 Hz–8 kHz (0 = tonal peaks … 1 = noise):
    /// distorted guitars and cymbal wash read high.
    pub flatness: f32,
    /// The mosh pit's three parts (loud, beat, heavy), 0..1 each (diagnostics).
    pub mosh_parts: [f32; 3],
    /// Drive now and over the last seconds (diagnostics).
    pub drive: [f32; 2],
    /// The classifier's latest (singing, rapping), unsmoothed.
    pub singing: f32,
    pub rapping: f32,
    /// The lead spectrum in dB, `LEAD_ROWS` log-spaced rows from 80 Hz to 4 kHz
    /// (for the timeline).
    pub lead_db: [f32; LEAD_ROWS],
    /// Harmony: the chord and the key (0–11 major from C, 12–23 minor; −1 =
    /// none), this hop's chord change (its strength, 0 if none), and how far
    /// the tonal centroid is moving.
    pub chord: i8,
    pub key: i8,
    pub chord_change: f32,
    pub harmony_movement: f32,
    /// Tension's parts: the harmony's function, dissonance and suspense, and
    /// the build-up (0..1 each).
    pub tension_parts: [f32; 4],
    /// How well the key fits (correlation; diagnostics).
    pub key_fit: f32,
    /// Energy's parts: loudness, fullness, drive, heaviness and the lift (was:
    /// song's range, and the absolute energy (0..1 each).
    pub energy_parts: [f32; 5],
    /// The layers that make fullness (beat, bass, body, top, lead), and
    /// tension's holding back.
    pub layers: [f32; 5],
    pub hold: f32,
}

pub const LEAD_ROWS: usize = 96;
pub const LEAD_LO_HZ: f32 = 80.0;
pub const LEAD_HI_HZ: f32 = 4000.0;

impl Default for Raw {
    fn default() -> Self {
        Self {
            level_db: 0.0,
            bands_db: [0.0; 4],
            centroid_hz: 0.0,
            flux: 0.0,
            onset: false,
            beat: false,
            onset_strength: 0.0,
            beat_strength: 0.0,
            tempo_bpm: 0.0,
            spectrum_db: [0.0; SPECTRUM_BINS],
            melody_hz: 0.0,
            flatness: 0.0,
            mosh_parts: [0.0; 3],
            drive: [0.0; 2],
            singing: 0.0,
            rapping: 0.0,
            lead_db: [-120.0; LEAD_ROWS],
            chord: -1,
            key: -1,
            chord_change: 0.0,
            harmony_movement: 0.0,
            tension_parts: [0.0; 4],
            key_fit: 0.0,
            energy_parts: [0.0; 5],
            layers: [0.0; 5],
            hold: 0.0,
        }
    }
}

/// Maps a dB signal to 0..1 on a slowly adapting scale.
///
/// The ceiling follows peaks (fast up, ~1 min down); the bottom of the scale
/// follows the dips (fast down, ~1 min up), so a whole song shares one scale
/// and its sections keep their contrast. The range stays between `min_range`
/// and `max_range` dB.
///
/// Separately, a short-term floor (~10 s) measures whether the signal is
/// moving at all: a steady sound (fan, hum, a held tone) fades to 0.
#[derive(Debug, Clone, Copy)]
struct AutoGain {
    ceil: f32,
    floor: f32,
    short_ceil: f32,
    short_floor: f32,
    init: bool,
    min_range: f32,
    max_range: f32,
    /// 0 for a steady signal, 1 once it moves by at least `STEADY_DB`.
    presence: f32,
}

/// Below this much movement (dB, over ~10 s) the input counts as steady.
const STEADY_DB: f32 = 6.0;
/// How slowly the scale forgets a loud passage (s).
const SCALE_MEMORY: f32 = 60.0;

impl AutoGain {
    fn new(min_range: f32, max_range: f32) -> Self {
        Self { ceil: 0.0, floor: 0.0, short_ceil: 0.0, short_floor: 0.0, init: false, min_range, max_range, presence: 1.0 }
    }

    /// Track `x` (dB) without producing a reading (for shared scales).
    fn track(&mut self, x: f32, dt: f32) {
        if !self.init {
            self.init = true;
            self.ceil = x + self.min_range * 0.5;
            self.floor = x - self.min_range * 0.5;
            self.short_ceil = self.ceil;
            self.short_floor = self.floor;
        }
        let toward = |v: f32, up: f32, down: f32| if x > v { follow(v, x, dt, up, up) } else { follow(v, x, dt, down, down) };
        self.ceil = toward(self.ceil, 0.05, SCALE_MEMORY);
        self.floor = toward(self.floor, SCALE_MEMORY, 1.0);
        self.short_ceil = toward(self.short_ceil, 0.05, 10.0);
        self.short_floor = toward(self.short_floor, 10.0, 1.0);
        self.presence = ((self.short_ceil - self.short_floor) / STEADY_DB).clamp(0.0, 1.0);
    }

    fn update(&mut self, x: f32, dt: f32) -> f32 {
        self.track(x, dt);
        let range = (self.ceil - self.floor).clamp(self.min_range, self.max_range);
        ((x - (self.ceil - range)) / range).clamp(0.0, 1.0) * self.presence
    }
}

/// Adaptive-threshold onset picker on a flux signal: fires when flux rises
/// above 1.5 × its recent mean plus a fraction of its recent peak, then waits
/// out a refractory time.
#[derive(Debug, Clone)]
struct OnsetDetector {
    history: VecDeque<f32>,
    peak: f32,
    refractory: f32,
    min_interval: f32,
}

impl OnsetDetector {
    fn new(min_interval: f32) -> Self {
        Self { history: VecDeque::with_capacity(32), peak: 1e-3, refractory: 0.0, min_interval }
    }

    /// `Some(strength)` on an onset, strength 0.3..1 from how far flux overshot
    /// the threshold (a soft tap ≈ 0.3, a hard kick ≈ 1).
    fn detect(&mut self, flux: f32, dt: f32, gated: bool) -> Option<f32> {
        let mean = if self.history.is_empty() {
            flux
        } else {
            self.history.iter().sum::<f32>() / self.history.len() as f32
        };
        self.peak = (self.peak * (-dt / 5.0).exp()).max(flux).max(1e-3);
        let threshold = 1.5 * mean + 0.2 * self.peak;
        self.refractory = (self.refractory - dt).max(0.0);
        let onset = !gated && self.refractory == 0.0 && flux > threshold;
        if onset {
            self.refractory = self.min_interval;
        }
        if self.history.len() == 24 {
            self.history.pop_front();
        }
        self.history.push_back(flux);
        onset.then(|| (0.3 + 0.35 * (flux / threshold.max(1e-9)).ln()).clamp(0.3, 1.0))
    }
}

pub struct Analyzer {
    sample_rate: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// Left and right samples waiting (equal lengths).
    input_l: Vec<f32>,
    input_r: Vec<f32>,
    frame: Vec<f32>,
    /// Mono spectrum (average of the two channels).
    spectrum: Vec<Complex<f32>>,
    spectrum_r: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    lead: LeadSpectrum,
    prev_log_mag: Vec<f32>,
    onsets: OnsetDetector,
    beat: crate::beat::BeatTracker,
    /// Strength-weighted onset count, decaying over ~2 s (note density).
    density: f32,
    /// Spectrum averaged over ~2 s (dominant pitch).
    slow_spectrum: [f32; SPECTRUM_BINS],
    /// How much each band is changing (its flux, smoothed).
    motion: [f32; 4],
    gain_level: AutoGain,
    gain_bands: [AutoGain; 4],
    /// One ceiling shared by all bands, so they stay comparable: a band reads
    /// high only if it is loud relative to the others.
    band_ceiling: Option<f32>,
    /// Power per FFT bin of the current hop.
    power: Vec<f32>,
    gain_spectrum: [AutoGain; SPECTRUM_BINS],
    spectrum_ceiling: Option<f32>,
    melody: crate::melody::MelodyTracker,
    /// The sound classifier (macOS only), and the hop's mono samples for it.
    voice: Option<crate::voice_detect::VoiceDetector>,
    hop_mono: Vec<f32>,
    harmony: crate::harmony::Harmony,
    energy: crate::energy::Energy,
    /// What the ear makes of the spectrum (loudness, sharpness, impact).
    pub ear: crate::ear::Ear,
    aspects: crate::aspects::Aspects,
    /// The level against the loudest of the last minute (drops and the pit).
    rel_loud: f32,
    /// How long (s) there has been no voice.
    voiceless: f32,
    /// Drive (loudness, low end, activity) now and over the last seconds,
    /// time until another drop may fire, and the smoothed flatness.
    drive_short: f32,
    drive_long: f32,
    drop_cooldown: f32,
    /// Flatness of the last ~2 s, and its median.
    flat_history: VecDeque<f32>,
    flat_tick: usize,
    flatness: f32,
    /// How long (s) the mosh pit has been off.
    pit_off: f32,
    /// How long it has been silent (s), and whether the song was forgotten.
    silent: f32,
    forgotten: bool,
    since_onset: f32,
    features: AudioFeatures,
    pub raw: Raw,
}

/// Spectrum bands are tilted by this much per octave (relative to 1 kHz), as
/// the four bands are: music gets quieter towards the treble.
const SPECTRUM_TILT_DB_PER_OCTAVE: f32 = 1.5;

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let spectrum = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        Self {
            sample_rate,
            window,
            input_l: Vec::with_capacity(FFT_SIZE * 2),
            input_r: Vec::with_capacity(FFT_SIZE * 2),
            frame: vec![0.0; FFT_SIZE],
            prev_log_mag: vec![0.0; spectrum.len()],
            lead: LeadSpectrum::new(spectrum.len()),
            spectrum_r: spectrum.clone(),
            spectrum,
            scratch,
            fft,
            onsets: OnsetDetector::new(0.1),
            beat: crate::beat::BeatTracker::new(HOP as f32 / sample_rate),
            density: 0.0,
            slow_spectrum: [0.0; SPECTRUM_BINS],
            motion: [0.0; 4],
            gain_level: AutoGain::new(18.0, 30.0),
            gain_bands: [AutoGain::new(18.0, 30.0); 4],
            band_ceiling: None,
            power: vec![0.0; FFT_SIZE / 2 + 1],
            gain_spectrum: [AutoGain::new(18.0, 30.0); SPECTRUM_BINS],
            spectrum_ceiling: None,
            melody: crate::melody::MelodyTracker::new(FFT_SIZE / 2 + 1),
            ear: crate::ear::Ear::new(sample_rate, FFT_SIZE),
            aspects: crate::aspects::Aspects::default(),
            rel_loud: 0.0,
            voice: crate::voice_detect::VoiceDetector::new(sample_rate),
            hop_mono: vec![0.0; HOP],
            harmony: crate::harmony::Harmony::new(sample_rate),
            energy: crate::energy::Energy::default(),
            voiceless: 0.0,
            drive_short: 0.0,
            drive_long: f32::NAN,
            // Not in the first seconds: intros build up while the memory fills.
            drop_cooldown: 2.0 * DRIVE_MEMORY,
            flat_history: VecDeque::with_capacity(FLAT_HOPS),
            flat_tick: 0,
            flatness: 0.0,
            pit_off: 0.0,
            silent: NEW_SONG,
            forgotten: true,
            since_onset: f32::MAX,
            // No music yet: the positions rest in the middle.
            features: AudioFeatures { brightness: REST, bass_pitch: REST, dominant_pitch: REST, melody_pitch: REST, major: REST, ..AudioFeatures::default() },
            raw: Raw::default(),
        }
    }

    /// Power of the centre (lead) spectrum and of everything, 300 Hz–3 kHz.
    #[cfg(test)]
    pub fn centre_and_all(&self) -> (f32, f32) {
        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        let (k0, k1) = ((300.0 / bin_hz) as usize, (3000.0 / bin_hz) as usize);
        (self.lead.power[k0..k1].iter().sum(), self.power[k0..k1].iter().sum())
    }

    /// Feed mono samples; `on_frame` is called once per completed hop.
    #[cfg(test)]
    pub fn push(&mut self, samples: &[f32], on_frame: impl FnMut(&AudioFeatures, &Raw)) {
        self.input_l.extend_from_slice(samples);
        self.input_r.extend_from_slice(samples);
        self.run(on_frame);
    }

    /// Feed interleaved stereo samples (left, right, left, …).
    pub fn push_stereo(&mut self, samples: &[f32], on_frame: impl FnMut(&AudioFeatures, &Raw)) {
        for pair in samples.chunks_exact(2) {
            self.input_l.push(pair[0]);
            self.input_r.push(pair[1]);
        }
        self.run(on_frame);
    }

    fn run(&mut self, mut on_frame: impl FnMut(&AudioFeatures, &Raw)) {
        while self.input_l.len() >= FFT_SIZE {
            self.analyse();
            on_frame(&self.features, &self.raw);
            self.features.clear_events();
            self.input_l.drain(..HOP);
            self.input_r.drain(..HOP);
        }
    }

    fn analyse(&mut self) {
        let dt = HOP as f32 / self.sample_rate;
        let bin_hz = self.sample_rate / FFT_SIZE as f32;

        // Level of the window (time-domain RMS of the mono mix).
        let mut sum_sq = 0.0;
        for i in 0..FFT_SIZE {
            let x = 0.5 * (self.input_l[i] + self.input_r[i]);
            sum_sq += x * x;
        }
        let rms = (sum_sq / FFT_SIZE as f32).sqrt();
        let level_db = 20.0 * (rms + 1e-9).log10();

        // Each channel's spectrum; the mono one is their average (the FFT is linear).
        for (o, (&x, &w)) in self.frame.iter_mut().zip(self.input_r[..FFT_SIZE].iter().zip(&self.window)) {
            *o = x * w;
        }
        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum_r, &mut self.scratch)
            .expect("fft buffer sizes are fixed");
        for (o, (&x, &w)) in self.frame.iter_mut().zip(self.input_l[..FFT_SIZE].iter().zip(&self.window)) {
            *o = x * w;
        }
        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .expect("fft buffer sizes are fixed");
        let norm = 4.0 / FFT_SIZE as f32;
        self.lead.update(&self.spectrum, &self.spectrum_r, norm, bin_hz);
        for (m, r) in self.spectrum.iter_mut().zip(&self.spectrum_r) {
            *m = (*m + *r) * 0.5;
        }

        // Magnitudes, scaled so a full-scale sine reads about 1.
        let mut band_power = [0.0f32; 4];
        let (mut c_num, mut c_den) = (0.0f32, 0.0f32);
        let mut flux = 0.0f32;
        let mut flux_bins = 0usize;
        let mut bass_flux = 0.0f32;
        let mut bass_bins = 0usize;
        let mut band_flux = [0.0f32; 4];
        let mut band_bins = [0usize; 4];
        let (mut bp_num, mut bp_den) = (0.0f32, 0.0f32);
        for (k, c) in self.spectrum.iter().enumerate() {
            let f = k as f32 * bin_hz;
            let mag = c.norm() * norm;
            let p = mag * mag;
            self.power[k] = p;
            for (b, &(lo, hi)) in BANDS.iter().enumerate() {
                if f >= lo && f < hi {
                    band_power[b] += p;
                }
            }
            if (30.0..160.0).contains(&f) {
                bp_num += f.log2() * p;
                bp_den += p;
            }
            if (20.0..10_000.0).contains(&f) {
                c_num += f * p;
                c_den += p;
                // Log-compressed magnitude: onsets show up as positive jumps.
                let lm = (1.0 + 1000.0 * mag).ln();
                let rise = (lm - self.prev_log_mag[k]).max(0.0);
                flux += rise;
                flux_bins += 1;
                if let Some(b) = BANDS.iter().position(|&(lo, hi)| f >= lo && f < hi) {
                    band_flux[b] += rise;
                    band_bins[b] += 1;
                }
                if f < 160.0 {
                    bass_flux += rise;
                    bass_bins += 1;
                }
                self.prev_log_mag[k] = lm;
            }
        }
        flux /= flux_bins.max(1) as f32;
        bass_flux /= bass_bins.max(1) as f32;
        // Where the bass sits: power-weighted log frequency, 40 Hz → 0, 140 Hz → 1.
        let bass_pitch = if bp_den > 1e-12 {
            ((bp_num / bp_den - 40f32.log2()) / (140f32.log2() - 40f32.log2())).clamp(0.0, 1.0)
        } else {
            self.features.bass_pitch
        };
        let bands_db = band_power.map(|p| 10.0 * (p + 1e-12).log10());
        let centroid_hz = if c_den > 1e-12 { c_num / c_den } else { 0.0 };

        // Silence: to the meter, or to the ear (a faded-out tail, hiss). The
        // ear itself only stops for the meter's: it has to keep adapting to a
        // microphone far from the music.
        self.ear.push(&self.power, dt, level_db < GATE_DB);
        let faint = self.ear.out.now < QUIET;
        let onset_strength = self.onsets.detect(flux, dt, level_db < GATE_DB);
        self.since_onset = if onset_strength.is_some() { 0.0 } else { self.since_onset + dt };
        let gated = level_db < GATE_DB || (faint && self.since_onset > STRUCK);
        let onset_strength = onset_strength.filter(|_| !gated);
        self.silent = if gated { self.silent + dt } else { 0.0 };
        let presence = 1.0 - smoothstep(GAP, NEW_SONG, self.silent);
        if self.silent >= NEW_SONG && !self.forgotten {
            self.forget_the_song();
        }
        self.forgotten &= gated;
        let onset = onset_strength.is_some();
        // The beat tracker listens to all onsets, with the bass counted twice
        // (kicks and bass notes carry most grooves).
        let beat_strength = self.beat.process(if gated { 0.0 } else { flux + bass_flux });
        let beat = beat_strength.is_some();

        self.raw = Raw {
            level_db,
            bands_db,
            centroid_hz,
            flux,
            onset,
            beat,
            onset_strength: onset_strength.unwrap_or(0.0),
            beat_strength: beat_strength.unwrap_or(0.0),
            tempo_bpm: self.beat.tempo_bpm(),
            spectrum_db: self.raw.spectrum_db,
            melody_hz: 0.0,
            flatness: {
                let (k0, k1) = ((500.0 / bin_hz) as usize, ((8000.0 / bin_hz) as usize).min(self.power.len()));
                let n = (k1 - k0).max(1) as f32;
                let mean = self.power[k0..k1].iter().sum::<f32>() / n;
                let log_mean = self.power[k0..k1].iter().map(|p| (p + 1e-20).ln()).sum::<f32>() / n;
                if mean > 1e-15 { (log_mean.exp() / mean).clamp(0.0, 1.0) } else { 0.0 }
            },
            mosh_parts: [0.0; 3],
            drive: [0.0; 2],
            singing: 0.0,
            rapping: 0.0,
            lead_db: std::array::from_fn(|i| {
                let ratio = LEAD_HI_HZ / LEAD_LO_HZ;
                let (lo, hi) = (LEAD_LO_HZ * ratio.powf(i as f32 / LEAD_ROWS as f32), LEAD_LO_HZ * ratio.powf((i + 1) as f32 / LEAD_ROWS as f32));
                let (k0, k1) = ((lo / bin_hz).floor() as usize, ((hi / bin_hz).ceil() as usize).max((lo / bin_hz).floor() as usize + 1));
                let p = self.lead.power[k0..k1.min(self.lead.power.len())].iter().copied().fold(0.0, f32::max);
                10.0 * (p + 1e-12).log10()
            }),
            chord: -1,
            key: -1,
            chord_change: 0.0,
            harmony_movement: 0.0,
            tension_parts: [0.0; 4],
            key_fit: 0.0,
            energy_parts: [0.0; 5],
            layers: [0.0; 5],
            hold: 0.0,
        };
        // The lead line, in the lead spectrum.
        let line = self.melody.frame(&self.lead.power, bin_hz);
        self.melody.update(line, dt, gated);
        self.raw.melody_hz = line.hz.filter(|_| !gated).unwrap_or(0.0);
        // This hop's new samples, mono, for the harmony and the classifier.
        for (m, (&l, &r)) in self.hop_mono.iter_mut().zip(self.input_l[FFT_SIZE - HOP..FFT_SIZE].iter().zip(&self.input_r[FFT_SIZE - HOP..FFT_SIZE])) {
            *m = 0.5 * (l + r);
        }
        let chord_change = self.harmony.push(&self.hop_mono, dt, gated);
        let harmony = self.harmony.out;
        self.raw.chord = harmony.chord.map_or(-1, |c| c as i8);
        self.raw.key = harmony.key.map_or(-1, |k| k as i8);
        self.raw.chord_change = chord_change.unwrap_or(0.0);
        self.raw.harmony_movement = harmony.movement;
        // Is someone singing?
        if let Some(v) = &mut self.voice {
            v.push(&self.hop_mono);
            (self.raw.singing, self.raw.rapping) = v.voices();
        }

        // Normalise, then smooth with the same envelopes as the synthetic source.
        let (loud, bands, centroid) = if gated {
            (0.0, [0.0; 4], self.features.brightness)
        } else {
            let loud = self.gain_level.update(level_db, dt);
            let tilted: [f32; 4] = std::array::from_fn(|b| bands_db[b] + BAND_TILT_DB[b]);
            let top = tilted.iter().copied().fold(f32::MIN, f32::max);
            let ceil = self.band_ceiling.get_or_insert(top);
            *ceil = if top > *ceil { follow(*ceil, top, dt, 0.05, 0.05) } else { follow(*ceil, top, dt, SCALE_MEMORY, SCALE_MEMORY) };
            let lo = *ceil - BAND_RANGE_DB;
            // How much each band moves, relative to the busiest band: a static
            // layer keeps a third of its level, a moving part all of it.
            for b in 0..4 {
                let fb = band_flux[b] / band_bins[b].max(1) as f32;
                self.motion[b] = follow(self.motion[b], fb, dt, 0.05, 0.5);
            }
            let busiest = self.motion.iter().copied().fold(1e-9, f32::max);
            let mut bands = [0.0; 4];
            for b in 0..4 {
                // Each band still tracks its own floor, only so steady sounds fade.
                self.gain_bands[b].track(tilted[b], dt);
                let presence = self.gain_bands[b].presence;
                let level = ((tilted[b] - lo) / BAND_RANGE_DB).clamp(0.0, 1.0) * presence;
                bands[b] = level * (0.35 + 0.65 * self.motion[b] / busiest);
            }
            // Centroid on a log scale: 200 Hz → 0, 5 kHz → 1.
            let c = ((centroid_hz.max(1.0).log2() - 200f32.log2()) / (5000f32.log2() - 200f32.log2())).clamp(0.0, 1.0);
            // (Too faint to say where its weight sits: keep the last reading.)
            (loud, bands, if faint { self.features.brightness } else { c })
        };

        let f = &mut self.features;
        f.level_db = level_db;
        f.melody_pitch = self.melody.pitch;
        f.chord_change = (f.chord_change * (-dt / crate::features::CHORD_DECAY).exp()).max(chord_change.unwrap_or(0.0));
        f.major = REST + (harmony.major - REST) * presence;
        f.tonality = harmony.tonality * presence;
        // Energy, and tension: the harmony's, and on top of it the build-up.
        self.energy.push(band_power, flux, centroid_hz, self.beat.clarity, dt, gated);
        let energy = self.energy.out;
        let tension = harmony.tension + (1.0 - harmony.tension) * energy.build;
        self.raw.key_fit = harmony.key_fit;
        self.raw.tension_parts = [harmony.function, harmony.dissonance, harmony.suspense, energy.build];
        f.harmony_hue = harmony.hue;
        f.chroma = harmony.chroma.map(|c| c * presence);
        let (singing, rapping) = if gated { (0.0, 0.0) } else { (self.raw.singing, self.raw.rapping) };
        // A rap entry: rap rising past half after a while without any voice.
        f.rap_entry *= (-dt / crate::features::RAP_ENTRY_DECAY).exp();
        if f.rapping < 0.5 && follow(f.rapping, rapping, dt, 0.3, 1.5) >= 0.5 && self.voiceless >= crate::features::RAP_ENTRY_QUIET {
            f.rap_entry = rapping.max(f.rap_entry);
        }
        f.singing = follow(f.singing, singing, dt, 0.3, 1.5);
        f.rapping = follow(f.rapping, rapping, dt, 0.3, 1.5);
        self.voiceless = if f.singing.max(f.rapping) < 0.3 { self.voiceless + dt } else { 0.0 };

        // Drops and the mosh pit. "Drive": loudness, low end and activity together.
        let drive = (self.rel_loud + f.bass + (f.note_density * 2.0).min(1.0)) / 3.0;
        if self.drive_long.is_nan() {
            if gated {
                // (Nothing heard yet: start the memory with the first sound.)
                self.drive_long = f32::NAN;
            } else {
                self.drive_long = drive;
                self.drive_short = drive;
            }
        }
        self.drive_short = follow(self.drive_short, drive, dt, 0.3, 0.3);
        self.drive_long = follow(self.drive_long, drive, dt, DRIVE_MEMORY, DRIVE_MEMORY);
        f.drop *= (-dt / crate::features::RAP_ENTRY_DECAY).exp();
        // No drops until the memory has filled (at the start everything is a jump).
        self.drop_cooldown -= dt;
        let jump = self.drive_short - self.drive_long;
        self.raw.drive = [self.drive_short, self.drive_long];
        // A crowd needs a beat to mosh to: a piano swelling up is no drop.
        let beat = f.pulse_clarity > DROP_PULSE;
        if !gated && beat && self.drop_cooldown <= 0.0 && jump > DROP_JUMP && self.drive_short > DROP_LEVEL && !jump.is_nan() {
            f.drop = ((jump - DROP_JUMP) / DROP_JUMP + 0.5).min(1.0);
            self.drop_cooldown = crate::features::DROP_COOLDOWN;
            self.energy.release();
        }
        // Heavy: a sustained distorted, noisy upper range (distorted guitars). Cymbals
        // and hats are noisy too, but only for moments: hence the median over ~2 s.
        if self.flat_history.len() == FLAT_HOPS {
            self.flat_history.pop_front();
        }
        self.flat_history.push_back(if gated { 0.0 } else { self.raw.flatness });
        self.flat_tick += 1;
        if self.flat_tick % 4 == 0 {
            let mut v: Vec<f32> = self.flat_history.iter().copied().collect();
            self.flatness = median(&mut v);
        }
        let heavy = smoothstep(MOSH_FLAT.0, MOSH_FLAT.1, self.flatness);
        // Loud (not drive: onsets don't stand out in a wall of distorted guitars,
        // so activity reads low exactly there), on some beat, heavy.
        let parts = [smoothstep(0.5, 0.8, self.rel_loud), smoothstep(0.25, 0.5, f.pulse_clarity), heavy];
        self.raw.mosh_parts = parts;
        let mosh = parts[0] * parts[1] * parts[2];
        // Slow to fade: a clean riff in a heavy song doesn't end the pit.
        f.mosh = follow(f.mosh, mosh, dt, 0.5, MOSH_FADE);
        // The pit starting after a lull is a drop too (often the moment that
        // matters most: the bass and the distortion slamming in at once).
        if f.mosh > PIT_ON && self.pit_off >= PIT_LULL && self.drop_cooldown <= 0.0 && !gated {
            f.drop = f.drop.max(0.8);
            self.drop_cooldown = crate::features::DROP_COOLDOWN;
            self.energy.release();
        }
        self.pit_off = if f.mosh < PIT_OFF { self.pit_off + dt } else if f.mosh > PIT_ON { 0.0 } else { self.pit_off };
        self.rel_loud = follow(self.rel_loud, loud, dt, 0.02, 0.3);

        // The aspects (`aspects.rs`): loudness, fullness, drive, heaviness,
        // and the moments; energy and tension come out of them.
        // Whoever is in front (a voice, a solo) sits in the middle of the
        // stereo image, between 300 Hz and 3 kHz: its level against loud music.
        let (k0, k1) = ((300.0 / bin_hz) as usize, (3000.0 / bin_hz) as usize);
        let in_front: f32 = self.lead.power[k0..k1].iter().sum();
        let centre = 10.0 * (in_front / self.ear.reference().max(1e-12) + 1e-9).log10();
        let heard = crate::aspects::Heard {
            centre_db: centre,
            pulse: f.pulse_clarity,
            tempo: f.tempo,
            density: f.note_density,
            distorted: heavy,
            voice: f.singing.max(f.rapping),
            lead_hz: self.raw.melody_hz,
            tension,
            build: energy.build,
        };
        self.aspects.push(&self.ear.out, &heard, dt, gated);
        let a = self.aspects.out;
        f.loudness = a.loudness;
        f.fullness = a.fullness;
        f.drive = a.drive;
        f.heaviness = a.heaviness;
        f.solo = a.solo;
        f.energy = a.energy;
        f.tension = a.tension * presence;
        f.entrance = a.entrance;
        f.everyone_in = a.everyone_in;
        f.exit = a.exit;
        f.release = a.release;
        self.raw.energy_parts = [a.loudness, a.fullness, a.drive, a.heaviness, a.lift];
        self.raw.layers = a.layers;
        self.raw.hold = a.hold;
        f.bass = follow(f.bass, bands[0], dt, 0.01, 0.25);
        f.low_mids = follow(f.low_mids, bands[1], dt, 0.05, 0.4);
        f.high_mids = follow(f.high_mids, bands[2], dt, 0.05, 0.4);
        f.treble = follow(f.treble, bands[3], dt, 0.01, 0.2);
        f.brightness = follow(f.brightness, centroid, dt, 0.1, 0.3);
        let decay = (-dt / crate::features::EVENT_DECAY).exp();
        f.onset *= decay;
        f.beat *= decay;
        self.density *= (-dt / 2.0).exp();
        if let Some(strength) = onset_strength {
            f.onsets += 1;
            f.onset = f.onset.max(strength);
            self.density += strength;
        }
        f.note_density = (self.density / 2.0 / 6.0).min(1.0);
        if let Some(strength) = beat_strength {
            f.beats += 1;
            f.beat_strength = f.beat_strength.max(strength);
            f.beat = f.beat.max(strength * presence);
        }
        f.tempo = crate::features::tempo01(self.beat.tempo_bpm()) * presence;
        f.pulse_clarity = self.beat.clarity * presence;
        if !gated {
            f.bass_pitch = follow(f.bass_pitch, bass_pitch, dt, 0.05, 0.05);
        }
        if presence < 1.0 {
            // No music: the positions drift back to the middle.
            for v in [&mut f.brightness, &mut f.bass_pitch, &mut f.dominant_pitch, &mut self.melody.pitch] {
                *v = follow(*v, REST, dt, 0.5, 0.5);
            }
        }
        let spectrum = self.spectrum(bin_hz, dt, gated);
        for (v, target) in self.features.spectrum.iter_mut().zip(spectrum) {
            *v = follow(*v, target, dt, 0.015, 0.25);
        }
        // Dominant steady pitch: the peak of the spectrum averaged over ~2 s.
        for (s, &v) in self.slow_spectrum.iter_mut().zip(&self.features.spectrum) {
            *s = follow(*s, v, dt, 2.0, 2.0);
        }
        let peak = (0..SPECTRUM_BINS).max_by(|&a, &b| self.slow_spectrum[a].total_cmp(&self.slow_spectrum[b])).unwrap_or(0);
        if self.slow_spectrum[peak] > 0.05 && presence >= 1.0 {
            self.features.dominant_pitch = peak as f32 / (SPECTRUM_BINS - 1) as f32;
        }
    }

    #[cfg(test)]
    pub fn silent_for(&self) -> f32 {
        self.silent
    }

    /// The song is over: forget what was learned about it. (The harmony, the
    /// build-up and the aspects do the same for themselves; the ear keeps its
    /// reference, which is about the listening level, not the song.)
    fn forget_the_song(&mut self) {
        self.forgotten = true;
        self.beat = crate::beat::BeatTracker::new(self.beat.hop_secs());
        self.density = 0.0;
        self.drive_long = f32::NAN;
        self.drive_short = 0.0;
        self.band_ceiling = None;
        self.spectrum_ceiling = None;
        self.motion = [0.0; 4];
    }

    /// Normalised spectrum bands (unsmoothed), on a shared ceiling like the
    /// four bands, each fading out if it holds a steady sound.
    fn spectrum(&mut self, bin_hz: f32, dt: f32, gated: bool) -> [f32; SPECTRUM_BINS] {
        let mut db = [0.0f32; SPECTRUM_BINS];
        for (i, d) in db.iter_mut().enumerate() {
            let (lo, hi) = spectrum_edges(i);
            // Mean power per FFT bin in the band, times the band's width in
            // bins: the band's power, also for low bands narrower than one bin
            // (they read the bin nearest their centre).
            let (k0, k1) = ((lo / bin_hz).ceil() as usize, (hi / bin_hz).ceil() as usize);
            let k1 = k1.min(self.power.len());
            let mean = if k1 > k0 {
                self.power[k0..k1].iter().sum::<f32>() / (k1 - k0) as f32
            } else {
                self.power[((spectrum_center(i) / bin_hz).round() as usize).min(self.power.len() - 1)]
            };
            let power = mean * (hi - lo) / bin_hz;
            *d = 10.0 * (power + 1e-12).log10()
                + SPECTRUM_TILT_DB_PER_OCTAVE * (spectrum_center(i) / 1000.0).log2();
        }
        self.raw.spectrum_db = db;
        if gated {
            return [0.0; SPECTRUM_BINS];
        }
        let top = db.iter().copied().fold(f32::MIN, f32::max);
        let ceil = self.spectrum_ceiling.get_or_insert(top);
        *ceil = if top > *ceil { follow(*ceil, top, dt, 0.05, 0.05) } else { follow(*ceil, top, dt, SCALE_MEMORY, SCALE_MEMORY) };
        let lo = *ceil - SPECTRUM_RANGE_DB;
        std::array::from_fn(|i| {
            self.gain_spectrum[i].track(db[i], dt);
            ((db[i] - lo) / SPECTRUM_RANGE_DB).clamp(0.0, 1.0) * self.gain_spectrum[i].presence
        })
    }
}

/// The part of the spectrum a lead line lives in: the centre of the stereo
/// image, without percussion, with the bass damped.
///
/// - **Centre**: per bin, how alike the two channels are, in level and in
///   phase. A source panned centre is identical in both (kept); one panned to
///   a side, or wide reverb, differs (dropped). Mono input is all centre.
/// - **Harmonic**: a sustained sound is smooth along time in its bin, a hit is
///   smooth across frequency in its frame. The median over the last frames
///   (per bin) against the median across nearby bins (per frame) gives a soft
///   mask that keeps the sustained part. Causal: only past frames.
/// - **Bass** is damped below ~350 Hz (gone below 150): bass lines are centred
///   and sustained too, and their harmonics (110, 220, 330 Hz… for a 55 Hz
///   note) would win otherwise. A voice is still found by its harmonics: its
///   pitch doesn't need its fundamental.
pub struct LeadSpectrum {
    /// The lead spectrum's power per bin (what the melody tracker reads).
    pub power: Vec<f32>,
    /// The centre magnitudes of the last `HPSS_FRAMES` frames, per bin.
    history: Vec<Vec<f32>>,
    next: usize,
    centre: Vec<f32>,
    scratch: Vec<f32>,
}

/// Frames in the harmonic median (~100 ms), and bins either side in the
/// percussive one (~170 Hz).
const HPSS_FRAMES: usize = 9;
const HPSS_BINS: usize = 8;

impl LeadSpectrum {
    fn new(bins: usize) -> Self {
        Self {
            power: vec![0.0; bins],
            history: vec![vec![0.0; bins]; HPSS_FRAMES],
            next: 0,
            centre: vec![0.0; bins],
            scratch: Vec::with_capacity(2 * HPSS_BINS + HPSS_FRAMES + 1),
        }
    }

    fn update(&mut self, l: &[Complex<f32>], r: &[Complex<f32>], norm: f32, bin_hz: f32) {
        let n = self.power.len();
        for k in 0..n {
            let (a, b) = (l[k], r[k]);
            let (pa, pb) = (a.norm_sqr(), b.norm_sqr());
            let cross = a * b.conj();
            // Alike in level (1 when equal) and in phase (cos of the difference).
            let level = 2.0 * (pa * pb).sqrt() / (pa + pb + 1e-20);
            let phase = (cross.re / (cross.norm() + 1e-20)).max(0.0);
            let mask = (level * phase).powi(2);
            self.centre[k] = 0.5 * (a + b).norm() * norm * mask;
        }
        self.history[self.next].copy_from_slice(&self.centre);
        self.next = (self.next + 1) % HPSS_FRAMES;
        for k in 0..n {
            self.scratch.clear();
            self.scratch.extend(self.history.iter().map(|h| h[k]));
            let harmonic = median(&mut self.scratch);
            self.scratch.clear();
            self.scratch.extend_from_slice(&self.centre[k.saturating_sub(HPSS_BINS)..(k + HPSS_BINS + 1).min(n)]);
            let percussive = median(&mut self.scratch);
            let (h2, p2) = (harmonic * harmonic, percussive * percussive);
            let keep = h2 / (h2 + p2 + 1e-20);
            let f = k as f32 * bin_hz;
            // Fully damped below 150 Hz, fully kept above 350 Hz.
            let bass = ((f - 150.0) / 200.0).clamp(0.0, 1.0);
            let m = self.centre[k] * keep * bass;
            self.power[k] = m * m;
        }
    }
}

/// A drop: drive at least this much above its recent average (`DRIVE_MEMORY` s)…
const DRIVE_MEMORY: f32 = 6.0;
const DROP_JUMP: f32 = 0.2;
/// …and at least this high, on at least this clear a beat.
const DROP_LEVEL: f32 = 0.55;
const DROP_PULSE: f32 = 0.5;
/// The mosh pit fades over this long (s)…
const MOSH_FADE: f32 = 4.0;
/// …and it starting counts as a drop when it rises past `PIT_ON` after at
/// least `PIT_LULL` s below `PIT_OFF`.
const PIT_ON: f32 = 0.8;
const PIT_OFF: f32 = 0.3;
const PIT_LULL: f32 = 6.0;
/// Hops in the flatness median (~2 s).
const FLAT_HOPS: usize = 180;
/// Flatness (the median over ~2 s) that reads as clean / fully distorted. On the
/// reference tracks the median is 0.02 (solo piano), 0.07–0.08 (disco-funk,
/// guitar rock), 0.14 (distorted indie rock).
const MOSH_FLAT: (f32, f32) = (0.11, 0.20);

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn median(v: &mut [f32]) -> f32 {
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, f32::total_cmp).1
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(SR * secs) as usize)
            .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    fn last_raw(samples: &[f32]) -> Raw {
        let mut a = Analyzer::new(SR);
        let mut raw = Raw::default();
        a.push(samples, |_, r| raw = *r);
        raw
    }

    fn loudest_band(r: &Raw) -> usize {
        (0..4).max_by(|&a, &b| r.bands_db[a].total_cmp(&r.bands_db[b])).unwrap()
    }

    #[test]
    fn sines_land_in_their_bands() {
        for (freq, band) in [(80.0, 0), (300.0, 1), (1000.0, 2), (4000.0, 3)] {
            let r = last_raw(&sine(freq, 0.5, 0.5));
            assert_eq!(loudest_band(&r), band, "{freq} Hz: {:?}", r.bands_db);
        }
    }

    #[test]
    fn level_and_centroid_of_a_sine() {
        let r = last_raw(&sine(1000.0, 0.5, 0.5));
        // RMS of a 0.5-amplitude sine is 0.354 → −9 dBFS.
        assert!((r.level_db + 9.0).abs() < 0.5, "{}", r.level_db);
        assert!((r.centroid_hz - 1000.0).abs() < 60.0, "{}", r.centroid_hz);
    }

    #[test]
    fn clicks_are_detected_as_onsets() {
        // Quiet noise floor with a short burst every 0.5 s, starting at 0.25 s.
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let n = (SR * 4.0) as usize;
        let mut s: Vec<f32> = (0..n).map(|_| 0.003 * noise()).collect();
        let clicks: Vec<f32> = (0..8).map(|i| 0.25 + 0.5 * i as f32).collect();
        for &t in &clicks {
            let start = (t * SR) as usize;
            for i in 0..480 {
                let env = (-(i as f32) / 120.0).exp();
                s[start + i] += 0.6 * env * noise();
            }
        }
        let mut a = Analyzer::new(SR);
        let mut hops = 0usize;
        let mut onsets = Vec::new();
        a.push(&s, |_, r| {
            hops += 1;
            if r.onset {
                // Time of the newest sample in the window.
                onsets.push(((hops - 1) * HOP + FFT_SIZE) as f32 / SR);
            }
        });
        assert_eq!(onsets.len(), clicks.len(), "onsets at {onsets:?}");
        for (o, c) in onsets.iter().zip(&clicks) {
            assert!((o - c).abs() < 0.03, "onset {o} vs click {c}");
        }
    }

    #[test]
    fn bass_pitch_orders_low_and_high_bass() {
        let pitch = |freq: f32| {
            let mut a = Analyzer::new(SR);
            let mut p = 0.0;
            a.push(&sine(freq, 1.0, 0.5), |f, _| p = f.bass_pitch);
            p
        };
        let (p45, p80, p130) = (pitch(45.0), pitch(80.0), pitch(130.0));
        assert!(p45 < p80 && p80 < p130, "{p45} {p80} {p130}");
        assert!(p45 < 0.3 && p130 > 0.7, "{p45} {p130}");
    }

    #[test]
    fn kicks_and_hats_give_beats_on_the_kicks() {
        // 16 s at 120 BPM: a kick on every beat, a hat on every off-beat.
        let secs = 16.0;
        let n = (SR * secs) as usize;
        let mut s = vec![0.0f32; n];
        let mut seed = 7u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for k in 0..32 {
            let kick = ((0.25 + 0.5 * k as f32) * SR) as usize;
            for i in 0..(0.2 * SR) as usize {
                let t = i as f32 / SR;
                s[kick + i] += 0.8 * (-t * 15.0).exp() * (std::f32::consts::TAU * 55.0 * t).sin();
            }
            let hat = kick + (0.25 * SR) as usize;
            for i in 0..(0.04 * SR) as usize {
                if hat + i < n {
                    s[hat + i] += 0.3 * (-(i as f32) / 300.0).exp() * noise();
                }
            }
        }
        let mut a = Analyzer::new(SR);
        let mut hops = 0usize;
        let mut beats = Vec::new();
        let mut tempo = 0.0;
        a.push(&s, |_, r| {
            hops += 1;
            tempo = r.tempo_bpm;
            if r.beat {
                beats.push(((hops - 1) * HOP + FFT_SIZE) as f32 / SR);
            }
        });
        assert!((tempo - 120.0).abs() < 4.0, "tempo {tempo}");
        let late: Vec<f32> = beats.into_iter().filter(|&t| t > 9.0).collect();
        assert!(late.len() >= 12, "beats {late:?}");
        for &t in &late {
            // Kicks at 0.25 + 0.5k; the hats sit exactly between.
            let off = ((t - 0.25) / 0.5).fract();
            let off = off.min(1.0 - off) * 0.5;
            assert!(off < 0.07, "beat at {t:.3} is {off:.3} s from a kick");
        }
    }

    #[test]
    fn shared_gain_shows_which_band_dominates() {
        // Pulsing (so it isn't "steady") bass with a quiet hiss on top.
        let n = (SR * 4.0) as usize;
        let mut seed = 3u32;
        let s: Vec<f32> = (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let hiss = ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.01;
                let t = i as f32 / SR;
                let pulse = 0.5 + 0.5 * (std::f32::consts::TAU * 2.0 * t).sin();
                0.5 * pulse * (std::f32::consts::TAU * 70.0 * t).sin() + hiss
            })
            .collect();
        let mut a = Analyzer::new(SR);
        let (mut low, mut high) = (0.0f32, 0.0f32);
        a.push(&s, |f, _| {
            low = low.max(f.bass);
            high = high.max(f.treble);
        });
        assert!(low > 0.8, "low {low}");
        assert!(high < 0.5 * low, "high {high} vs low {low}");
    }

    /// (drop strength seen, mosh at the end) for a stretch of samples.
    fn energy(samples: &[f32]) -> (f32, f32) {
        let mut a = Analyzer::new(SR);
        let (mut drop, mut mosh) = (0.0f32, 0.0f32);
        for chunk in samples.chunks(HOP) {
            a.push(chunk, |f, _| {
                drop = drop.max(f.drop);
                mosh = f.mosh;
            });
        }
        (drop, mosh)
    }

    /// Kicks at 120 BPM, a bass note, and (if `distortion` > 0) a wall of noise
    /// standing in for distorted guitars, at `level`.
    fn band(secs: f32, level: f32, distortion: f32, seed: u64) -> Vec<f32> {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        (0..(secs * SR) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                let kt = t % 0.5;
                let kick = (std::f32::consts::TAU * 55.0 * kt).sin() * (-kt / 0.1).exp();
                let bass = 0.3 * (std::f32::consts::TAU * 82.0 * t).sin();
                let wall = distortion * rng.random_range(-1.0..1.0);
                level * (0.6 * kick + bass + wall)
            })
            .collect()
    }

    #[test]
    fn the_band_crashing_back_in_is_a_drop_and_a_mosh_pit() {
        // The band, a quiet breakdown with a soft beat, then the band crashing
        // back in, loud and distorted. (Loudness is relative to the last
        // minute: a breakdown is quiet only after something louder.)
        let mut x = band(12.0, 0.5, 0.35, 1);
        x.extend(band(10.0, 0.03, 0.0, 2));
        x.extend(band(10.0, 0.5, 0.35, 3));
        let (drop, mosh) = energy(&x);
        assert!(drop > 0.5, "drop {drop}");
        assert!(mosh > 0.5, "mosh {mosh}");
    }

    #[test]
    fn a_piano_swelling_up_without_a_beat_is_no_drop() {
        // Soft, then loud, held chords: no beat, no distortion.
        let chord = |secs: f32, amp: f32| -> Vec<f32> {
            (0..(secs * SR) as usize)
                .map(|i| {
                    let t = i as f32 / SR;
                    amp * [262.0f32, 330.0, 392.0].iter().map(|f| (std::f32::consts::TAU * f * t).sin()).sum::<f32>() * (-(t % 2.0) / 1.5).exp()
                })
                .collect()
        };
        let mut x = chord(12.0, 0.3);
        x.extend(chord(10.0, 0.02));
        x.extend(chord(10.0, 0.3));
        let (drop, mosh) = energy(&x);
        assert!(drop < 0.1, "drop {drop}");
        assert!(mosh < 0.1, "mosh {mosh}");
    }

    #[test]
    fn silence_reads_as_zero() {
        let mut a = Analyzer::new(SR);
        let mut last = AudioFeatures::default();
        a.push(&vec![0.0; (SR * 1.0) as usize], |f, _| last = f.clone());
        assert_eq!(last.loudness, 0.0);
        assert_eq!(last.bass, 0.0);
        assert_eq!(last.onset, 0.0);
    }

    #[test]
    fn steady_hum_fades_out_but_beats_keep_reading() {
        let mut a = Analyzer::new(SR);
        let mut last = AudioFeatures::default();
        a.push(&sine(120.0, 40.0, 0.2), |f, _| last = f.clone());
        assert!(last.loudness < 0.15, "steady hum loudness {}", last.loudness);
        assert!(last.bass < 0.15, "steady hum low band {}", last.bass);

        // Same hum with a kick every 0.5 s on top: the kicks must register.
        let mut s = sine(120.0, 40.0, 0.2);
        for k in 0..80 {
            let start = (k as f32 * 0.5 * SR) as usize;
            for i in 0..(0.15 * SR) as usize {
                let t = i as f32 / SR;
                s[start + i] += 0.7 * (-t * 25.0).exp() * (std::f32::consts::TAU * 55.0 * t).sin();
            }
        }
        let mut a = Analyzer::new(SR);
        let mut peak_low = 0.0f32;
        let mut n = 0;
        a.push(&s, |f, _| {
            n += 1;
            // Last 5 s only.
            if n as f32 * HOP as f32 / SR > 35.0 {
                peak_low = peak_low.max(f.bass);
            }
        });
        assert!(peak_low > 0.6, "kicks over hum: peak low {peak_low}");
    }

    #[test]
    fn loud_after_quiet_reads_high() {
        let mut s = sine(200.0, 2.0, 0.01);
        s.extend(sine(200.0, 1.0, 0.5));
        let mut a = Analyzer::new(SR);
        let (mut n, mut quiet) = (0usize, 0.0);
        let mut last = AudioFeatures::default();
        a.push(&s, |f, _| {
            n += 1;
            if (n * HOP) as f32 / SR < 1.9 {
                quiet = f.loudness;
            }
            last = f.clone();
        });
        // One pure tone is never as loud as music filling every band.
        assert!(last.loudness > quiet + 0.1, "quiet {quiet}, loud {}", last.loudness);
    }

    #[test]
    fn spectrum_peaks_at_the_tone() {
        for freq in [60.0, 1000.0, 6000.0] {
            let mut a = Analyzer::new(SR);
            // A little noise so the tone is not "steady" against silence, then the tone.
            let mut x = sine(freq, 1.0, 0.3);
            for (i, v) in x.iter_mut().enumerate() {
                *v *= if (i / 4800) % 2 == 0 { 1.0 } else { 0.3 };
            }
            let mut spectrum = [0.0; SPECTRUM_BINS];
            a.push(&x, |f, _| spectrum = f.spectrum);
            let peak = (0..SPECTRUM_BINS).max_by(|&i, &j| spectrum[i].total_cmp(&spectrum[j])).unwrap();
            let (lo, hi) = spectrum_edges(peak);
            assert!(lo * 0.8 <= freq && freq <= hi * 1.25, "{freq} Hz peaked in {lo:.0}-{hi:.0} Hz: {spectrum:?}");
        }
    }
}

#[cfg(test)]
mod after_the_end {
    use super::*;

    fn band(bars: usize, bpm: f32) -> Vec<f32> {
        let row = ["C", "G", "Am", "F"].iter().cycle().take(bars).map(|c| format!("band={c}")).collect::<Vec<_>>().join(" ");
        crate::scenarios::Scenario::new("", "", bpm)
            .chords(&row, 4.0, |s| {
                (s.kick, s.hat, s.snare, s.melody, s.sixteenths) = (true, true, true, true, true);
                s.bass = s.notes.first().map(|n| n - 24);
            })
            .render()
    }

    fn hiss(secs: f32, db: f32) -> Vec<f32> {
        let amp = 10f32.powf(db / 20.0) * 3f32.sqrt();
        let mut seed = 1u32;
        (0..(crate::scenarios::SAMPLE_RATE * secs) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                amp * ((seed >> 8) as f32 / 8388608.0 - 1.0)
            })
            .collect()
    }

    fn last(a: &mut Analyzer, samples: &[f32]) -> AudioFeatures {
        let mut l = AudioFeatures::default();
        for chunk in samples.chunks(HOP) {
            a.push(chunk, |f, _| l = f.clone());
        }
        l
    }

    /// Nothing of the song is left: what measures the music reads 0, and
    /// what says where it sits (brightness, pitches, mood) is in the middle.
    fn assert_at_rest(f: &AudioFeatures, when: &str) {
        let zero = [
            ("loudness", f.loudness), ("energy", f.energy), ("fullness", f.fullness), ("drive", f.drive), ("heaviness", f.heaviness), ("solo", f.solo),
            ("tension", f.tension), ("bass", f.bass), ("treble", f.treble), ("tempo", f.tempo), ("pulse", f.pulse_clarity), ("singing", f.singing),
            ("mosh", f.mosh), ("tonality", f.tonality), ("beat", f.beat), ("onset", f.onset), ("notes", f.chroma.iter().sum()), ("spectrum", f.spectrum.iter().sum()),
        ];
        for (name, v) in zero {
            assert!(v.abs() < 0.02, "{when}: {name} {v}");
        }
        for (name, v) in [("brightness", f.brightness), ("bass pitch", f.bass_pitch), ("dominant pitch", f.dominant_pitch), ("lead pitch", f.melody_pitch), ("major", f.major)] {
            assert!((v - REST).abs() < 0.02, "{when}: {name} {v}");
        }
    }

    #[test]
    fn before_any_music_everything_is_at_rest() {
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        assert_at_rest(&last(&mut a, &vec![0.0; 48_000]), "before");
    }

    #[test]
    fn after_a_song_everything_comes_to_rest() {
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        let playing = last(&mut a, &band(8, 120.0));
        assert!(playing.tempo > 0.4 && playing.pulse_clarity > 0.5 && playing.tonality > 0.3, "{} {} {}", playing.tempo, playing.pulse_clarity, playing.tonality);
        assert_at_rest(&last(&mut a, &vec![0.0; 6 * 48_000]), "6 s after the song");
    }

    #[test]
    fn hiss_after_a_song_is_silence_too() {
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        last(&mut a, &band(8, 120.0));
        // The noise floor of a tape or a quiet room: nothing below the meter's gate.
        assert_at_rest(&last(&mut a, &hiss(8.0, -45.0)), "8 s into the hiss");
        assert_at_rest(&last(&mut a, &hiss(30.0, -45.0)), "38 s into the hiss");
    }

    #[test]
    fn a_gap_in_the_music_does_not_end_the_song() {
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        let before = last(&mut a, &band(8, 120.0));
        // Under a second of nothing: a break, the breath before a drop.
        let gap = last(&mut a, &vec![0.0; (0.9 * 48_000.0) as usize]);
        assert!((gap.tempo - before.tempo).abs() < 0.01 && gap.pulse_clarity > 0.5, "tempo {} → {}, pulse {}", before.tempo, gap.tempo, gap.pulse_clarity);
        assert!((gap.brightness - before.brightness).abs() < 0.05, "brightness {} → {}", before.brightness, gap.brightness);
        let after = last(&mut a, &band(2, 120.0));
        assert!((after.tempo - before.tempo).abs() < 0.03, "tempo {} → {}", before.tempo, after.tempo);
    }

    #[test]
    fn the_next_song_starts_afresh() {
        // The same song heard first, and heard after another at another tempo
        // and a silence, reads the same.
        let song = band(6, 150.0);
        let alone = last(&mut Analyzer::new(crate::scenarios::SAMPLE_RATE), &song);
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        last(&mut a, &band(8, 96.0));
        last(&mut a, &vec![0.0; 5 * 48_000]);
        let next = last(&mut a, &song);
        for (name, x, y) in [("tempo", alone.tempo, next.tempo), ("pulse", alone.pulse_clarity, next.pulse_clarity), ("energy", alone.energy, next.energy), ("bass", alone.bass, next.bass), ("tonality", alone.tonality, next.tonality)] {
            assert!((x - y).abs() < 0.05, "{name}: alone {x}, after another song {y}");
        }
        let first = last(&mut Analyzer::new(crate::scenarios::SAMPLE_RATE), &band(8, 96.0));
        assert!((alone.tempo - first.tempo).abs() > 0.1, "the two songs' tempo: {} and {}", first.tempo, alone.tempo);
    }

    /// Run by hand: `SCEN=syn-steady cargo test --release -- --ignored end_probe --nocapture`
    #[test]
    #[ignore]
    fn end_probe() {
        let name = std::env::var("SCEN").unwrap_or("syn-steady".into());
        let s = crate::scenarios::all().into_iter().find(|s| s.name == name).unwrap();
        let mut samples = s.render();
        let music = samples.len() as f32 / crate::scenarios::SAMPLE_RATE;
        // TAIL: the level (dBFS) of hiss after the music; none by default.
        let hiss: Option<f32> = std::env::var("TAIL").ok().and_then(|s| s.parse().ok());
        let amp = hiss.map_or(0.0, |db| 10f32.powf(db / 20.0) * 3f32.sqrt());
        let mut seed = 1u32;
        samples.extend((0..(crate::scenarios::SAMPLE_RATE * 40.0) as usize).map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            amp * ((seed >> 8) as f32 / 8388608.0 - 1.0)
        }));
        let mut a = Analyzer::new(crate::scenarios::SAMPLE_RATE);
        let per = (crate::scenarios::SAMPLE_RATE / HOP as f32) as usize;
        let from: f32 = std::env::var("FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(music - 3.0);
        println!("END music ends at {music:.1}");
        println!("END    t  loud   nrg  full drive heavy  solo  tens  bass  lowm  high  treb  brig tempo pulse  dens  sing  mosh major tonal   hue  bpit  dpit  mpit level chrom");
        let mut hop = 0;
        for chunk in samples.chunks(HOP) {
            let mut l = AudioFeatures::default();
            a.push(chunk, |f, _| l = f.clone());
            hop += 1;
            let t = hop as f32 / per as f32;
            if hop % per == 0 && t >= from {
                let spec = a.ear.out.now;
                let chroma = l.chroma.iter().sum::<f32>();
                let v = [l.loudness, l.energy, l.fullness, l.drive, l.heaviness, l.solo, l.tension, l.bass, l.low_mids, l.high_mids, l.treble, l.brightness, l.tempo, l.pulse_clarity, l.note_density, l.singing, l.mosh, l.major, l.tonality, l.harmony_hue, l.bass_pitch, l.dominant_pitch, l.melody_pitch, spec, chroma];
                println!("END {t:4.0} {}", v.map(|x| format!("{x:5.2}")).join(" "));
            }
        }
    }
}
