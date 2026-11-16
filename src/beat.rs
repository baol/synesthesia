//! Real-time beat tracking, after Stark & Davies' BTrack (2009) and Ellis'
//! dynamic-programming tracker (2007), simplified.
//!
//! Input: one onset-strength value per analysis hop (spectral flux).
//!
//! - **Tempo**: every half second, the autocorrelation of the last 6 s of
//!   onset strength, combed over 1–4 multiples of each lag (a real pulse
//!   repeats) and weighted towards 120 BPM, picks the beat period. It only
//!   changes after the new tempo has been seen twice, so it doesn't flicker.
//! - **Pulse clarity**: how strongly the onset strength repeats at that period
//!   (0 = no pulse, 1 = a clear beat). Music without a steady pulse (rubato
//!   piano, ambient) reads low, and its beats come out weak.
//! - **Phase**: half a beat after each beat, the next one is predicted: for
//!   each possible moment in the coming period, add up the onset strength one,
//!   two, three and four periods before it. The moment whose past lines up
//!   with the strongest onsets wins (gently preferring a steady continuation),
//!   so the tracker settles on the strong beats, not the off-beats, and finds
//!   its way back after a break.
//!
//! Each beat comes with a strength: how strong the onsets around it are
//! (relative to this piece) times the pulse clarity.

use std::collections::VecDeque;

const HISTORY_SECS: f32 = 6.0;
const MIN_BPM: f32 = 70.0;
const MAX_BPM: f32 = 180.0;
const PRIOR_BPM: f32 = 120.0;
/// How many past periods the phase comb looks at.
const COMB_PERIODS: usize = 4;

pub struct BeatTracker {
    dt: f32,
    odf: VecDeque<f32>,
    /// Beat period in hops.
    period: f32,
    candidate: Option<f32>,
    pub clarity: f32,
    since_beat: f32,
    /// Hops until the predicted beat, once predicted.
    due: Option<f32>,
    hops: usize,
    /// 90th percentile of recent onset strength, for beat strengths.
    loud_odf: f32,
}

impl BeatTracker {
    pub fn new(hop_secs: f32) -> Self {
        let len = (HISTORY_SECS / hop_secs) as usize;
        Self {
            dt: hop_secs,
            odf: VecDeque::from(vec![0.0; len]),
            period: 60.0 / PRIOR_BPM / hop_secs,
            candidate: None,
            clarity: 0.0,
            since_beat: 0.0,
            due: None,
            hops: 0,
            loud_odf: 1e-6,
        }
    }

    pub fn tempo_bpm(&self) -> f32 {
        60.0 / (self.period * self.dt)
    }

    fn push(q: &mut VecDeque<f32>, v: f32) {
        q.pop_front();
        q.push_back(v);
    }

    /// Feed one hop's onset strength; returns the beat strength (0..1) if a
    /// beat falls on this hop.
    pub fn process(&mut self, onset_strength: f32) -> Option<f32> {
        let x = onset_strength.max(0.0);
        Self::push(&mut self.odf, x);
        self.hops += 1;
        if self.hops % (0.5 / self.dt).max(1.0) as usize == 0 {
            self.estimate_tempo();
        }

        self.since_beat += 1.0;
        if self.due.is_none() && self.since_beat >= (self.period / 2.0).round() {
            self.due = Some(self.predict());
        }
        let mut beat = None;
        if let Some(d) = self.due.as_mut() {
            *d -= 1.0;
            if *d <= 0.0 {
                beat = Some(self.strength());
            }
        }
        // Safety net: never go more than 1.5 periods without a beat.
        if beat.is_none() && self.since_beat > 1.5 * self.period {
            beat = Some(self.strength());
        }
        if beat.is_some() {
            self.since_beat = 0.0;
            self.due = None;
        }
        beat
    }

    /// Hops from now to the next beat: the moment in the coming period whose
    /// past (1..=4 periods back) holds the strongest onsets, mildly weighted
    /// towards where the current rhythm expects it.
    fn predict(&self) -> f32 {
        let p = self.period;
        let n = self.odf.len() as f32;
        let at = |i: f32| -> f32 {
            // Onset strength at fractional index i (lightly smoothed).
            if i < 1.0 || i > n - 2.0 {
                return 0.0;
            }
            let k = i.round() as usize;
            0.25 * self.odf[k - 1] + 0.5 * self.odf[k] + 0.25 * self.odf[k + 1]
        };
        let expected = (p - self.since_beat).max(1.0);
        let (mut best, mut best_at) = (f32::MIN, expected);
        for m in 1..=p.round() as usize {
            let m = m as f32;
            let now = n - 1.0 + m;
            let comb: f32 = (1..=COMB_PERIODS).map(|k| at(now - k as f32 * p) / k as f32).sum();
            let steady = (-0.5 * ((m - expected) / (0.5 * p)).powi(2)).exp();
            let s = comb * (0.4 + 0.6 * steady);
            if s > best {
                best = s;
                best_at = m;
            }
        }
        best_at
    }

    fn strength(&self) -> f32 {
        // The strongest onset within ±2 hops of now (the last few hops).
        let n = self.odf.len();
        let near = self.odf.range(n - 3..).copied().fold(0.0, f32::max);
        let s = (near / self.loud_odf).min(1.0);
        ((0.3 + 0.7 * s) * self.clarity).clamp(0.0, 1.0)
    }

    fn estimate_tempo(&mut self) {
        let n = self.odf.len();
        // Remove the local mean (adaptive threshold), keep the rises.
        let mut d = vec![0.0f32; n];
        let half = 8usize;
        for i in 0..n {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(n));
            let mean = self.odf.range(a..b).sum::<f32>() / (b - a) as f32;
            d[i] = (self.odf[i] - mean).max(0.0);
        }
        // Centre it, so only repetition (not the average level) correlates.
        let dm = d.iter().sum::<f32>() / n as f32;
        for v in &mut d {
            *v -= dm;
        }
        let mut sorted = self.odf.iter().copied().collect::<Vec<_>>();
        sorted.sort_by(f32::total_cmp);
        self.loud_odf = sorted[n * 9 / 10].max(1e-6);

        let ac = |lag: usize| -> f32 {
            if lag >= n {
                return 0.0;
            }
            (lag..n).map(|i| d[i] * d[i - lag]).sum::<f32>() / (n - lag) as f32
        };
        let energy = ac(0);
        if energy < 1e-12 {
            self.clarity *= 0.5;
            return;
        }
        let (lag_min, lag_max) = ((60.0 / MAX_BPM / self.dt) as usize, (60.0 / MIN_BPM / self.dt).ceil() as usize);
        let prior_lag = 60.0 / PRIOR_BPM / self.dt;
        let (mut best, mut best_lag, mut best_ac) = (f32::MIN, prior_lag, 0.0);
        let mut acs = vec![0.0f32; lag_max + 2];
        for (lag, a) in acs.iter_mut().enumerate().skip(lag_min.saturating_sub(1)) {
            *a = ac(lag);
        }
        for lag in lag_min..=lag_max {
            // A real pulse also repeats at 2, 3 and 4 periods.
            let comb = (1..=4).map(|k| ac(lag * k) / k as f32).sum::<f32>();
            let prior = (-0.5 * ((lag as f32 / prior_lag).log2() / 0.7).powi(2)).exp();
            let s = comb * prior;
            if s > best {
                best = s;
                best_lag = lag as f32;
                best_ac = acs[lag];
            }
        }
        // Parabolic refinement of the peak.
        let l = best_lag as usize;
        if l > lag_min && l < lag_max {
            let (a, b, c) = (acs[l - 1], acs[l], acs[l + 1]);
            let den = a - 2.0 * b + c;
            if den.abs() > 1e-12 {
                best_lag += (0.5 * (a - c) / den).clamp(-0.5, 0.5);
            }
        }
        let r = best_ac / energy;
        let clarity = ((r - 0.05) / 0.35).clamp(0.0, 1.0);
        self.clarity += (clarity - self.clarity) * 0.5;

        // Change tempo only when the new one is seen twice in a row.
        if (best_lag / self.period - 1.0).abs() < 0.06 {
            self.period += (best_lag - self.period) * 0.3;
            self.candidate = None;
        } else if self.candidate.is_some_and(|c| (best_lag / c - 1.0).abs() < 0.06) {
            self.period = best_lag;
            self.candidate = None;
        } else {
            self.candidate = Some(best_lag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOP: f32 = 512.0 / 48_000.0;

    /// Onset strength of a click track: a spike on each beat, a smaller one on
    /// off-beats, and a little noise.
    fn clicks(bpm: f32, secs: f32, offbeats: bool) -> Vec<f32> {
        let n = (secs / HOP) as usize;
        let period = 60.0 / bpm;
        let mut seed = 12345u32;
        (0..n)
            .map(|i| {
                let t = i as f32 * HOP;
                let phase = (t / period).fract();
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.1;
                let on = if phase * period < HOP { 1.0 } else { 0.0 };
                let off = if offbeats && ((phase + 0.5).fract() * period) < HOP { 0.5 } else { 0.0 };
                on + off + noise
            })
            .collect()
    }

    fn run(odf: &[f32]) -> (BeatTracker, Vec<f32>) {
        let mut b = BeatTracker::new(HOP);
        let mut beats = Vec::new();
        for (i, &x) in odf.iter().enumerate() {
            if b.process(x).is_some() {
                beats.push(i as f32 * HOP);
            }
        }
        (b, beats)
    }

    #[test]
    fn finds_the_tempo_and_keeps_time() {
        for bpm in [96.0, 120.0, 140.0] {
            let (b, beats) = run(&clicks(bpm, 20.0, true));
            assert!((b.tempo_bpm() - bpm).abs() < 3.0, "{bpm}: tempo {}", b.tempo_bpm());
            assert!(b.clarity > 0.6, "{bpm}: clarity {}", b.clarity);
            // After settling, beats land on the clicks, one period apart.
            let period = 60.0 / bpm;
            let late: Vec<f32> = beats.iter().copied().filter(|&t| t > 10.0).collect();
            assert!(late.len() as f32 >= 9.0 / period, "{bpm}: only {} beats", late.len());
            for w in late.windows(2) {
                assert!(((w[1] - w[0]) / period - 1.0).abs() < 0.1, "{bpm}: gap {}", w[1] - w[0]);
            }
            for &t in &late {
                let off = ((t / period).fract() * period).min(period - (t / period).fract() * period);
                assert!(off < 0.05, "{bpm}: beat at {t:.3} is {off:.3} s off the click");
            }
        }
    }

    #[test]
    fn noise_has_no_clear_pulse() {
        let mut seed = 99u32;
        let odf: Vec<f32> = (0..(20.0 / HOP) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32
            })
            .collect();
        let (b, _) = run(&odf);
        assert!(b.clarity < 0.3, "clarity {}", b.clarity);
    }
}
