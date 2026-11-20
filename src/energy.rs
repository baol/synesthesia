//! Energy: how intense the music is, as a listener would say it ("the chorus
//! lifts", "the breakdown", "it builds"), and the build-up before a drop.
//!
//! Loudness alone cannot tell: a mastered track holds its level within a
//! decibel or two from verse to chorus, and a club track within a fraction.
//! What changes is the arrangement. So energy is read from four things, each
//! averaged over about a second:
//!
//! - **level**: the four bands summed with the bass counted less and the top
//!   more (roughly as loud as it sounds, not as it measures);
//! - **top**: the level of 2–8 kHz (hats, cymbals, bright guitars and synths,
//!   the "air" that opens up in a chorus);
//! - **low**: the level of the bass band (kick and bass: there, or dropped out);
//! - **activity**: the mean spectral flux (how much is being struck, plucked
//!   and hit; unlike a count of onsets, it keeps rising in a dense mix).
//!
//! Each is placed within **this song's own range** so far: between its 10th and
//! 90th percentile over the last minute and a half (a decaying histogram), with
//! a smallest range so a steady passage is not blown up into drama. That gives
//! the *relative* energy: where this moment sits in the song.
//!
//! The *absolute* energy says what kind of music it is, from readings that do
//! not depend on the playback level: activity, how much of the sound is in the
//! top, how clear the beat is. A piano ballad sits near 0.2, a rock band near
//! 0.9.
//!
//! The song's range is only trusted once the song has arrived (its beat is
//! clear) or has gone on a while without one: an intro that grows is always at
//! the top of what has been heard so far, and is still an intro.
//!
//! Energy = 40% absolute + 60% relative: every song uses 60% of the scale for
//! its own contrasts, placed higher or lower by what kind of song it is.
//!
//! **Build** is the build-up: level, brightness or activity rising steadily
//! for seconds (a crescendo, a riser, a snare roll speeding up), or the low
//! end taken away while the rest plays on. It holds through the gap before a
//! drop and lets go when the low end or the drop arrives.

use crate::features::follow;

/// Weights (dB) of the four bands in the level: the bass sounds less loud
/// than it measures, the top louder.
const LEVEL_WEIGHT_DB: [f32; 4] = [-8.0, -1.0, 2.0, 4.0];
/// Everything is averaged over this long (s) first.
const SMOOTH: f32 = 1.0;
/// The song's range forgets with this time constant (s).
const MEMORY: f32 = 90.0;
/// The smallest range each part is placed in: level, top, low (dB), and
/// activity (as a share of its middle value).
const MIN_SPAN_DB: [f32; 3] = [5.0, 8.0, 10.0];
const MIN_SPAN_ACTIVITY: f32 = 0.35;
/// How much each part counts: level, top, low, activity.
const WEIGHTS: [f32; 4] = [0.3, 0.25, 0.2, 0.25];
/// Share of the absolute energy in the result.
const ABSOLUTE_SHARE: f32 = 0.4;
/// Absolute energy: mean flux from … to, top minus the two middle bands (dB)
/// from … to (against the middle, not the whole: with the bass missing the
/// top would look large), and how clear the beat is.
const ACTIVITY_RANGE: (f32, f32) = (0.03, 0.21);
const TOP_SHARE_RANGE: (f32, f32) = (-17.0, -6.0);
const ABSOLUTE_WEIGHTS: [f32; 3] = [0.45, 0.15, 0.4];
/// The beat counts with the bass band from this far below the whole (dB) …
/// to in full.
const BEAT_LOW: (f32, f32) = (-25.0, -12.0);
/// A song's own range is trusted once its beat is clear, or after this long
/// (s) without one. Until then an intro, however it grows, is not the top.
const RANGE_KNOWN: f32 = 45.0;
/// Build-up: a steady rise counts from … to full, for the level (dB/s),
/// the brightness (octaves/s) and the activity (share of itself per second).
const RISE_LEVEL: (f32, f32) = (0.2, 0.8);
const RISE_BRIGHT: (f32, f32) = (0.03, 0.14);
const RISE_ACTIVITY: (f32, f32) = (0.02, 0.09);
/// The low end missing, on its own, is this much of a build-up.
const LOW_OUT: f32 = 0.3;
/// The trends behind the build-up start this long (s) into a song.
const TREND_START: f32 = 2.0;
/// Silence this long (s) starts a new song.
const NEW_SONG: f32 = 3.0;

fn map(x: f32, (lo, hi): (f32, f32)) -> f32 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// A decaying histogram: where a value has been over the last while.
#[derive(Clone)]
struct Spread {
    bins: Vec<f32>,
    lo: f32,
    step: f32,
}

impl Spread {
    fn new(lo: f32, hi: f32, bins: usize) -> Self {
        Self { bins: vec![0.0; bins], lo, step: (hi - lo) / bins as f32 }
    }

    fn clear(&mut self) {
        self.bins.fill(0.0);
    }

    fn add(&mut self, x: f32, keep: f32) {
        for b in &mut self.bins {
            *b *= keep;
        }
        let i = (((x - self.lo) / self.step) as isize).clamp(0, self.bins.len() as isize - 1) as usize;
        self.bins[i] += 1.0;
    }

    fn percentile(&self, p: f32) -> Option<f32> {
        let total: f32 = self.bins.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut seen = 0.0;
        for (i, b) in self.bins.iter().enumerate() {
            seen += b;
            if seen >= p * total {
                return Some(self.lo + (i as f32 + 0.5) * self.step);
            }
        }
        Some(self.lo + self.step * self.bins.len() as f32)
    }

    /// Where `x` sits in the range heard so far: −0.5 (its bottom) … 0.5 (its
    /// top), 0 in the middle. `min_span` is the smallest range, given the
    /// middle.
    fn place(&self, x: f32, min_span: impl Fn(f32) -> f32) -> f32 {
        let (Some(lo), Some(hi)) = (self.percentile(0.1), self.percentile(0.9)) else { return 0.0 };
        let mid = 0.5 * (lo + hi);
        // The 10th percentile sits at −0.3 and the 90th at 0.4: there is room
        // beyond them, more of it below (a breakdown is further from a verse
        // than a peak is from a chorus).
        let at = (x - mid) / (hi - lo).max(min_span(mid));
        (if at < 0.0 { 0.6 * at } else { 0.8 * at }).clamp(-0.5, 0.5)
    }
}

/// A value averaged over three lengths of time, to tell a steady rise from a jump.
#[derive(Clone, Copy, Default)]
struct Trend {
    short: f32,
    mid: f32,
    long: f32,
}

impl Trend {
    const TIMES: (f32, f32, f32) = (1.0, 3.0, 6.0);

    fn start(x: f32) -> Self {
        Self { short: x, mid: x, long: x }
    }

    fn step(&mut self, x: f32, dt: f32) {
        self.short = follow(self.short, x, dt, Self::TIMES.0, Self::TIMES.0);
        self.mid = follow(self.mid, x, dt, Self::TIMES.1, Self::TIMES.1);
        self.long = follow(self.long, x, dt, Self::TIMES.2, Self::TIMES.2);
    }

    /// Rise per second, if it has been steady: each average lags a ramp by
    /// its own time, so both gaps must be there. A jump shows in one only.
    fn rise(&self) -> f32 {
        ((self.short - self.mid) / (Self::TIMES.1 - Self::TIMES.0)).min((self.mid - self.long) / (Self::TIMES.2 - Self::TIMES.1))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnergyOut {
    /// 0 = silence or the quietest the song gets … 1 = its peak, and a big one.
    pub energy: f32,
    /// The build-up before a drop, 0..1.
    pub build: f32,
    /// What kind of music it is, 0..1 (diagnostics).
    pub absolute: f32,
    /// Where level, top, low and activity sit in the song's range, 0..1 each
    /// (diagnostics).
    pub parts: [f32; 4],
}

pub struct Energy {
    /// Band powers and flux, averaged over [`SMOOTH`].
    power: [f32; 4],
    flux: f32,
    started: bool,
    spreads: [Spread; 4],
    since_add: f32,
    /// Seconds of this song heard so far.
    heard: f32,
    silent: f32,
    absolute: f32,
    /// The clearest beat this song has had, 0..1.
    arrived: f32,
    level: Trend,
    bright: Trend,
    activity: Trend,
    /// How long (s, up to 4) the low end has been missing.
    low_out: f32,
    pub out: EnergyOut,
}

impl Default for Energy {
    fn default() -> Self {
        Self {
            power: [0.0; 4],
            flux: 0.0,
            started: false,
            spreads: [Spread::new(-110.0, 10.0, 240), Spread::new(-110.0, 10.0, 240), Spread::new(-110.0, 10.0, 240), Spread::new(0.0, 0.5, 250)],
            since_add: 0.0,
            heard: 0.0,
            silent: 0.0,
            absolute: 0.0,
            arrived: 0.0,
            level: Trend::default(),
            bright: Trend::default(),
            activity: Trend::default(),
            low_out: 0.0,
            out: EnergyOut::default(),
        }
    }
}

impl Energy {
    /// One hop: the power in the four bands, the spectral flux, the spectral
    /// centroid (Hz) and how clear the beat is (0..1).
    pub fn push(&mut self, band_power: [f32; 4], flux: f32, centroid_hz: f32, pulse: f32, dt: f32, gated: bool) {
        if gated {
            self.silent += dt;
            if self.silent >= NEW_SONG {
                self.started = false;
                for s in &mut self.spreads {
                    s.clear();
                }
            }
            self.out.energy = follow(self.out.energy, 0.0, dt, 0.3, 0.3);
            // The gap before a drop: the build holds its breath for a moment.
            self.out.build = follow(self.out.build, 0.0, dt, 4.0, 4.0);
            self.out.parts = self.out.parts.map(|p| follow(p, 0.0, dt, 0.3, 0.3));
            return;
        }
        let db = |p: f32| 10.0 * (p + 1e-12).log10();
        let bright = centroid_hz.max(50.0).log2();
        if !self.started {
            self.started = true;
            self.power = band_power;
            self.flux = flux;
            self.level = Trend::start(db(level(&band_power)));
            self.bright = Trend::start(bright);
            self.activity = Trend::start(flux);
            self.absolute = f32::NAN;
            self.arrived = 0.0;
            self.low_out = 0.0;
            self.heard = 0.0;
        }
        // A song starting is no build-up: the trends start once it is going.
        if self.heard < TREND_START && self.heard + dt >= TREND_START {
            self.level = Trend::start(db(level(&self.power)));
            self.bright = Trend::start(self.bright.short);
            self.activity = Trend::start(self.flux);
        }
        self.heard += dt;
        self.silent = 0.0;
        for (s, p) in self.power.iter_mut().zip(band_power) {
            *s = follow(*s, p, dt, SMOOTH, SMOOTH);
        }
        self.flux = follow(self.flux, flux, dt, SMOOTH, SMOOTH);
        let values = [db(level(&self.power)), db(self.power[3]), db(self.power[0]), self.flux];

        self.since_add += dt;
        if self.since_add >= 0.1 {
            let keep = (-self.since_add / MEMORY).exp();
            self.since_add = 0.0;
            for (s, v) in self.spreads.iter_mut().zip(values) {
                s.add(v, keep);
            }
        }
        let place: [f32; 4] = std::array::from_fn(|i| {
            self.spreads[i].place(values[i], |mid| if i < 3 { MIN_SPAN_DB[i] } else { (MIN_SPAN_ACTIVITY * mid).max(0.01) })
        });
        let relative = 0.5 + place.iter().zip(WEIGHTS).map(|(p, w)| p * w).sum::<f32>();

        // A beat counts only with a low end under it (kick, bass): held chords
        // changing in time read as a clear pulse too.
        let total: f32 = self.power.iter().sum();
        let pulse = pulse.clamp(0.0, 1.0) * map(values[2] - db(total), BEAT_LOW);
        let absolute = ABSOLUTE_WEIGHTS[0] * map(self.flux, ACTIVITY_RANGE)
            + ABSOLUTE_WEIGHTS[1] * map(values[1] - db(self.power[1] + self.power[2]), TOP_SHARE_RANGE)
            + ABSOLUTE_WEIGHTS[2] * pulse;
        self.absolute = if self.absolute.is_nan() { absolute } else { follow(self.absolute, absolute, dt, 2.5, 2.5) };
        // Where this moment sits in the song's range says little while the
        // range is only an intro: it counts as far as the song has arrived.
        self.arrived = self.arrived.max(pulse);
        let known = self.arrived.max(self.heard / RANGE_KNOWN).min(1.0);

        let o = &mut self.out;
        o.absolute = self.absolute;
        o.parts = place.map(|p| p + 0.5);
        let energy = (ABSOLUTE_SHARE * self.absolute + (1.0 - ABSOLUTE_SHARE) * (0.5 + known * (relative - 0.5))).clamp(0.0, 1.0);
        o.energy = follow(o.energy, energy, dt, 0.15, 0.15);

        // The build-up.
        self.level.step(db(level(&band_power).max(1e-10)), dt);
        self.bright.step(bright, dt);
        self.activity.step(flux, dt);
        let rising = 0.5 * map(self.level.rise(), RISE_LEVEL)
            + 0.5 * map(self.bright.rise(), RISE_BRIGHT)
            + 0.4 * map(self.activity.rise() / self.activity.long.max(0.02), RISE_ACTIVITY);
        // The low end taken away while the rest plays on.
        if place[2] < -0.3 {
            self.low_out = (self.low_out + dt).min(4.0);
        } else if place[2] > -0.05 {
            if self.low_out >= 1.5 {
                // It is back: whatever was building has arrived.
                self.release();
            }
            self.low_out = 0.0;
        }
        let low_out = (self.low_out / 1.5).min(1.0);
        let target = if self.heard < 2.0 * TREND_START { 0.0 } else { (rising + LOW_OUT * low_out).min(1.0) };
        self.out.build = follow(self.out.build, target, dt, 1.5, 1.0);
    }

    /// The drop: the build-up is over.
    pub fn release(&mut self) {
        self.out.build *= 0.2;
        self.level = Trend::start(self.level.short);
        self.bright = Trend::start(self.bright.short);
        self.activity = Trend::start(self.activity.short);
    }
}

fn level(power: &[f32; 4]) -> f32 {
    power.iter().zip(LEVEL_WEIGHT_DB).map(|(p, w)| p * 10f32.powf(w / 10.0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenarios::{self, measure};

    fn scenario(name: &str) -> Vec<measure::Reading> {
        let s = scenarios::all().into_iter().find(|s| s.name == name).unwrap();
        let r = measure::run(&s);
        println!("{}", measure::table(name, &r));
        r
    }

    #[test]
    fn a_ramp_is_a_rise_and_a_jump_is_not() {
        let dt = 0.01;
        let mut ramp = Trend::start(0.0);
        let mut jump = Trend::start(0.0);
        let mut most = 0.0f32;
        for i in 0..4000 {
            ramp.step(i as f32 * dt * 0.7, dt);
            jump.step(4.0, dt);
            most = most.max(jump.rise());
        }
        assert!((ramp.rise() - 0.7).abs() < 0.1, "ramp reads {}", ramp.rise());
        assert!(most < 0.35, "a 4 dB jump reads as {most} dB/s");
    }

    #[test]
    fn the_spread_places_values_in_what_it_has_seen() {
        let mut s = Spread::new(-100.0, 0.0, 200);
        for i in 0..1000 {
            s.add(if i % 2 == 0 { -20.0 } else { -14.0 }, 0.999);
        }
        let span = |_| 4.0;
        assert!(s.place(-14.0, span) > 0.3);
        assert!(s.place(-20.0, span) < -0.25);
        assert!(s.place(-17.0, span).abs() < 0.1);
        // A steady value sits in the middle, and small wobbles stay small.
        let mut steady = Spread::new(-100.0, 0.0, 200);
        for _ in 0..1000 {
            steady.add(-15.0, 0.999);
        }
        assert!(steady.place(-14.0, span).abs() < 0.35);
    }

    #[test]
    fn chorus_above_verse_above_breakdown() {
        let r = scenario("syn-sections");
        let e = |label: &str, skip| measure::mean(&r, label, skip, |r| r.energy);
        let (verse, chorus, breakdown) = (e("verse", 1), e("chorus", 0), e("breakdown", 0));
        assert!(chorus > verse + 0.2, "chorus {chorus}, verse {verse}");
        assert!(verse > breakdown, "verse {verse}, breakdown {breakdown}");
        assert!(chorus > 0.6 && breakdown < 0.35, "chorus {chorus}, breakdown {breakdown}");
    }

    #[test]
    fn each_layer_adds_energy_without_adding_level() {
        let r = scenario("syn-layers");
        let e: Vec<f32> = r.iter().map(|r| r.energy).collect();
        assert!(e[1] > e[0] && e[2] > e[1] + 0.03 && e[3] > e[2] + 0.05, "{e:?}");
        assert!(e[4] < e[3] - 0.25, "back to kick and bass: {e:?}");
    }

    #[test]
    fn steady_music_is_steady() {
        let s = scenarios::all().into_iter().find(|s| s.name == "syn-steady").unwrap();
        let mut a = crate::analysis::Analyzer::new(scenarios::SAMPLE_RATE);
        let dt = crate::analysis::HOP as f32 / scenarios::SAMPLE_RATE;
        let (mut lo, mut hi, mut build, mut hop) = (1.0f32, 0.0f32, 0.0f32, 0usize);
        for chunk in s.render().chunks(crate::analysis::HOP) {
            a.push(chunk, |f, raw| {
                hop += 1;
                if hop as f32 * dt > 10.0 {
                    lo = lo.min(f.energy);
                    hi = hi.max(f.energy);
                    build = build.max(raw.tension_parts[3]);
                }
            });
        }
        assert!(hi - lo < 0.15, "energy wanders from {lo} to {hi}");
        assert!((0.3..0.9).contains(&lo), "energy {lo}..{hi}");
        assert!(build < 0.2, "a build-up of {build} in steady music");
    }

    #[test]
    fn an_intro_is_low_until_the_beat_arrives() {
        let r = scenario("syn-intro");
        let intro = r.iter().filter(|r| r.label == "intro").map(|r| r.energy_end).fold(0.0, f32::max);
        let beat = measure::mean(&r, "beat", 0, |r| r.energy);
        assert!(intro < 0.6 && beat > intro + 0.25, "the most in the intro {intro}, with the beat {beat}");
    }

    #[test]
    fn a_fade_out_loses_energy() {
        let r = scenario("syn-fade");
        assert!(r[1].energy_end < r[0].energy - 0.3, "groove {}, end of fade {}", r[0].energy, r[1].energy_end);
    }

    #[test]
    fn a_build_up_builds_and_the_drop_lets_go() {
        let r = scenario("syn-build-drop");
        let (groove, build, gap, drop) = (&r[0], &r[1], &r[2], &r[3]);
        assert!(groove.parts[3] < 0.2, "build in the groove: {}", groove.parts[3]);
        assert!(build.parts[3] > 0.5, "build in the build-up: {}", build.parts[3]);
        assert!(build.tension > groove.tension + 0.25, "tension: groove {}, build {}", groove.tension, build.tension);
        assert!(gap.tension > 0.5, "tension over the gap: {}", gap.tension);
        assert!(drop.parts[3] < 0.2, "build after the drop: {}", drop.parts[3]);
        assert!(drop.energy > build.energy + 0.05, "energy: build {}, drop {}", build.energy, drop.energy);
    }

    #[test]
    fn a_far_microphone_is_brought_within_reach() {
        // The ear's reference is the level of loud music: a recording 20 dB
        // quieter reads quieter. One much further off than that (a
        // microphone across the room) is got used to in seconds, down to
        // that same distance.
        let s = scenarios::all().into_iter().find(|s| s.name == "syn-sections").unwrap();
        let x = s.render();
        let run = |gain: f32| {
            let mut a = crate::analysis::Analyzer::new(scenarios::SAMPLE_RATE);
            let mut v = Vec::new();
            let quiet: Vec<f32> = x.iter().map(|s| s * gain).collect();
            for chunk in quiet.chunks(crate::analysis::HOP) {
                a.push(chunk, |f, _| v.push(f.energy));
            }
            v
        };
        let (near, far) = (run(0.1), run(0.03));
        let n = near.len().saturating_sub(2000).max(1) as f32;
        let apart = near.iter().zip(&far).skip(2000).map(|(a, b)| (a - b).abs()).sum::<f32>() / n;
        assert!(apart < 0.12, "10 dB further off reads {apart} differently on average");
    }
}
