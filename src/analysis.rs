//! Turns a mono sample stream into [`AudioFeatures`], at one frame per hop
//! (512 samples ≈ 10.7 ms at 48 kHz). Pure and thread-agnostic: the audio
//! thread pushes samples in, frames come out.
//!
//! Per hop: a 2048-sample Hann-windowed FFT gives
//! - loudness (RMS, dBFS),
//! - energy in four bands and 32 spectrum bands (dB),
//! - spectral centroid (Hz),
//! - spectral flux on log magnitudes: onsets, and the beat tracker's input,
//! - where the bass sits in pitch, and the dominant steady pitch.
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

/// Unnormalised measurements of one hop (used by the tests).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default)]
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
    input: Vec<f32>,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
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
            input: Vec::with_capacity(FFT_SIZE * 2),
            frame: vec![0.0; FFT_SIZE],
            prev_log_mag: vec![0.0; spectrum.len()],
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
            features: AudioFeatures::default(),
            raw: Raw::default(),
        }
    }

    /// Feed mono samples; `on_frame` is called once per completed hop.
    pub fn push(&mut self, samples: &[f32], mut on_frame: impl FnMut(&AudioFeatures, &Raw)) {
        self.input.extend_from_slice(samples);
        while self.input.len() >= FFT_SIZE {
            self.analyse();
            on_frame(&self.features, &self.raw);
            self.features.clear_events();
            self.input.drain(..HOP);
        }
    }

    fn analyse(&mut self) {
        let dt = HOP as f32 / self.sample_rate;
        let bin_hz = self.sample_rate / FFT_SIZE as f32;

        // Level of the newest hop region of the window (time-domain RMS).
        let mut sum_sq = 0.0;
        for (o, (&x, &w)) in self.frame.iter_mut().zip(self.input[..FFT_SIZE].iter().zip(&self.window)) {
            *o = x * w;
            sum_sq += x * x;
        }
        let rms = (sum_sq / FFT_SIZE as f32).sqrt();
        let level_db = 20.0 * (rms + 1e-9).log10();

        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .expect("fft buffer sizes are fixed");

        // Magnitudes, scaled so a full-scale sine reads about 1.
        let norm = 4.0 / FFT_SIZE as f32;
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

        let gated = level_db < GATE_DB;
        let onset_strength = self.onsets.detect(flux, dt, gated);
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
        };

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
            (loud, bands, c)
        };

        let f = &mut self.features;
        f.level_db = level_db;
        f.loudness = follow(f.loudness, loud, dt, 0.02, 0.3);
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
            f.beat = f.beat.max(strength);
        }
        f.tempo = crate::features::tempo01(self.beat.tempo_bpm());
        f.pulse_clarity = self.beat.clarity;
        if !gated {
            f.bass_pitch = follow(f.bass_pitch, bass_pitch, dt, 0.05, 0.05);
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
        if self.slow_spectrum[peak] > 0.05 {
            self.features.dominant_pitch = peak as f32 / (SPECTRUM_BINS - 1) as f32;
        }
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
        let mut last = AudioFeatures::default();
        a.push(&s, |f, _| last = f.clone());
        assert!(last.loudness > 0.8, "{}", last.loudness);
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
