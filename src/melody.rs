//! The lead line's pitch: where the melody on top of the mix is, read from the
//! analyser's *lead spectrum* (the centre of the stereo image, without
//! percussion and bass; see `LeadSpectrum` in analysis.rs). Whether that line
//! is a voice is asked of the sound classifier (`voice_detect.rs`): measures of
//! the pitch itself (bends, vibrato) could not tell a voice from a piano in a
//! full mix.
//!
//! Per hop:
//!
//! 1. **Whiten** the log spectrum (subtract its local average), so a peak
//!    counts by how much it stands out, not by how loud the timbre is there.
//! 2. **Harmonic summation**: for each candidate pitch from 100 Hz to 1 kHz
//!    (a third of a semitone apart), average the whitened spectrum at its
//!    harmonics (weighted 0.8ⁿ, from 150 Hz to 5 kHz: below is the bass's).
//!    Pitches near the last hop's get a head start, since a line is
//!    continuous.
//! 3. **Refine** the winner on its strongest low harmonic (parabolic peak).
//! 4. The pitch output follows notes that last (~0.1 s), and holds between.

use crate::features::follow;

/// Candidate pitches: 100 Hz × 2^(j/36), j = 0..CANDIDATES (to ~1 kHz).
const CANDIDATES: usize = 120;
const STEPS_PER_OCTAVE: f32 = 36.0;
const F0_MIN: f32 = 100.0;
const HARMONICS: usize = 8;
const MIN_HARMONIC_HZ: f32 = 150.0;
const MAX_HARMONIC_HZ: f32 = 5000.0;
/// Half-width (bins) of the local average the spectrum is whitened against.
const WHITEN_HALF: usize = 12;
/// Peak-to-average salience below this is no line, above the top a clear one.
const VOICED_LO: f32 = 1.6;
const VOICED_HI: f32 = 2.8;
/// The winning pitch's harmonics must also stand this far above their
/// surroundings (log units, ≈ 4 dB), so near-silence never reads as a line.
const MIN_SALIENCE: f32 = 0.45;
/// Head start for pitches near the last hop's: up to this factor, falling off
/// over `CONTINUITY_SEMITONES`.
const CONTINUITY_BONUS: f32 = 0.6;
const CONTINUITY_SEMITONES: f32 = 1.5;

#[derive(Debug, Clone, Copy, Default)]
pub struct MelodyFrame {
    /// The lead pitch this hop, if any.
    pub hz: Option<f32>,
    /// 0..1: how clearly one pitched line stands out right now.
    pub voicing: f32,
}

pub struct MelodyTracker {
    whitened: Vec<f32>,
    log_mag: Vec<f32>,
    /// The note being held: its pitch (log2 Hz) and age (s).
    note: Option<(f32, f32)>,
    /// The lead pitch, 0 = 100 Hz … 1 = 1 kHz (log), held between notes.
    pub pitch: f32,
    pub last: MelodyFrame,
}

impl MelodyTracker {
    pub fn new(bins: usize) -> Self {
        Self { whitened: vec![0.0; bins], log_mag: vec![0.0; bins], note: None, pitch: 0.0, last: MelodyFrame::default() }
    }

    /// Find this hop's lead pitch from a power spectrum (bins `bin_hz` apart).
    pub fn frame(&mut self, power: &[f32], bin_hz: f32) -> MelodyFrame {
        let n = power.len().min(self.log_mag.len());
        for k in 0..n {
            self.log_mag[k] = (1.0 + 1000.0 * power[k].sqrt()).ln();
        }
        // Whiten: what stands out above the local average (a running sum).
        let mut sum: f32 = self.log_mag[..(WHITEN_HALF + 1).min(n)].iter().sum();
        let mut count = (WHITEN_HALF + 1).min(n);
        for k in 0..n {
            if k + WHITEN_HALF + 1 < n {
                sum += self.log_mag[k + WHITEN_HALF + 1];
                count += 1;
            }
            if k > WHITEN_HALF {
                sum -= self.log_mag[k - WHITEN_HALF - 1];
                count -= 1;
            }
            self.whitened[k] = (self.log_mag[k] - sum / count as f32).max(0.0);
        }
        // The whitened level near a frequency (the best of 3 bins, so a
        // slightly sharp or flat harmonic still counts).
        let near = |w: &[f32], hz: f32| -> f32 {
            let k = (hz / bin_hz).round() as usize;
            if k + 1 >= n || k == 0 {
                return 0.0;
            }
            w[k - 1].max(w[k]).max(w[k + 1])
        };
        let prev = self.last.hz.map(|hz| (hz / F0_MIN).log2() * STEPS_PER_OCTAVE);
        let mut best = (0usize, 0.0f32);
        let mut total = 0.0;
        for j in 0..CANDIDATES {
            let f0 = F0_MIN * (j as f32 / STEPS_PER_OCTAVE).exp2();
            let (mut s, mut wsum, mut weight) = (0.0, 0.0, 1.0);
            for h in 1..=HARMONICS {
                let f = f0 * h as f32;
                if f > MAX_HARMONIC_HZ {
                    break;
                }
                if f >= MIN_HARMONIC_HZ {
                    s += weight * near(&self.whitened, f);
                    wsum += weight;
                }
                weight *= 0.8;
            }
            let s = if wsum > 0.0 { s / wsum } else { 0.0 };
            total += s;
            let biased = match prev {
                Some(pj) => {
                    let semis = (j as f32 - pj) / (STEPS_PER_OCTAVE / 12.0);
                    s * (1.0 + CONTINUITY_BONUS * (-semis * semis / (2.0 * CONTINUITY_SEMITONES * CONTINUITY_SEMITONES)).exp())
                }
                None => s,
            };
            if biased > best.1 {
                best = (j, biased);
            }
        }
        let mean = total / CANDIDATES as f32;
        let ratio = if mean > 1e-6 { best.1 / mean } else { 0.0 };
        let voicing = smoothstep(VOICED_LO, VOICED_HI, ratio) * smoothstep(MIN_SALIENCE, 2.0 * MIN_SALIENCE, best.1);
        let hz = (voicing > 0.0).then(|| {
            let f0 = F0_MIN * (best.0 as f32 / STEPS_PER_OCTAVE).exp2();
            refine(power, bin_hz, f0).unwrap_or(f0)
        });
        self.last = MelodyFrame { hz, voicing };
        self.last
    }

    /// Follow notes that last (`dt` seconds per hop).
    pub fn update(&mut self, frame: MelodyFrame, dt: f32, gated: bool) {
        match (frame.hz, gated) {
            (Some(hz), false) if frame.voicing > 0.2 => {
                let p = hz.log2();
                match &mut self.note {
                    // Same note (within a semitone): older.
                    Some((note, age)) if ((p - *note) * 1200.0).abs() < 100.0 => {
                        *note = p;
                        *age += dt;
                    }
                    _ => self.note = Some((p, 0.0)),
                }
                let (_, age) = self.note.unwrap();
                if age > 0.1 {
                    let target = ((p - F0_MIN.log2()) / (1000f32.log2() - F0_MIN.log2())).clamp(0.0, 1.0);
                    self.pitch = follow(self.pitch, target, dt, 0.08, 0.08);
                }
            }
            _ => self.note = None,
        }
    }
}

/// The pitch to a few cents, from the strongest of the first harmonics.
fn refine(power: &[f32], bin_hz: f32, f0: f32) -> Option<f32> {
    let mag = |k: usize| power[k].sqrt();
    let (h, k) = (1..=4)
        .filter_map(|h| {
            let k = (f0 * h as f32 / bin_hz).round() as usize;
            (k >= 2 && k + 2 < power.len()).then(|| {
                let k = (k - 1..=k + 1).max_by(|&a, &b| mag(a).total_cmp(&mag(b))).unwrap();
                (h, k)
            })
        })
        .max_by(|a, b| mag(a.1).total_cmp(&mag(b.1)))?;
    // Parabolic interpolation on log magnitude.
    let (a, b, c) = (mag(k - 1).max(1e-12).ln(), mag(k).max(1e-12).ln(), mag(k + 1).max(1e-12).ln());
    let den = a - 2.0 * b + c;
    let offset = if den.abs() > 1e-9 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
    Some((k as f32 + offset) * bin_hz / h as f32)
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use crate::analysis::{Analyzer, HOP};
    use rand::{Rng, SeedableRng, rngs::StdRng};

    const SR: f32 = 44_100.0;

    /// A harmonic tone: `f0(t)` Hz, harmonics falling as 1/h, amplitude `amp(t)`.
    fn tone(secs: f32, f0: impl Fn(f32) -> f32, amp: impl Fn(f32) -> f32) -> Vec<f32> {
        let n = (secs * SR) as usize;
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                phase = (phase + f0(t) / SR).fract();
                let mut s = 0.0;
                for h in 1..=10 {
                    s += (std::f32::consts::TAU * phase * h as f32).sin() / h as f32;
                }
                0.3 * amp(t) * s
            })
            .collect()
    }

    /// A drum loop: kicks and noisy hats at 120 BPM.
    fn drums(secs: f32, rng: &mut StdRng) -> Vec<f32> {
        let n = (secs * SR) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                let kt = t % 0.5;
                let ht = (t + 0.25) % 0.5;
                let kick = (std::f32::consts::TAU * 55.0 * kt).sin() * (-kt / 0.08).exp();
                let hat = rng.random_range(-1.0..1.0) * (-ht / 0.03).exp();
                0.5 * kick + 0.2 * hat
            })
            .collect()
    }

    /// Run the analyser on (left, right); return the share of the last half's
    /// hops with a lead pitch, and that pitch's median (Hz).
    fn listen(left: &[f32], right: &[f32]) -> (f32, f32) {
        let mut a = Analyzer::new(SR);
        let mut out = Vec::new();
        let inter: Vec<f32> = left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect();
        for chunk in inter.chunks(2 * HOP) {
            a.push_stereo(chunk, |_, raw| out.push(raw.melody_hz));
        }
        let tail = &out[out.len() / 2..];
        let mut hz: Vec<f32> = tail.iter().copied().filter(|&h| h > 0.0).collect();
        hz.sort_by(f32::total_cmp);
        (hz.len() as f32 / tail.len() as f32, hz.get(hz.len() / 2).copied().unwrap_or(0.0))
    }

    fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(x, y)| x + y).collect()
    }

    #[test]
    fn a_sung_line_over_drums_is_found_at_its_pitch() {
        let mut rng = StdRng::seed_from_u64(1);
        let voice = tone(4.0, |t| 220.0 * (40.0 / 1200.0 * (std::f32::consts::TAU * 5.5 * t).sin()).exp2(), |_| 1.0);
        let x = mix(&voice, &drums(4.0, &mut rng));
        let (share, hz) = listen(&x, &x);
        assert!(share > 0.8, "a pitch in {share} of hops");
        assert!((hz / 220.0).log2().abs() < 0.05, "pitch {hz}");
    }

    #[test]
    fn drums_alone_have_no_lead_pitch() {
        let mut rng = StdRng::seed_from_u64(2);
        let x = drums(4.0, &mut rng);
        let (share, _) = listen(&x, &x);
        assert!(share < 0.2, "a pitch in {share} of hops");
    }

    #[test]
    fn the_line_in_the_centre_wins_over_a_louder_instrument_at_the_side() {
        let mut rng = StdRng::seed_from_u64(3);
        let voice = tone(4.0, |t| 300.0 * (40.0 / 1200.0 * (std::f32::consts::TAU * 5.5 * t).sin()).exp2(), |_| 0.6);
        // A guitar-ish line, steady, twice as loud, mostly on the left.
        let guitar = tone(4.0, |t| [196.0, 247.0][((t / 0.8) as usize) % 2], |_| 1.2);
        let kit = drums(4.0, &mut rng);
        let left: Vec<f32> = (0..voice.len()).map(|i| voice[i] + guitar[i] + kit[i]).collect();
        let right: Vec<f32> = (0..voice.len()).map(|i| voice[i] + 0.15 * guitar[i] + kit[i]).collect();
        let (_, hz) = listen(&left, &right);
        assert!((hz / 300.0).log2().abs() < 0.05, "followed {hz} Hz, not the line at 300");
    }

    #[test]
    fn the_pitch_follows_the_line_up_and_down() {
        let low = tone(3.0, |_| 150.0, |_| 1.0);
        let high = tone(3.0, |_| 600.0, |_| 1.0);
        let (_, l) = listen(&low, &low);
        let (_, h) = listen(&high, &high);
        assert!((l / 150.0).log2().abs() < 0.05, "{l}");
        assert!((h / 600.0).log2().abs() < 0.05, "{h}");
    }
}
