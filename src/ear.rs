//! A small model of hearing, after Zwicker: what the ear makes of a spectrum.
//!
//! Power is not loudness. The ear splits sound into about 24 critical bands
//! (the Bark scale), hears each band's power compressed (twice as loud takes
//! ten times the power), and adds the bands up. So the same power spread over
//! more bands is louder: a voice or a guitar coming in between a bass and a
//! hi-hat adds loudness though the meter hardly moves.
//!
//! Per hop this gives:
//!
//! - **loudness**: the sum of the bands' specific loudness (in sones, up to a
//!   constant: only ratios mean anything, since the playback level is unknown);
//! - **sharpness**: where the loudness sits, with the top counted more (acum,
//!   after DIN 45692);
//! - **impact**: how much loudness is newly struck per second, as a share of
//!   the loudness (hits, in the ear's own units);
//! - **fullness**: over how many bands the loudness is spread (1..24).
//!
//! Left out: roughness (it needs envelope modulation up to 300 Hz, and a hop
//! is 12 ms) and the exact level dependence of masking.

use crate::features::follow;

/// Upper edges (Hz) of the 24 critical bands; the first starts at 20 Hz.
const BARK_EDGES: [f32; 24] = [
    100.0, 200.0, 300.0, 400.0, 510.0, 630.0, 770.0, 920.0, 1080.0, 1270.0, 1480.0, 1720.0, 2000.0, 2320.0, 2700.0, 3150.0, 3700.0, 4400.0, 5300.0, 6400.0,
    7700.0, 9500.0, 12000.0, 15500.0,
];
pub const BANDS: usize = 24;
/// Specific loudness grows with the band's power to this (Zwicker: 0.23).
const COMPRESSION: f32 = 0.23;
/// A band this far (dB) below the loudest the stream has been is at the
/// threshold of hearing.
const THRESHOLD_DB: f32 = 60.0;
/// The ear's reference: the level of loud music. It starts at what a
/// mastered recording gives (a file, the computer's output) and follows the
/// loudest the stream has been, forgetting this slowly (s): a quiet intro
/// reads quiet, a quiet recording stays quiet, and a microphone far from the
/// music is brought within reach in seconds.
const REFERENCE_START: f32 = 0.15;
/// It comes down towards the loudest of the last ten seconds by this many dB
/// a second; faster while it is more than `REFERENCE_FAR` dB above it (a
/// microphone far from the music, not a quiet passage).
const REFERENCE_FALL: f32 = 0.03;
const REFERENCE_FALL_FAR: f32 = 3.0;
const REFERENCE_FAR: f32 = 20.0;
/// Masking: a band's excitation spreads to its neighbours, falling by this
/// many dB per band upwards and downwards.
const SPREAD_UP_DB: f32 = 12.0;
const SPREAD_DOWN_DB: f32 = 27.0;

/// A-weighting (dB) at `f` Hz: the ear's sensitivity at a low level.
fn a_weight_db(f: f32) -> f32 {
    let f2 = f * f;
    let ra = 12194.0f32.powi(2) * f2 * f2 / ((f2 + 20.6f32.powi(2)) * ((f2 + 107.7f32.powi(2)) * (f2 + 737.9f32.powi(2))).sqrt() * (f2 + 12194.0f32.powi(2)));
    20.0 * ra.log10() + 2.0
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EarOut {
    /// Loudness, averaged over a fifth of a second (sones up to a constant).
    pub loudness: f32,
    /// Sharpness (acum, roughly: 1 = narrow-band noise at 1 kHz).
    pub sharpness: f32,
    /// Loudness newly struck per second, as a share of the loudness.
    pub impact: f32,
    /// The same below 300 Hz only (kick, bass), as a share of all loudness.
    pub low_impact: f32,
    /// Over how many bands the loudness is spread, 1..24.
    pub fullness: f32,
    /// Share of the loudness below 300 Hz, 300 Hz–2 kHz, above 2 kHz.
    pub shares: [f32; 3],
    /// Loudness against that of loud music filling every band (1 = as loud).
    pub level: f32,
    /// The loudest single band against a band of loud music, averaged over a
    /// twentieth of a second only: is there anything to hear right now? A
    /// note stands out in its band however soft the rest; hiss is spread thin
    /// over all of them.
    pub now: f32,
    /// The same for each range: below 300 Hz, 300 Hz–2 kHz, above 2 kHz.
    pub ranges: [f32; 3],
    /// Loudness struck per second in each range, against the range's loudness
    /// in loud music.
    pub hits: [f32; 3],
}

pub struct Ear {
    /// Bin range of each band.
    bins: [(usize, usize); BANDS],
    /// The ear's sensitivity per band (power factor).
    weight: [f32; BANDS],
    reference: f32,
    /// The loudest of the last ten seconds.
    recent: f32,
    specific: [f32; BANDS],
    started: bool,
    pub out: EarOut,
}

impl Ear {
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        let bin_hz = sample_rate / fft_size as f32;
        let last = fft_size / 2;
        let mut bins = [(0, 0); BANDS];
        let mut weight = [0.0; BANDS];
        let mut lo = 20.0f32;
        for b in 0..BANDS {
            let hi = BARK_EDGES[b];
            let k0 = ((lo / bin_hz).ceil() as usize).min(last);
            let k1 = ((hi / bin_hz).ceil() as usize).clamp(k0, last);
            bins[b] = (k0, k1);
            // Music is heard well above the level A-weighting was made for,
            // where the ear is flatter: half of it.
            weight[b] = 10f32.powf(0.5 * a_weight_db((lo * hi).sqrt()) / 10.0);
            lo = hi;
        }
        Self { bins, weight, reference: REFERENCE_START, recent: 0.0, specific: [0.0; BANDS], started: false, out: EarOut::default() }
    }

    /// The level of loud music, in the units of the power spectrum.
    pub fn reference(&self) -> f32 {
        self.reference
    }

    /// One hop: the power spectrum (a full-scale sine reads about 1).
    pub fn push(&mut self, power: &[f32], dt: f32, gated: bool) {
        let mut excitation = [0.0f32; BANDS];
        for b in 0..BANDS {
            let (k0, k1) = self.bins[b];
            excitation[b] = self.weight[b] * power[k0.min(power.len())..k1.min(power.len())].iter().sum::<f32>();
        }
        let total: f32 = excitation.iter().sum();
        // Masking: each band also hears its neighbours, weaker by the band.
        let (up, down) = (10f32.powf(-SPREAD_UP_DB / 10.0), 10f32.powf(-SPREAD_DOWN_DB / 10.0));
        let mut spread = excitation;
        for b in 1..BANDS {
            spread[b] = spread[b].max(spread[b - 1] * up);
        }
        for b in (0..BANDS - 1).rev() {
            spread[b] = spread[b].max(spread[b + 1] * down);
        }
        if !gated {
            self.recent = if total > self.recent { follow(self.recent, total, dt, 0.5, 0.5) } else { follow(self.recent, total, dt, 10.0, 10.0) };
            let far = self.reference > self.recent * 10f32.powf(REFERENCE_FAR / 10.0);
            let fall = if far { REFERENCE_FALL_FAR } else { REFERENCE_FALL };
            self.reference = (self.reference * 10f32.powf(-fall * dt / 10.0)).max(self.recent);
        }
        let threshold = (self.reference * 10f32.powf(-THRESHOLD_DB / 10.0)).max(1e-12);
        let mut loudness = 0.0;
        let mut squares = 0.0;
        let mut sharp = 0.0;
        let mut struck = 0.0;
        let mut low_struck = 0.0;
        let mut shares = [0.0f32; 3];
        let mut hits = [0.0f32; 3];
        let mut peak = 0.0f32;
        // A band's loudness in loud music spread evenly.
        let nominal = (0.5 + 0.5 * self.reference / BANDS as f32 / threshold).powf(COMPRESSION) - 1.0;
        const SIZE: [f32; 3] = [3.0, 10.0, 11.0];
        for b in 0..BANDS {
            let n = (0.5 + 0.5 * spread[b] / threshold).powf(COMPRESSION) - 1.0;
            let rise = if self.started { (n - self.specific[b]).max(0.0) } else { 0.0 };
            self.specific[b] = n;
            struck += rise;
            if b < 3 {
                low_struck += rise;
            }
            loudness += n;
            peak = peak.max(n);
            squares += n * n;
            let z = b as f32 + 0.5;
            let g = if z <= 15.8 { 1.0 } else { 0.15 * (0.42 * (z - 15.8)).exp() + 0.85 };
            sharp += n * g * z;
            let range = if b < 3 { 0 } else if b < 13 { 1 } else { 2 };
            shares[range] += n;
            hits[range] += rise;
        }
        self.started = true;
        let o = &mut self.out;
        if gated || loudness <= 1e-6 {
            *o = EarOut { loudness: follow(o.loudness, 0.0, dt, 0.2, 0.2), ..EarOut::default() };
            return;
        }
        o.loudness = follow(o.loudness, loudness, dt, 0.2, 0.2);
        o.sharpness = follow(o.sharpness, 0.11 * sharp / loudness, dt, 0.2, 0.2);
        o.impact = follow(o.impact, struck / dt / loudness, dt, 1.0, 1.0);
        o.low_impact = follow(o.low_impact, low_struck / dt / loudness, dt, 1.0, 1.0);
        o.fullness = follow(o.fullness, loudness * loudness / squares.max(1e-12), dt, 0.2, 0.2);
        o.level = follow(o.level, loudness / (nominal * BANDS as f32), dt, 0.2, 0.2);
        o.now = follow(o.now, peak / nominal, dt, 0.05, 0.05);
        for r in 0..3 {
            o.shares[r] = follow(o.shares[r], shares[r] / loudness, dt, 0.2, 0.2);
            o.ranges[r] = follow(o.ranges[r], shares[r] / (nominal * SIZE[r]), dt, 0.2, 0.2);
            o.hits[r] = follow(o.hits[r], hits[r] / dt / (nominal * SIZE[r]), dt, 1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Run by hand: `PROBE=file FROM=s TO=s cargo test --release -- --ignored ear_probe --nocapture`
    #[test]
    #[ignore]
    fn ear_probe() {
        let path = std::env::var("PROBE").unwrap();
        let from: usize = std::env::var("FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let to: usize = std::env::var("TO").ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
        let (samples, ch, rate) = crate::audio::decode_file(std::path::Path::new(&path)).unwrap();
        let stereo: Vec<f32> = samples.chunks(ch).flat_map(|c| [c[0], c[if ch > 1 { 1 } else { 0 }]]).collect();
        let mut a = crate::analysis::Analyzer::new(rate as f32);
        let per = (rate as f32 / crate::analysis::HOP as f32) as usize;
        let mut acc = [0.0f32; 12];
        let mut hop = 0;
        println!("EAR    t level  sharp   low   mid   top  hitL  hitM  hitT voice pulse  flat    dB");
        for chunk in stereo.chunks(2 * crate::analysis::HOP) {
            let mut level = 0.0;
            let (mut pulse, mut flat) = (0.0, 0.0);
            a.push_stereo(chunk, |f, raw| (level, pulse, flat) = (raw.level_db, f.pulse_clarity, raw.flatness));
            let voicing = a.silent_for();
            let e = a.ear.out;
            hop += 1;
            for (i, (s, v)) in acc.iter_mut().zip([e.level, e.sharpness, e.ranges[0], e.ranges[1], e.ranges[2], e.hits[0], e.hits[1], e.hits[2], voicing, pulse, flat, level]).enumerate() {
                // (Column 8, "voice", is now the longest silence in the second.)
                if i == 8 { *s = s.max(v) } else { *s += v / per as f32 }
            }
            if hop % per == 0 {
                let t = hop / per;
                if t >= from && t <= to {
                    println!("EAR {t:4} {:5.2} {:6.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} {:5.1}", acc[0], acc[1], acc[2], acc[3], acc[4], acc[5], acc[6], acc[7], acc[8], acc[9], acc[10], acc[11]);
                }
                acc = [0.0; 12];
            }
        }
    }
}
