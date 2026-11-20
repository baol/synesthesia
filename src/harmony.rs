//! Harmony: which notes are sounding, regardless of octave, and what follows
//! from that: the chord, when it changes, whether it is major or minor, the
//! key, and how far the harmony has moved from where the song lives.
//!
//! The analyser's own FFT (2048 samples, 23 Hz per bin) cannot tell semitones
//! apart below ~400 Hz, where most of a chord sits. So this module keeps its
//! own longer window (8192 samples, ~170 ms, 5.9 Hz per bin at 48 kHz) and
//! looks at it every 4th hop (~43 ms): harmony moves slowly.
//!
//! Per look:
//!
//! 1. **Peaks**: local maxima of the spectrum from 55 Hz to 2 kHz that stand
//!    out from their surroundings (noise and drums mostly don't), with their
//!    frequency refined between bins.
//! 2. **Chroma**: each peak adds to its pitch class (C, C♯, … B), less if it
//!    sits between two semitones or at the edges of the range. Twelve numbers.
//! 3. **Tonal centroid** (Harte, Sandler & Gasser 2006): the chroma as a point
//!    in a 6-D space where fifths and thirds are neighbours, so harmonically
//!    close chords are close points. It gives the hue, and how strong a chord
//!    change is.
//! 4. **Chord**: the major or minor triad whose three notes fit the chroma
//!    best; a chord change is that triad becoming another.
//! 5. **Key**: the chroma of the last ~20 s against the 24 Krumhansl–Kessler
//!    key profiles.
//! 6. **Tension**, the harmony's part of it, from three things musicians do:
//!    - **function**: how much the sounding notes pull *in the key*. Each
//!      note of the scale has a pull ([`PULL_MAJOR`], [`PULL_MINOR`]): none
//!      for the tonic, little for its third and fifth, more for the second,
//!      fourth and sixth, most for the leading note, and more still for the
//!      notes outside the scale. Tension is the average pull of the notes
//!      that are sounding. So the tonic chord is rest, the subdominant a
//!      step away, the dominant (leading note, second) wants home, and a
//!      chord from outside the key pulls hardest. All 24 keys vote, by how
//!      well they fit, so C major and A minor (the same notes) don't flip
//!      the reading.
//!    - **dissonance**: the intervals inside the chord, whatever the key:
//!      semitones and tritones grind, fifths and thirds don't. A dominant
//!      seventh, a diminished chord, a suspension, a cluster.
//!    - **suspense**: how long the harmony has been away from rest. It grows
//!      over seconds while the function stays tense and lets go on arrival.
//!
//!    (Lerdahl's tonal pitch space, Krumhansl & Kessler's probe-tone profiles,
//!    and Farbood's and Herremans & Chew's tension models are the sources.)
//!    Build-ups, the other way to make tension, are in `energy.rs`.
//!
//! Tuning is taken as A = 440 Hz. Overtones are not removed: a note's third
//! harmonic adds a little to the fifth above it.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};

pub const FFT_SIZE: usize = 8192;
/// Look once every this many hops.
const EVERY: usize = 4;
const LO_HZ: f32 = 55.0;
const HI_HZ: f32 = 2000.0;
/// Half-width (bins) of the surroundings a peak is compared with.
const AROUND: usize = 10;
/// A peak counts when it is this far above its surroundings (log units, ≈ 9 dB).
const STAND_OUT: f32 = 1.0;
/// Time constants (s) of the four copies of the chroma: the one that leads,
/// the one read as "now", the one left behind, and the song's.
const FAST: f32 = 0.12;
const MID: f32 = 0.5;
const SLOW: f32 = 0.8;
const LONG: f32 = 20.0;
/// A chord change is the best-fitting triad becoming another one, at most
/// once in this long (s). Its strength comes from how far the tonal centroid
/// is moving (its fast copy from its slow one): this much is a full-strength
/// change. (That movement alone was too restless on real music to pick changes
/// from: half the time above 0.3 on a four-chord song.)
const CHANGE_GAP: f32 = 0.3;
const CHANGE_FULL: f32 = 0.6;
/// How well the best triad must fit (cosine; a flat chroma fits 0.5, a pure
/// triad 1) to read as no chord … a clear chord.
const FIT: (f32, f32) = (0.65, 0.9);
/// Tension. Function: the notes' mean instability in the key (0 = all on
/// the tonic … 1 = all outside the scale) from … to full.
const FUNCTION: (f32, f32) = (0.15, 0.62);
/// How much each note pulls towards resolution, by its distance (semitones)
/// above the tonic. Major: the tonic chord rests (1, 3, 5), the leading note
/// (7) pulls hardest of the scale, the fourth next; the five notes outside
/// the scale more than any inside it.
const PULL_MAJOR: [f32; 12] = [0.0, 1.0, 0.55, 0.9, 0.15, 0.55, 1.0, 0.25, 0.95, 0.35, 0.85, 0.8];
/// Minor: the same idea; both sevenths and both sixths occur, the raised ones
/// (leading note, major sixth) pull more.
const PULL_MINOR: [f32; 12] = [0.0, 0.9, 0.5, 0.15, 0.9, 0.5, 1.0, 0.25, 0.45, 0.8, 0.45, 0.8];
/// A note counts as sounding when it is about as strong as the chord's third
/// strongest note; at this share of that strength it counts half.
const SOUNDING: f32 = 0.7;
/// How sharply the keys' votes follow their fit (smaller = the best key alone).
const KEY_VOTE: f32 = 0.05;
/// How much each interval (1–6 semitones) grinds: semitone, whole tone, minor
/// third, major third, fourth/fifth, tritone.
const GRIND: [f32; 6] = [1.0, 0.5, 0.2, 0.15, 0.05, 0.7];
/// Dissonance: the pairwise grind of the chroma from … to full (a plain triad
/// with its overtones reads about 0.06, a diminished seventh 0.14).
const DISSONANCE: (f32, f32) = (0.05, 0.13);
/// The song's plainest chords: their grind is followed upwards in the first
/// time (s), downwards in the second.
const PLAIN: (f32, f32) = (30.0, 1.5);
/// How well the best key must fit the song (correlation) for the function to
/// count: from not at all … fully.
const KEY_SURE: (f32, f32) = (0.5, 0.75);
/// Suspense: grows to full in this long (s) while the function is above the
/// first value; falls away (in the last value, s) once it is below the second.
const SUSPENSE: (f32, f32, f32, f32) = (12.0, 0.5, 0.35, 0.7);
/// What each part counts: function, dissonance, suspense.
const TENSION_MIX: [f32; 3] = [0.6, 0.4, 0.3];
/// Share of the chroma in its four strongest notes from … to: no pitches to
/// speak of (drums, noise) … clear ones.
const PITCHED: (f32, f32) = (0.5, 0.75);
/// Silence this long (s) starts a new song: the key is forgotten.
const NEW_SONG: f32 = 3.0;

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Krumhansl–Kessler key profiles (how much each scale degree belongs).
const MAJOR_PROFILE: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR_PROFILE: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// Name of a chord or key index: 0–11 major (C … B), 12–23 minor.
pub fn name(index: u8, major: &str, minor: &str) -> String {
    format!("{}{}", NOTE_NAMES[index as usize % 12], if index < 12 { major } else { minor })
}

#[derive(Debug, Clone, Copy)]
pub struct HarmonyOut {
    /// The chroma now (C … B), the strongest pitch class at 1.
    pub chroma: [f32; 12],
    /// The best-fitting triad (0–11 major, 12–23 minor), if any fits.
    pub chord: Option<u8>,
    /// The key (same numbering), once a few seconds have been heard.
    pub key: Option<u8>,
    /// 0 = a minor chord, 1 = a major one, 0.5 = neither or none.
    pub major: f32,
    /// 0 = no chord to be heard (drums, noise, a cluster), 1 = a clear one.
    pub tonality: f32,
    /// The harmony's tension, 0 = at rest: its three parts below, mixed.
    pub tension: f32,
    /// How unstable the notes are in the key (tonic 0 … outside the key 1).
    pub function: f32,
    /// How much the chord's own intervals grind.
    pub dissonance: f32,
    /// How long the harmony has been away from rest.
    pub suspense: f32,
    /// How well the key fits the song (correlation; diagnostics).
    pub key_fit: f32,
    /// Where the harmony sits on the circle of fifths, 0..1 **around**: the
    /// note C alone is 0, G 1/12, D 2/12, …; a chord sits between its notes (C
    /// major ≈ 0.14), and each fifth up adds 1/12. 0.99 and 0.01 are neighbours.
    pub hue: f32,
    /// How far the fast tonal centroid is from the slow one (diagnostics).
    pub movement: f32,
}

impl Default for HarmonyOut {
    fn default() -> Self {
        Self { chroma: [0.0; 12], chord: None, key: None, major: 0.5, tonality: 0.0, tension: 0.0, function: 0.0, dissonance: 0.0, suspense: 0.0, key_fit: 0.0, hue: 0.0, movement: 0.0 }
    }
}

pub struct Harmony {
    sample_rate: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// The last `FFT_SIZE` mono samples, and where the next one goes.
    ring: Vec<f32>,
    pos: usize,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    mag: Vec<f32>,
    log_mag: Vec<f32>,
    /// Running sum of `log_mag` (for the local average).
    prefix: Vec<f32>,
    tick: usize,
    fast: [f32; 12],
    mid: [f32; 12],
    slow: [f32; 12],
    long: [f32; 12],
    /// Seconds of music heard in this song, and of silence since.
    heard: f32,
    silent: f32,
    /// The last chord heard (kept through moments without one), and how long ago
    /// (s) the last change was.
    last_chord: Option<u8>,
    since_change: f32,
    /// Another chord fitting better than the current one, and for how many looks.
    challenger: (u8, u8),
    /// How well each key fits the song's chroma (once a few seconds are heard).
    fits: Option<[f32; 24]>,
    /// The grind this song's plainest chords have (its own "consonant").
    plain: f32,
    pub out: HarmonyOut,
}

impl Harmony {
    pub fn new(sample_rate: f32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let spectrum = fft.make_output_vec();
        let bins = spectrum.len();
        Self {
            sample_rate,
            window: (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos()).collect(),
            ring: vec![0.0; FFT_SIZE],
            pos: 0,
            frame: vec![0.0; FFT_SIZE],
            scratch: fft.make_scratch_vec(),
            spectrum,
            fft,
            mag: vec![0.0; bins],
            log_mag: vec![0.0; bins],
            prefix: vec![0.0; bins + 1],
            tick: 0,
            fast: [0.0; 12],
            mid: [0.0; 12],
            slow: [0.0; 12],
            long: [0.0; 12],
            heard: 0.0,
            silent: 0.0,
            last_chord: None,
            since_change: 0.0,
            challenger: (0, 0),
            fits: None,
            plain: 0.0,
            out: HarmonyOut::default(),
        }
    }

    /// Feed one hop of mono samples (`dt` seconds). Returns the strength
    /// (0.3..1) of a chord change, on the hop it is noticed.
    pub fn push(&mut self, hop: &[f32], dt: f32, gated: bool) -> Option<f32> {
        for &x in hop {
            self.ring[self.pos] = x;
            self.pos = (self.pos + 1) % FFT_SIZE;
        }
        self.tick += 1;
        if self.tick % EVERY != 0 {
            return None;
        }
        let dt = dt * EVERY as f32;
        self.since_change += dt;
        let Some(c) = (if gated { None } else { self.chroma_frame() }) else {
            self.silent += dt;
            if self.silent >= NEW_SONG {
                self.heard = 0.0;
            }
            let k = 1.0 - (-dt / 0.3f32).exp();
            self.out.tonality -= self.out.tonality * k;
            self.out.tension -= self.out.tension * k;
            self.out.function -= self.out.function * k;
            self.out.dissonance -= self.out.dissonance * k;
            self.out.suspense -= self.out.suspense * k;
            self.out.major += (0.5 - self.out.major) * k;
            self.out.chord = None;
            return None;
        };
        self.silent = 0.0;
        if self.heard == 0.0 {
            // A new song: everything starts from its first sound.
            (self.fast, self.mid, self.slow, self.long) = (c, c, c, c);
            self.out.key = None;
            self.fits = None;
            self.plain = f32::NAN;
            self.last_chord = None;
        }
        self.heard += dt;
        let ease = |v: &mut [f32; 12], tau: f32| {
            let k = 1.0 - (-dt / tau).exp();
            for (v, &x) in v.iter_mut().zip(&c) {
                *v += (x - *v) * k;
            }
        };
        ease(&mut self.fast, FAST);
        ease(&mut self.mid, MID);
        ease(&mut self.slow, SLOW);
        // The song's chroma: a plain average at first, then the last ~20 s.
        ease(&mut self.long, self.heard.clamp(dt, LONG));

        let o = &mut self.out;
        let top = self.mid.iter().copied().fold(1e-9, f32::max);
        o.chroma = self.mid.map(|v| v / top);

        // The chord: the triad that fits best, and how well.
        let size = self.mid.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        let fit = |index: u8| {
            let root = index as usize % 12;
            let third = if index < 12 { 4 } else { 3 };
            (self.mid[root] + self.mid[(root + third) % 12] + self.mid[(root + 7) % 12]) / (size * 3f32.sqrt())
        };
        let best = |range: std::ops::Range<u8>| range.map(|i| (i, fit(i))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let (major, minor) = (best(0..12), best(12..24));
        let (mut chord, mut score) = if major.1 >= minor.1 { major } else { minor };
        // The chord stays until another fits clearly better, twice in a row.
        if let Some(current) = o.chord {
            if chord != current {
                self.challenger = if self.challenger.0 == chord { (chord, self.challenger.1 + 1) } else { (chord, 1) };
                if fit(current) * 1.1 > score || self.challenger.1 < 2 {
                    (chord, score) = (current, fit(current));
                }
            } else {
                self.challenger.1 = 0;
            }
        }
        let clear = ((score - FIT.0) / (FIT.1 - FIT.0)).clamp(0.0, 1.0);
        o.chord = (clear > 0.0).then_some(chord);
        let k = 1.0 - (-dt / 0.3f32).exp();
        o.tonality += (clear - o.tonality) * k;
        let lean = if clear > 0.0 { ((major.1 - minor.1) * 3.0).clamp(-0.5, 0.5) } else { 0.0 };
        o.major += (0.5 + lean - o.major) * k;

        // Movement of the tonal centroid: chord changes, tension, hue.
        let (now, behind, ahead) = (centroid(&self.mid), centroid(&self.slow), centroid(&self.fast));
        o.movement = distance(&ahead, &behind);
        if now[0].hypot(now[1]) > 0.05 {
            o.hue = (now[0].atan2(now[1]) / std::f32::consts::TAU).rem_euclid(1.0);
        }
        // A change: another chord than the last one heard. (The first chord of
        // a song is no change.)
        let mut changed = false;
        if let Some(chord) = o.chord {
            if self.last_chord.is_some_and(|last| last != chord) && self.since_change >= CHANGE_GAP {
                changed = true;
                self.since_change = 0.0;
            }
            if changed || self.last_chord.is_none() {
                self.last_chord = Some(chord);
            }
        }

        // The key, from a few seconds in. It moves only to a clearly better one.
        if self.heard >= 4.0 {
            let fits: [f32; 24] = std::array::from_fn(|i| {
                let profile = if i < 12 { &MAJOR_PROFILE } else { &MINOR_PROFILE };
                correlation(&self.long, |pc| profile[(pc + 12 - i % 12) % 12])
            });
            let best = (0..24).max_by(|&a, &b| fits[a].total_cmp(&fits[b])).unwrap();
            if o.key.is_none_or(|k| fits[best] > fits[k as usize] + 0.05) {
                o.key = Some(best as u8);
            }
            self.fits = Some(fits);
        }

        // Tension.
        // Which notes are sounding: the chord's own notes count about equally
        // (a loud bass does not make its note count double), the overtones
        // and passing notes around them hardly.
        let whole = self.mid.iter().sum::<f32>().max(1e-9);
        let mut sorted = self.mid.map(|v| v / whole);
        sorted.sort_by(|a, b| b.total_cmp(a));
        let pitched = ((sorted[..4].iter().sum::<f32>() - PITCHED.0) / (PITCHED.1 - PITCHED.0)).clamp(0.0, 1.0);
        let half = (SOUNDING * sorted[2] * whole).max(1e-6);
        let sounding = self.mid.map(|v| v.powi(4) / (v.powi(4) + half.powi(4)));
        let total = sounding.iter().sum::<f32>().max(1e-9);
        let now: [f32; 12] = sounding.map(|v| v / total);
        let function = self.fits.map_or(0.0, |fits| {
            let best = fits.iter().copied().fold(f32::MIN, f32::max);
            o.key_fit = best;
            let (mut sum, mut votes) = (0.0, 0.0);
            for (i, fit) in fits.iter().enumerate() {
                let vote = ((fit - best) / KEY_VOTE).exp();
                if vote < 0.01 {
                    continue;
                }
                let unstable: f32 = (0..12).map(|pc| now[pc] * instability(i, pc)).sum();
                sum += vote * unstable;
                votes += vote;
            }
            // Without a clear key there is no home to be away from.
            let sure = ((best - KEY_SURE.0) / (KEY_SURE.1 - KEY_SURE.0)).clamp(0.0, 1.0);
            sure * ((sum / votes - FUNCTION.0) / (FUNCTION.1 - FUNCTION.0)).clamp(0.0, 1.0)
        });
        let mut grind = 0.0;
        for a in 0..12 {
            for b in a + 1..12 {
                let apart = (b - a).min(12 - (b - a));
                grind += now[a] * now[b] * GRIND[apart - 1];
            }
        }
        // Against the song's own plainest chords: a dense mix or a song all in
        // seventh chords grinds all the time, and that is its normal.
        if pitched > 0.5 {
            self.plain = if self.plain.is_nan() { grind } else { crate::features::follow(self.plain, grind, dt, PLAIN.0, PLAIN.1) };
        }
        let floor = if self.plain.is_nan() { DISSONANCE.0 } else { self.plain.max(DISSONANCE.0) };
        let dissonance = ((grind - floor) / (DISSONANCE.1 - DISSONANCE.0)).clamp(0.0, 1.0);
        let k = 1.0 - (-dt / 0.25f32).exp();
        o.function += (function * pitched - o.function) * k;
        o.dissonance += (dissonance * pitched - o.dissonance) * k;
        if o.function > SUSPENSE.1 {
            o.suspense = (o.suspense + dt / SUSPENSE.0).min(1.0);
        } else if o.function < SUSPENSE.2 {
            o.suspense -= o.suspense * (1.0 - (-dt / SUSPENSE.3).exp());
        }
        o.tension = (TENSION_MIX[0] * o.function + TENSION_MIX[1] * o.dissonance + TENSION_MIX[2] * o.suspense).min(1.0);
        changed.then(|| (0.3 + 0.7 * o.movement / CHANGE_FULL).min(1.0))
    }

    /// This window's chroma (summing to 1), or `None` if nothing stands out.
    fn chroma_frame(&mut self) -> Option<[f32; 12]> {
        for i in 0..FFT_SIZE {
            self.frame[i] = self.ring[(self.pos + i) % FFT_SIZE] * self.window[i];
        }
        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .expect("fft buffer sizes are fixed");
        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        // Scaled so a full-scale sine reads about 1.
        let norm = 4.0 / FFT_SIZE as f32;
        let top = ((HI_HZ / bin_hz) as usize + AROUND + 2).min(self.spectrum.len());
        for k in 0..top {
            let mag = self.spectrum[k].norm() * norm;
            self.mag[k] = mag;
            self.log_mag[k] = (1.0 + 1000.0 * mag).ln();
            self.prefix[k + 1] = self.prefix[k] + self.log_mag[k];
        }
        let mut chroma = [0.0f32; 12];
        for k in ((LO_HZ / bin_hz) as usize).max(2)..top - 1 {
            let (a, b, c) = (self.log_mag[k - 1], self.log_mag[k], self.log_mag[k + 1]);
            if !(b > a && b >= c) {
                continue;
            }
            let (i0, i1) = (k.saturating_sub(AROUND), (k + AROUND + 1).min(top));
            let around = (self.prefix[i1] - self.prefix[i0]) / (i1 - i0) as f32;
            if b - around < STAND_OUT {
                continue;
            }
            // The peak's place between bins (a parabola through three points).
            let bend = a - 2.0 * b + c;
            let offset = if bend < 0.0 { (0.5 * (a - c) / bend).clamp(-0.5, 0.5) } else { 0.0 };
            let hz = (k as f32 + offset) * bin_hz;
            if !(LO_HZ..HI_HZ).contains(&hz) {
                continue;
            }
            let note = 69.0 + 12.0 * (hz / 440.0).log2();
            let nearest = note.round();
            // Full weight on a semitone, none halfway between two; fading in
            // over the lowest octave and out over the highest.
            let in_tune = (std::f32::consts::PI * (note - nearest)).cos().powi(2);
            let register = (hz / LO_HZ).log2().min((HI_HZ / hz).log2()).clamp(0.0, 1.0);
            chroma[(nearest as i32).rem_euclid(12) as usize] += self.mag[k].sqrt() * in_tune * register;
        }
        let total: f32 = chroma.iter().sum();
        (total > 1e-4).then(|| chroma.map(|v| v / total))
    }
}

/// The tonal centroid of a chroma: circle of fifths, of minor thirds, of major
/// thirds (two coordinates each; the last counts half).
fn centroid(chroma: &[f32; 12]) -> [f32; 6] {
    use std::f32::consts::PI;
    let total = chroma.iter().sum::<f32>().max(1e-9);
    let mut c = [0.0f32; 6];
    for (pc, &v) in chroma.iter().enumerate() {
        let l = pc as f32;
        let v = v / total;
        for (i, (angle, radius)) in [(7.0 * PI / 6.0, 1.0), (3.0 * PI / 2.0, 1.0), (2.0 * PI / 3.0, 0.5)].into_iter().enumerate() {
            c[2 * i] += v * radius * (l * angle).sin();
            c[2 * i + 1] += v * radius * (l * angle).cos();
        }
    }
    c
}

/// How much pitch class `pc` pulls in key `key` (0–11 major, 12–23 minor).
fn instability(key: usize, pc: usize) -> f32 {
    let pull = if key < 12 { &PULL_MAJOR } else { &PULL_MINOR };
    pull[(pc + 12 - key % 12) % 12]
}

fn distance(a: &[f32; 6], b: &[f32; 6]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

/// Pearson correlation of a chroma with a profile.
fn correlation(chroma: &[f32; 12], profile: impl Fn(usize) -> f32) -> f32 {
    let (mc, mp) = (chroma.iter().sum::<f32>() / 12.0, (0..12).map(&profile).sum::<f32>() / 12.0);
    let (mut num, mut dc, mut dp) = (0.0, 0.0, 0.0);
    for (pc, &c) in chroma.iter().enumerate() {
        let (x, y) = (c - mc, profile(pc) - mp);
        num += x * y;
        dc += x * x;
        dp += y * y;
    }
    num / (dc * dp).sqrt().max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    const HOP: usize = 512;

    fn hz(note: i32) -> f32 {
        440.0 * 2f32.powf((note - 69) as f32 / 12.0)
    }

    /// Notes (MIDI numbers) played together, each with four overtones.
    fn chord(notes: &[i32], secs: f32) -> Vec<f32> {
        (0..(SR * secs) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                notes
                    .iter()
                    .map(|&n| (1..=4).map(|h| 0.1 / h as f32 * (std::f32::consts::TAU * hz(n) * h as f32 * t).sin()).sum::<f32>())
                    .sum()
            })
            .collect()
    }

    fn noise(secs: f32) -> Vec<f32> {
        let mut seed = 7u32;
        (0..(SR * secs) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                0.3 * ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
            })
            .collect()
    }

    /// Runs the samples; returns the last output and the times of chord changes.
    fn run(h: &mut Harmony, samples: &[f32]) -> (HarmonyOut, Vec<f32>) {
        let dt = HOP as f32 / SR;
        let mut changes = Vec::new();
        for (i, hop) in samples.chunks_exact(HOP).enumerate() {
            if h.push(hop, dt, false).is_some() {
                changes.push(i as f32 * dt);
            }
        }
        (h.out, changes)
    }

    const C_MAJOR: [i32; 3] = [60, 64, 67];
    const F_MAJOR: [i32; 3] = [60, 65, 69];
    const G_MAJOR: [i32; 3] = [59, 62, 67];
    const A_MINOR: [i32; 3] = [57, 60, 64];

    #[test]
    fn a_triad_reads_as_its_chord() {
        for (notes, index, major) in [(C_MAJOR, 0, true), (A_MINOR, 12 + 9, false), (G_MAJOR, 7, true), ([50, 53, 57], 12 + 2, false)] {
            let (out, changes) = run(&mut Harmony::new(SR), &chord(&notes, 2.0));
            assert_eq!(out.chord, Some(index), "{notes:?}: chroma {:?}", out.chroma);
            assert!(out.tonality > 0.7, "{notes:?}: tonality {}", out.tonality);
            assert!(if major { out.major > 0.8 } else { out.major < 0.2 }, "{notes:?}: major {}", out.major);
            assert!(changes.is_empty(), "{notes:?}: a held chord changed at {changes:?}");
            // The three notes are the three strongest pitch classes.
            let mut order: Vec<usize> = (0..12).collect();
            order.sort_by(|&a, &b| out.chroma[b].total_cmp(&out.chroma[a]));
            let mut top: Vec<usize> = order[..3].to_vec();
            top.sort();
            let mut want: Vec<usize> = notes.iter().map(|n| (n % 12) as usize).collect();
            want.sort();
            assert_eq!(top, want, "{notes:?}: chroma {:?}", out.chroma);
        }
    }

    #[test]
    fn low_notes_land_on_the_right_semitone() {
        // E2 (82 Hz) and F2 (87 Hz) are less than one FFT bin apart.
        for note in [40, 41, 45] {
            let (out, _) = run(&mut Harmony::new(SR), &chord(&[note], 1.5));
            let top = (0..12).max_by(|&a, &b| out.chroma[a].total_cmp(&out.chroma[b])).unwrap();
            assert_eq!(top, (note % 12) as usize, "note {note}: chroma {:?}", out.chroma);
        }
    }

    #[test]
    fn a_chord_change_is_noticed_once_and_soon() {
        let mut samples = chord(&C_MAJOR, 2.0);
        samples.extend(chord(&A_MINOR, 2.0));
        samples.extend(chord(&F_MAJOR, 2.0));
        let (_, changes) = run(&mut Harmony::new(SR), &samples);
        assert_eq!(changes.len(), 2, "changes at {changes:?}");
        for (at, want) in changes.iter().zip([2.0, 4.0]) {
            assert!((0.0..0.7).contains(&(at - want)), "change at {at}, chord changed at {want}");
        }
    }

    #[test]
    fn the_key_of_a_cadence() {
        let mut samples = Vec::new();
        for _ in 0..4 {
            for notes in [C_MAJOR, F_MAJOR, G_MAJOR, C_MAJOR] {
                samples.extend(chord(&notes, 1.0));
            }
        }
        let (out, _) = run(&mut Harmony::new(SR), &samples);
        assert_eq!(out.key, Some(0), "long chroma key");
        // Back home on C: little tension.
        assert!(out.tension < 0.5, "tension {}", out.tension);
    }

    #[test]
    fn a_fifth_up_is_a_twelfth_around() {
        let hue = |notes: &[i32]| run(&mut Harmony::new(SR), &chord(notes, 2.0)).0.hue;
        let (c, g, d) = (hue(&C_MAJOR), hue(&G_MAJOR), hue(&[62, 66, 69]));
        for (step, name) in [(g - c, "C to G"), (d - g, "G to D")] {
            assert!((step - 1.0 / 12.0).abs() < 0.03, "{name}: {step} (hues {c} {g} {d})");
        }
    }

    #[test]
    fn noise_is_no_chord() {
        let (out, changes) = run(&mut Harmony::new(SR), &noise(4.0));
        assert!(out.tonality < 0.2, "tonality {}", out.tonality);
        assert!((out.major - 0.5).abs() < 0.2, "major {}", out.major);
        assert!(changes.len() <= 1, "changes at {changes:?}");
    }

    #[test]
    fn silence_starts_a_new_song() {
        let mut h = Harmony::new(SR);
        run(&mut h, &chord(&C_MAJOR, 6.0));
        assert_eq!(h.out.key, Some(0));
        let dt = HOP as f32 / SR;
        for _ in 0..(4.0 / dt) as usize {
            h.push(&[0.0; HOP], dt, true);
        }
        assert!(h.out.tonality < 0.05 && h.out.chord.is_none());
        // A (major) for a few seconds: the old key is gone.
        run(&mut h, &chord(&[57, 61, 64], 6.0));
        assert_eq!(h.out.key, Some(9));
    }
}

/// Tension on the synthetic chord progressions of `scenarios.rs`: what the
/// harmony does, read section by section.
#[cfg(test)]
mod progressions {
    use crate::scenarios::{self, measure};

    fn read(name: &str) -> Vec<measure::Reading> {
        let s = scenarios::all().into_iter().find(|s| s.name == name).unwrap();
        let r = measure::run(&s);
        println!("{}", measure::table(name, &r));
        r
    }

    /// Mean tension of a chord, leaving out its first time (the key is still
    /// being found then).
    fn t(r: &[measure::Reading], label: &str) -> f32 {
        measure::mean(r, label, 1, |r| r.tension)
    }

    /// The same, counting every time (for chords that come after the key is set).
    fn t0(r: &[measure::Reading], label: &str) -> f32 {
        measure::mean(r, label, 0, |r| r.tension)
    }

    #[test]
    fn tonic_subdominant_dominant() {
        let r = read("syn-cadence");
        let (i, iv, v) = (t(&r, "I"), t(&r, "IV"), t(&r, "V"));
        assert!(i < 0.15, "I {i}");
        assert!(iv > i + 0.1 && v > iv + 0.2, "I {i}, IV {iv}, V {v}");
    }

    #[test]
    fn the_same_in_every_key() {
        let cadence = scenarios::all().into_iter().find(|s| s.name == "syn-cadence").unwrap();
        let home = measure::run(&cadence);
        for up in [1, 6, 10] {
            let r = measure::run(&cadence.clone().transposed(up));
            for label in ["I", "IV", "V"] {
                let (here, there) = (t(&home, label), t(&r, label));
                assert!((here - there).abs() < 0.12, "{label}: {here} in C, {there} {up} semitones up");
            }
        }
    }

    #[test]
    fn a_seventh_on_the_dominant_adds_tension() {
        let r = read("syn-seventh");
        assert!(t(&r, "V7") > t(&r, "V") + 0.15, "V {}, V7 {}", t(&r, "V"), t(&r, "V7"));
    }

    #[test]
    fn two_five_one() {
        let r = read("syn-two-five-one");
        let (i, ii, v) = (t(&r, "I"), t(&r, "ii7"), t(&r, "V7"));
        assert!(v > ii + 0.1 && v > i + 0.2, "I {i}, ii7 {ii}, V7 {v}");
    }

    #[test]
    fn a_minor_key() {
        let r = read("syn-minor");
        let (i, iv, v) = (t(&r, "i"), t(&r, "iv"), t(&r, "V"));
        assert!(i < 0.15 && iv > i + 0.1 && v > iv + 0.15, "i {i}, iv {iv}, V {v}");
    }

    #[test]
    fn the_four_chord_loop_with_and_without_a_band() {
        let (plain, band) = (read("syn-pop-loop"), read("syn-band"));
        for r in [&plain, &band] {
            let (i, v, vi, iv) = (t(r, "I"), t(r, "V"), t(r, "vi"), t(r, "IV"));
            assert!(i < vi - 0.08 && i < iv - 0.08, "I {i}, vi {vi}, IV {iv}");
            assert!(v > vi + 0.15 && v > iv + 0.15, "V {v}, vi {vi}, IV {iv}");
        }
        // Drums, bass and a melody don't change the reading much.
        for label in ["I", "V", "vi", "IV"] {
            assert!((t(&plain, label) - t(&band, label)).abs() < 0.15, "{label}: {} alone, {} with the band", t(&plain, label), t(&band, label));
        }
    }

    #[test]
    fn a_deceptive_cadence_resolves_only_partly() {
        let r = read("syn-deceptive");
        let (i, v, vi) = (t(&r, "I"), t(&r, "V"), t(&r, "vi"));
        assert!(vi < v - 0.15 && vi > i + 0.08, "I {i}, V {v}, vi {vi}");
    }

    #[test]
    fn chords_from_outside_the_key() {
        let r = read("syn-borrowed");
        let (iv, v, flat6, flat7) = (t0(&r, "IV"), t0(&r, "V"), t0(&r, "bVI"), t0(&r, "bVII"));
        assert!(flat6 > v && flat7 > v && flat6 > iv + 0.3, "IV {iv}, V {v}, bVI {flat6}, bVII {flat7}");
        let r = read("syn-secondary");
        assert!(t0(&r, "V/V") > t0(&r, "ii") + 0.15, "ii {}, V/V {}", t0(&r, "ii"), t0(&r, "V/V"));
    }

    #[test]
    fn tense_versions_of_the_home_chord() {
        let r = read("syn-colours");
        let i = t(&r, "I");
        let (sus, dim, aug, cluster) = (t0(&r, "sus4"), t0(&r, "dim"), t0(&r, "aug"), t0(&r, "cluster"));
        assert!(sus > i + 0.1 && aug > i + 0.2, "I {i}, sus4 {sus}, aug {aug}");
        assert!(dim > 0.6 && cluster > 0.6 && dim > aug && cluster > aug, "dim {dim}, aug {aug}, cluster {cluster}");
    }

    #[test]
    fn home_is_rest() {
        let r = read("syn-pedal");
        assert!(r[0].tension < 0.1 && r[0].tension_end < 0.1, "{} … {}", r[0].tension, r[0].tension_end);
    }

    #[test]
    fn a_dominant_left_hanging_grows_and_then_resolves() {
        let r = read("syn-hanging");
        let hanging = r.iter().find(|r| r.label == "V7").unwrap();
        assert!(hanging.parts[2] > 0.5, "suspense {}", hanging.parts[2]);
        assert!(hanging.tension_end > t0(&r, "V") + 0.12, "V {}, V7 at its end {}", t0(&r, "V"), hanging.tension_end);
        let home = r.last().unwrap();
        assert!(home.label == "I" && home.tension < 0.2, "after resolving: {}", home.tension);
    }

    #[test]
    fn a_new_key_becomes_home() {
        let r = read("syn-modulation");
        let new_home: Vec<f32> = r.iter().filter(|r| r.label == "I'").map(|r| r.tension).collect();
        assert!(new_home[0] > 0.4, "F sharp right after C major: {new_home:?}");
        assert!(new_home[new_home.len() - 1] < 0.15, "F sharp after half a minute: {new_home:?}");
    }
}

