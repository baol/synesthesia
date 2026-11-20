//! Aspects: what a listener would say about the music, one question each.
//!
//! Read from the hearing model (`ear.rs`), not from raw power:
//!
//! - **loudness**: how loud does it sound?
//! - **fullness**: how much of the band is playing? Five layers, each there
//!   or not: the beat (drums), the bass, the body (chords, guitars, keys:
//!   300 Hz–2 kHz), the top (cymbals, air: above 2 kHz) and a lead (a voice
//!   or a solo: whoever is loud in the middle of the stereo image).
//! - **drive**: how much does it make you move? A clear beat that hits hard,
//!   faster and busier counting more.
//! - **heaviness**: how heavy and dirty is the sound? A distorted, noisy
//!   upper range on a full low end.
//! - **solo**: an instrument in front, with no voice.
//!
//! And moments, which fire once and fade:
//!
//! - **entrance**: a layer comes in;
//! - **everyone in**: the band is suddenly (nearly) complete;
//! - **exit**: layers leave (a breakdown);
//! - **release**: tension lets go.
//!
//! Two things are mixes of these and measure nothing of their own:
//!
//! - **energy** = 40% fullness + 30% drive + 30% loudness, plus the *lift* of
//!   an entrance, which fades slowly as one gets used to the new sound;
//! - **holding back**, a part of tension: a beat going on with layers missing
//!   makes one wait for them, more the longer it lasts.
//!
//! The layers are inferred from where the loudness is and whether it is
//! struck; nothing is separated. A guitar and a saxophone are the same lead.

use crate::ear::EarOut;
use crate::features::follow;

/// Loudness: the ear's level from … to (1 = loud music filling every band).
const LOUD: (f32, f32) = (0.1, 0.75);
/// The beat's hits (loudness struck per second, low range plus half the top)
/// from … to.
const HITS: (f32, f32) = (1.2, 5.0);
/// A layer is there from … to: the beat (drive), the bass, the body, the top
/// (each range's loudness against loud music).
const BEAT_THERE: (f32, f32) = (0.15, 0.5);
const BASS_THERE: (f32, f32) = (0.2, 0.65);
const BODY_THERE: (f32, f32) = (0.25, 0.65);
const TOP_THERE: (f32, f32) = (0.1, 0.45);
/// How much each layer counts in fullness: beat, bass, body, top, lead.
const LAYER_WEIGHT: [f32; 5] = [0.2, 0.2, 0.2, 0.2, 0.2];
/// Someone is in front from … to: the middle of the stereo image, 300 Hz–
/// 3 kHz, this far (dB) under loud music. (The voice classifier alone hears
/// half a voice in guitars and keyboards, and steps too little when one
/// comes in.)
const LEAD_THERE: (f32, f32) = (-18.0, -8.0);
/// Energy: fullness, drive, loudness.
const ENERGY_MIX: [f32; 3] = [0.4, 0.3, 0.3];
/// A layer comes in: this far above its average of the last seconds, and
/// this much there.
const ENTER_JUMP: f32 = 0.35;
const ENTER_LEVEL: f32 = 0.6;
/// No moments in the first seconds of a song (the beat takes that long to
/// be found), and a layer that came in cannot come in again for a while.
const SETTLE: f32 = 5.0;
const ENTER_REST: f32 = 6.0;
/// Layers are compared with their average over this long (s).
const LAYER_MEMORY: f32 = 6.0;
/// Everyone in: fullness past the first having been under the second.
const EVERYONE: (f32, f32) = (0.8, 0.62);
/// …for at least this long (s): a bar's rest is not a breakdown.
const THIN_FOR: f32 = 2.0;
/// Layers leaving: fullness this far under its average.
const EXIT_FALL: f32 = 0.28;
/// Tension lets go: this far under what it was two seconds ago.
const RELEASE_FALL: f32 = 0.3;
/// The lift of an entrance fades over this long (s), and is at most this.
const LIFT_FADE: f32 = 40.0;
const LIFT_MOST: f32 = 0.3;
/// Holding back grows over this long (s) and is this much of tension at most.
const HOLD_GROW: f32 = 12.0;
const HOLD_SHARE: f32 = 0.5;
/// A solo: the lead line moving by this many semitones per second … to.
const SOLO_MOVES: (f32, f32) = (4.0, 14.0);
/// Moments fade over this long (s).
const MOMENT_FADE: f32 = 0.5;

fn map(x: f32, (lo, hi): (f32, f32)) -> f32 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// What the aspects need besides the ear.
#[derive(Debug, Clone, Copy, Default)]
pub struct Heard {
    /// How clear the beat is, tempo (60 BPM = 0 … 180 = 1), onsets per second (6/s = 1).
    pub pulse: f32,
    pub tempo: f32,
    pub density: f32,
    /// A distorted, noisy upper range, 0..1.
    pub distorted: f32,
    /// A voice (singing or rap), 0..1.
    pub voice: f32,
    /// The middle of the stereo image, 300 Hz–3 kHz, against loud music (dB).
    pub centre_db: f32,
    /// The lead line's pitch (Hz), 0 if none.
    pub lead_hz: f32,
    /// Tension from the harmony and the build-up, 0..1, and the build-up alone.
    pub tension: f32,
    pub build: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AspectsOut {
    pub loudness: f32,
    pub fullness: f32,
    pub drive: f32,
    pub heaviness: f32,
    pub solo: f32,
    pub energy: f32,
    /// Tension with holding back added.
    pub tension: f32,
    pub entrance: f32,
    pub everyone_in: f32,
    pub exit: f32,
    pub release: f32,
    /// The layers: beat, bass, body, top, lead (diagnostics).
    pub layers: [f32; 5],
    /// Holding back and the lift (diagnostics).
    pub hold: f32,
    pub lift: f32,
}

#[derive(Default)]
pub struct Aspects {
    heard: f32,
    silent: f32,
    /// Each layer and fullness: now (over 0.4 s) and its average.
    short: [f32; 6],
    long: [f32; 6],
    /// Each layer may enter again once it has settled.
    armed: [bool; 5],
    rest: [f32; 5],
    /// Fullness has been low since everyone was last in.
    was_thin: bool,
    thin_for: f32,
    exit_armed: bool,
    tension_past: f32,
    release_wait: f32,
    last_hz: f32,
    front: f32,
    moves: f32,
    voiceless: f32,
    pub out: AspectsOut,
}

impl Aspects {
    pub fn push(&mut self, ear: &EarOut, h: &Heard, dt: f32, gated: bool) {
        let o = &mut self.out;
        for m in [&mut o.entrance, &mut o.everyone_in, &mut o.exit, &mut o.release] {
            *m *= (-dt / MOMENT_FADE).exp();
        }
        o.lift *= (-dt / LIFT_FADE).exp();
        if gated {
            self.silent += dt;
            if self.silent >= 3.0 {
                // A new song.
                self.heard = 0.0;
                o.lift = 0.0;
            }
            for v in [&mut o.loudness, &mut o.fullness, &mut o.drive, &mut o.heaviness, &mut o.solo, &mut o.energy] {
                *v = follow(*v, 0.0, dt, 0.3, 0.3);
            }
            o.layers = o.layers.map(|l| follow(l, 0.0, dt, 0.3, 0.3));
            // The gap before a drop: what was held stays held for a moment.
            o.hold = follow(o.hold, 0.0, dt, 4.0, 4.0);
            o.tension = h.tension + (1.0 - h.tension) * HOLD_SHARE * o.hold;
            return;
        }
        self.silent = 0.0;
        self.heard += dt;

        let loudness = map(ear.level, LOUD);
        let hits = map(ear.hits[0] + 0.5 * ear.hits[2], HITS);
        let drive = h.pulse * (0.8 * hits * (0.7 + 0.3 * h.tempo) + 0.2 * h.density.min(1.0));
        let bass = map(ear.ranges[0], BASS_THERE);
        let heaviness = h.distorted * (0.5 + 0.5 * bass);

        // A solo: the lead line moving, with no voice for a while.
        let moved = if h.lead_hz > 0.0 && self.last_hz > 0.0 { (12.0 * (h.lead_hz / self.last_hz).log2()).abs().min(12.0) } else { 0.0 };
        self.last_hz = h.lead_hz;
        self.moves = follow(self.moves, moved / dt, dt, 2.0, 2.0);
        self.voiceless = if h.voice < 0.3 { self.voiceless + dt } else { 0.0 };
        let body = map(ear.ranges[1], BODY_THERE);
        let front = follow(self.front, map(h.centre_db, LEAD_THERE), dt, 0.3, 0.3);
        self.front = front;
        let solo = if self.voiceless > 2.0 { map(self.moves, SOLO_MOVES) * (0.5 + 0.5 * front) } else { 0.0 };

        self.out.solo = follow(self.out.solo, solo, dt, 0.5, 1.5);
        // The drums are there when something hits, even on a beat too loose
        // to be sure of (drive wants the beat clear; the layer does not).
        let beat = map(hits * (0.4 + 0.6 * h.pulse) * (0.7 + 0.3 * h.tempo), BEAT_THERE);
        let layers = [beat, bass, body, map(ear.ranges[2], TOP_THERE), front];
        let fullness: f32 = layers.iter().zip(LAYER_WEIGHT).map(|(l, w)| l * w).sum();

        // Moments: each layer and fullness against its own recent average.
        let first = self.heard <= dt;
        for i in 0..6 {
            let x = if i < 5 { layers[i] } else { fullness };
            if first {
                (self.short[i], self.long[i]) = (x, x);
            }
            self.short[i] = follow(self.short[i], x, dt, 0.4, 0.4);
            self.long[i] = follow(self.long[i], x, dt, LAYER_MEMORY, LAYER_MEMORY);
        }
        // Once the song has settled, the averages start from where it is.
        if self.heard >= SETTLE && self.heard - dt < SETTLE {
            self.long = self.short;
        }
        if first {
            self.armed = [true; 5];
            self.was_thin = true;
            self.thin_for = 0.0;
            self.exit_armed = true;
            self.tension_past = h.tension;
        }
        let mut entered = 0.0;
        for i in 0..5 {
            let jump = self.short[i] - self.long[i];
            self.rest[i] -= dt;
            if self.armed[i] && self.rest[i] <= 0.0 && jump > ENTER_JUMP && self.short[i] > ENTER_LEVEL && self.heard > SETTLE {
                self.armed[i] = false;
                self.rest[i] = ENTER_REST;
                entered += LAYER_WEIGHT[i];
            } else if jump < 0.1 {
                self.armed[i] = true;
            }
        }
        // Everyone means the rhythm section too: a riser and a snare roll
        // fill the spectrum without it.
        let rhythm = self.short[0].min(self.short[1]);
        let o = &mut self.out;
        if entered > 0.0 {
            o.entrance = o.entrance.max((0.4 + 2.4 * entered).min(1.0));
            o.lift = (o.lift + entered).min(LIFT_MOST);
        }
        // (Nor is a build-up everyone, however full it sounds: it is the wait.)
        if self.short[5] < EVERYONE.1 || rhythm < 0.3 || h.build > 0.4 {
            self.thin_for += dt;
            self.was_thin |= self.thin_for >= THIN_FOR;
        } else if self.short[5] > EVERYONE.0 && rhythm > 0.6 {
            self.thin_for = 0.0;
            let was = std::mem::take(&mut self.was_thin);
            if was && self.heard > SETTLE {
                o.everyone_in = 1.0;
                o.lift = LIFT_MOST;
            }
        }
        let fall = self.long[5] - self.short[5];
        if self.exit_armed && fall > EXIT_FALL {
            self.exit_armed = false;
            o.exit = (fall / (2.0 * EXIT_FALL) + 0.5).min(1.0);
            o.lift = 0.0;
        } else if fall < 0.1 {
            self.exit_armed = true;
        }

        // Holding back: the beat goes on, layers are missing.
        let held = (2.0 * drive * (1.0 - fullness)).min(1.0);
        o.hold = if held > o.hold { follow(o.hold, held, dt, HOLD_GROW, HOLD_GROW) } else { follow(o.hold, held, dt, 1.0, 1.0) };
        let tension = h.tension + (1.0 - h.tension) * HOLD_SHARE * o.hold;
        self.tension_past = follow(self.tension_past, tension, dt, 2.0, 2.0);
        self.release_wait -= dt;
        if self.tension_past - tension > RELEASE_FALL && self.release_wait <= 0.0 {
            o.release = ((self.tension_past - tension) / (2.0 * RELEASE_FALL) + 0.5).min(1.0);
            self.release_wait = 4.0;
        }

        o.loudness = follow(o.loudness, loudness, dt, 0.05, 0.3);
        o.drive = follow(o.drive, drive, dt, 0.5, 0.5);
        o.heaviness = follow(o.heaviness, heaviness, dt, 0.5, 2.0);
        o.fullness = follow(o.fullness, fullness, dt, 0.4, 0.4);
        o.layers = self.short[..5].try_into().unwrap();
        o.tension = tension;
        let mix = ENERGY_MIX[0] * o.fullness + ENERGY_MIX[1] * o.drive + ENERGY_MIX[2] * o.loudness;
        o.energy = (mix + o.lift * (1.0 - mix)).clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use crate::scenarios;

    /// When each moment fires in a scenario: (seconds, name).
    fn moments(name: &str) -> Vec<(f32, &'static str)> {
        let s = scenarios::all().into_iter().find(|s| s.name == name).unwrap();
        let mut a = crate::analysis::Analyzer::new(scenarios::SAMPLE_RATE);
        let dt = crate::analysis::HOP as f32 / scenarios::SAMPLE_RATE;
        let mut out = Vec::new();
        let mut was = [0.0f32; 4];
        let mut n = 0;
        for chunk in s.render().chunks(crate::analysis::HOP) {
            a.push(chunk, |f, _| {
                n += 1;
                let now = [f.entrance, f.everyone_in, f.exit, f.release];
                for (i, name) in ["entrance", "everyone in", "exit", "release"].into_iter().enumerate() {
                    if now[i] > 0.5 && was[i] <= 0.5 {
                        out.push((n as f32 * dt, name));
                    }
                }
                was = now;
            });
        }
        println!("{name}: {out:?}");
        out
    }

    fn fired(m: &[(f32, &str)], name: &str, from: f32, to: f32) -> usize {
        m.iter().filter(|(t, n)| *n == name && (from..to).contains(t)).count()
    }

    #[test]
    fn the_band_coming_in_is_everyone_in() {
        // Held chords for 16 s, then the whole band.
        let m = moments("syn-intro");
        assert_eq!(fired(&m, "everyone in", 16.0, 19.0), 1, "{m:?}");
        assert_eq!(fired(&m, "everyone in", 0.0, 16.0), 0, "{m:?}");
        assert!(fired(&m, "entrance", 16.0, 19.0) >= 1, "{m:?}");
    }

    #[test]
    fn a_breakdown_is_an_exit_and_the_chorus_after_it_everyone_in() {
        // Verse 0, chorus 16, verse 32, chorus 48, breakdown 64, chorus 72.
        let m = moments("syn-sections");
        assert_eq!(fired(&m, "exit", 64.0, 67.0), 1, "{m:?}");
        assert_eq!(fired(&m, "everyone in", 72.0, 75.0), 1, "{m:?}");
        assert_eq!(fired(&m, "everyone in", 16.0, 19.0), 1, "{m:?}");
    }

    #[test]
    fn steady_music_has_no_moments() {
        let m = moments("syn-steady");
        assert!(m.iter().all(|(t, _)| *t < 4.0), "{m:?}");
    }

    #[test]
    fn the_drop_after_a_build_up_is_a_release() {
        // Groove 0, build 16, gap 32, drop 33.
        let m = moments("syn-build-drop");
        assert!(fired(&m, "release", 32.0, 37.0) >= 1, "{m:?}");
        assert_eq!(fired(&m, "everyone in", 33.0, 36.0), 1, "{m:?}");
    }

    /// Run by hand: `PROBE=file FROM=s TO=s cargo test --release -- --ignored centre_probe --nocapture`
    #[test]
    #[ignore]
    fn centre_probe() {
        let var = |k: &str, d: usize| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        let path = std::env::var("PROBE").unwrap();
        let (from, to) = (var("FROM", 0), var("TO", usize::MAX));
        let (samples, ch, rate) = crate::audio::decode_file(std::path::Path::new(&path)).unwrap();
        let stereo: Vec<f32> = samples.chunks(ch).flat_map(|c| [c[0], c[if ch > 1 { 1 } else { 0 }]]).collect();
        let mut a = crate::analysis::Analyzer::new(rate as f32);
        let per = (rate as f32 / crate::analysis::HOP as f32) as usize;
        let (mut c, mut all, mut sing, mut hop) = (0.0f32, 0.0f32, 0.0f32, 0usize);
        for chunk in stereo.chunks(2 * crate::analysis::HOP) {
            a.push_stereo(chunk, |f, _| sing += f.singing);
            let (x, y) = a.centre_and_all();
            c += x;
            all += y;
            hop += 1;
            if hop % per == 0 {
                let t = hop / per;
                if t >= from && t <= to {
                    println!("CEN {t:4} centre {:6.1} dB  all {:6.1} dB  share {:5.1} dB  singing {:.2}", 10.0 * (c / per as f32 + 1e-12).log10(), 10.0 * (all / per as f32 + 1e-12).log10(), 10.0 * (c / all.max(1e-12)).log10(), sing / per as f32);
                }
                (c, all, sing) = (0.0, 0.0, 0.0);
            }
        }
    }

    /// Run by hand: `PROBE=file FROM=s TO=s EVERY=s cargo test --release -- --ignored aspects_probe --nocapture`
    #[test]
    #[ignore]
    fn aspects_probe() {
        let var = |k: &str, d: usize| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        let path = std::env::var("PROBE").unwrap();
        let (from, to, every) = (var("FROM", 0), var("TO", usize::MAX), var("EVERY", 1));
        let (samples, ch, rate) = crate::audio::decode_file(std::path::Path::new(&path)).unwrap();
        let stereo: Vec<f32> = samples.chunks(ch).flat_map(|c| [c[0], c[if ch > 1 { 1 } else { 0 }]]).collect();
        let mut a = crate::analysis::Analyzer::new(rate as f32);
        let per = (rate as f32 / crate::analysis::HOP as f32) as usize * every;
        let mut acc = [0.0f32; 14];
        let mut moments = [0.0f32; 5];
        let mut hop = 0;
        println!("ASP    t energy  loud  full drive heavy  solo tensn  hold  lift | beat bass body  top lead | moments");
        for chunk in stereo.chunks(2 * crate::analysis::HOP) {
            a.push_stereo(chunk, |f, raw| {
                hop += 1;
                let v = [f.energy, f.loudness, f.fullness, f.drive, f.heaviness, f.solo, f.tension, raw.hold, raw.energy_parts[4], raw.layers[0], raw.layers[1], raw.layers[2], raw.layers[3], raw.layers[4]];
                for (s, x) in acc.iter_mut().zip(v) {
                    *s += x / per as f32;
                }
                for (m, x) in moments.iter_mut().zip([f.entrance, f.everyone_in, f.exit, f.release, f.drop]) {
                    *m = m.max(x);
                }
                if hop % per == 0 {
                    let t = hop / per * every;
                    if t >= from && t <= to {
                        let names = ["ENTRANCE", "EVERYONE-IN", "EXIT", "RELEASE", "DROP"];
                        let fired: Vec<String> = names.iter().zip(moments).filter(|(_, m)| *m > 0.3).map(|(n, m)| format!("{n} {m:.1}")).collect();
                        println!(
                            "ASP {t:4}  {:.2}   {:.2}  {:.2}  {:.2}  {:.2}  {:.2}  {:.2}  {:.2}  {:.2} | {:.2} {:.2} {:.2} {:.2} {:.2} | {}",
                            acc[0], acc[1], acc[2], acc[3], acc[4], acc[5], acc[6], acc[7], acc[8], acc[9], acc[10], acc[11], acc[12], acc[13], fired.join(" ")
                        );
                    }
                    acc = [0.0; 14];
                    moments = [0.0; 5];
                }
            });
        }
    }
}
