//! Synthetic music with a known answer: chord progressions, arrangements and
//! build-ups, for testing how energy and tension read.
//!
//! A [`Scenario`] is a row of [`Section`]s. Each section has a label (what a
//! musician would call it: `I`, `V7`, `chorus`, `build`…), a length in beats,
//! and what plays in it: a chord (a soft keyboard, restruck every beat), a
//! bass note, a melody, kick, hat and snare, a snare roll, a riser.
//!
//! `--scenarios` writes each one as `scenarios/<name>.wav` with its labels in
//! `scenarios/<name>.labels`, and analyses them all into the timeline
//! (`analysis/index.html`), where the labels are drawn over the tension lane.
//! The tests (here and in `energy.rs`, `harmony.rs`) run the same audio
//! through the analyser and check the readings section by section.

use std::path::Path;

pub const SAMPLE_RATE: f32 = 48_000.0;

#[derive(Clone, Debug)]
pub struct Section {
    pub label: String,
    pub beats: f32,
    /// The chord's notes (MIDI numbers); empty for none.
    pub notes: Vec<i32>,
    /// The bass note (MIDI), if a bass plays.
    pub bass: Option<i32>,
    /// Level of everything, from the section's start to its end (1 = normal).
    pub gain: (f32, f32),
    pub kick: bool,
    pub hat: bool,
    pub snare: bool,
    /// A snare roll, getting faster: quarters, eighths, sixteenths, 32nds.
    pub roll: bool,
    /// A rising sweep of tone and noise, opening up over the section.
    pub riser: bool,
    /// A melody on the chord's notes, an octave up, in eighths.
    pub melody: bool,
    /// How bright the keyboard is (number of overtones).
    pub overtones: usize,
    /// Hats on every sixteenth, not only the off-beats.
    pub sixteenths: bool,
    /// The keyboard holds each chord (a pad swelling in) and is not restruck
    /// on every beat.
    pub held: bool,
    /// Where this section sits in its run of sections with the same label
    /// (0 = the run's start … 1 = its end): swells, rolls and risers span
    /// the whole run.
    pub span: (f32, f32),
}

impl Section {
    fn new(label: &str, beats: f32) -> Self {
        Self {
            label: label.to_string(),
            beats,
            notes: Vec::new(),
            bass: None,
            gain: (1.0, 1.0),
            kick: false,
            hat: false,
            snare: false,
            roll: false,
            riser: false,
            melody: false,
            overtones: 5,
            sixteenths: false,
            held: false,
            span: (0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: &'static str,
    /// What it shows, and what the readings should do.
    pub about: &'static str,
    pub bpm: f32,
    pub sections: Vec<Section>,
}

/// The notes of a chord by name, around middle C, and its bass note:
/// `C`, `Am`, `G7`, `Cmaj7`, `Dm7`, `Bdim`, `Bdim7`, `Bm7b5`, `Caug`,
/// `Csus4`, `Csus2`, `E7b9`, `C5` (no third), `Cclu` (a cluster: C, C♯, D).
pub fn chord(name: &str) -> (Vec<i32>, i32) {
    let bytes = name.as_bytes();
    let mut root = match bytes[0] {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => panic!("chord {name}?"),
    };
    let mut at = 1;
    match bytes.get(1) {
        Some(b'#') => (root, at) = (root + 1, 2),
        Some(b'b') => (root, at) = (root + 11, 2),
        _ => {}
    }
    let root = root % 12;
    let steps: &[i32] = match &name[at..] {
        "" => &[0, 4, 7],
        "m" => &[0, 3, 7],
        "7" => &[0, 4, 7, 10],
        "maj7" => &[0, 4, 7, 11],
        "m7" => &[0, 3, 7, 10],
        "dim" => &[0, 3, 6],
        "dim7" => &[0, 3, 6, 9],
        "m7b5" => &[0, 3, 6, 10],
        "aug" => &[0, 4, 8],
        "sus4" => &[0, 5, 7],
        "sus2" => &[0, 2, 7],
        "7b9" => &[0, 4, 7, 10, 13],
        "5" => &[0, 7],
        "clu" => &[0, 1, 2],
        other => panic!("chord quality {other}?"),
    };
    (steps.iter().map(|s| 55 + (root + 5) % 12 + s).collect(), 36 + root)
}

impl Scenario {
    pub(crate) fn new(name: &'static str, about: &'static str, bpm: f32) -> Self {
        Self { name, about, bpm, sections: Vec::new() }
    }

    /// One section per `label=chord` pair (or just `chord`, which is then the
    /// label too; `-` is no chord), each `beats` long, shaped by `with`.
    pub(crate) fn chords(mut self, row: &str, beats: f32, with: impl Fn(&mut Section)) -> Self {
        for item in row.split_whitespace() {
            let (label, name) = item.split_once('=').unwrap_or((item, item));
            let mut s = Section::new(label, beats);
            if name != "-" {
                let (notes, bass) = chord(name);
                s.notes = notes;
                s.bass = Some(bass);
            }
            with(&mut s);
            self.sections.push(s);
        }
        self
    }

    fn times(mut self, n: usize) -> Self {
        let once = self.sections.clone();
        for _ in 1..n {
            self.sections.extend(once.iter().cloned());
        }
        self
    }

    fn then(mut self, other: Scenario) -> Self {
        self.sections.extend(other.sections);
        self
    }

    /// The same music, `semitones` higher.
    #[cfg(test)]
    pub fn transposed(mut self, semitones: i32) -> Self {
        for s in &mut self.sections {
            for n in &mut s.notes {
                *n += semitones;
            }
            s.bass = s.bass.map(|b| b + semitones);
        }
        self
    }

    /// Where each section starts (s), and its label.
    pub fn labels(&self) -> Vec<(f32, String)> {
        let mut t = 0.0;
        self.sections
            .iter()
            .map(|s| {
                let at = t;
                t += s.beats * 60.0 / self.bpm;
                (at, s.label.clone())
            })
            .collect()
    }

    pub fn seconds(&self) -> f32 {
        self.sections.iter().map(|s| s.beats).sum::<f32>() * 60.0 / self.bpm
    }

    /// The audio, mono, at [`SAMPLE_RATE`].
    pub fn render(&self) -> Vec<f32> {
        let sr = SAMPLE_RATE;
        let beat = 60.0 / self.bpm;
        let mut out = Vec::with_capacity((self.seconds() * sr) as usize + 1);
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        };
        let hz = |note: i32| 440.0 * 2f32.powf((note - 69) as f32 / 12.0);
        let tau = std::f32::consts::TAU;
        // A struck note: quick attack, a slow fade to half.
        let struck = |t: f32, hold: f32| (t / 0.012).min(1.0) * (0.45 + 0.55 * (-t / 0.9).exp()) * ((hold - t) / 0.03).clamp(0.0, 1.0);
        for s in &self.sections {
            let n = (s.beats * beat * sr).round() as usize;
            let mut last_noise = 0.0;
            let mut riser_phase = 0.0f32;
            for i in 0..n {
                let t = i as f32 / sr;
                let along = s.span.0 + (s.span.1 - s.span.0) * i as f32 / n as f32;
                let in_beat = t % beat;
                let beat_no = (t / beat) as usize;
                let mut x = 0.0;
                // The keyboard, restruck every beat.
                for &note in &s.notes {
                    let f = hz(note);
                    let env = if s.held { (t / 0.4).min(1.0) * ((n as f32 / sr - t) / 0.2).clamp(0.0, 1.0) } else { struck(in_beat, beat) };
                    for h in 1..=s.overtones {
                        x += 0.07 / h as f32 * env * (tau * f * h as f32 * t).sin();
                    }
                }
                if let Some(b) = s.bass {
                    let f = hz(b);
                    let env = struck(in_beat, beat);
                    x += env * (0.22 * (tau * f * t).sin() + 0.08 * (tau * 2.0 * f * t).sin() + 0.04 * (tau * 3.0 * f * t).sin());
                }
                if s.melody && !s.notes.is_empty() {
                    let eighth = (t / (beat / 2.0)) as usize;
                    let te = t % (beat / 2.0);
                    // Up and down the chord, with a passing note now and then.
                    let order = [0usize, 1, 2, 1, 2, 0, 1, 2];
                    let note = s.notes[order[eighth % 8] % s.notes.len()] + 12 + if eighth % 8 == 5 { 2 } else { 0 };
                    let env = struck(te, beat / 2.0);
                    let f = hz(note);
                    x += 0.09 * env * ((tau * f * t).sin() + 0.4 * (tau * 2.0 * f * t).sin() + 0.2 * (tau * 3.0 * f * t).sin());
                }
                let white = noise();
                let hiss = white - last_noise;
                last_noise = white;
                if s.kick {
                    let phase = tau * (50.0 * in_beat + 90.0 * 0.03 * (1.0 - (-in_beat / 0.03).exp()));
                    x += 0.5 * (-in_beat / 0.12).exp() * phase.sin();
                }
                if s.hat {
                    let te = (t + beat / 2.0) % beat;
                    if beat_no > 0 || t >= beat / 2.0 {
                        x += 0.16 * (-te / 0.03).exp() * hiss;
                    }
                    if s.sixteenths {
                        x += 0.16 * (-(t % (beat / 4.0)) / 0.02).exp() * hiss;
                    }
                }
                if s.snare && beat_no % 2 == 1 {
                    x += (-in_beat / 0.08).exp() * (0.22 * white + 0.12 * (tau * 185.0 * in_beat).sin());
                }
                if s.roll {
                    // Four stages, each twice as fast.
                    let stage = ((along * 4.0) as usize).min(3);
                    let step = beat / (1 << stage) as f32;
                    let te = t % step;
                    x += (0.5 + 0.5 * along) * (-te / 0.05).exp() * (0.2 * white + 0.1 * (tau * 185.0 * te).sin());
                }
                if s.riser {
                    // 200 Hz to 3.2 kHz, with hiss that grows.
                    let f = 200.0 * 16f32.powf(along);
                    riser_phase += tau * f / sr;
                    x += along * (0.12 * riser_phase.sin() + 0.1 * hiss);
                }
                out.push(x * (s.gain.0 + (s.gain.1 - s.gain.0) * along));
            }
        }
        out
    }

    /// Write `<dir>/<name>.wav` (16-bit mono) and `<dir>/<name>.labels`.
    pub fn write(&self, dir: &Path) -> std::io::Result<std::path::PathBuf> {
        let samples = self.render();
        let peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-6);
        // Never clip; otherwise leave the level alone (it is part of the test).
        let scale = (0.98 / peak).min(1.0);
        let data: Vec<u8> = samples.iter().flat_map(|x| ((x * scale * 32767.0).round() as i16).to_le_bytes()).collect();
        let rate = SAMPLE_RATE as u32;
        let mut wav = Vec::with_capacity(44 + data.len());
        wav.extend(b"RIFF");
        wav.extend((36 + data.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(data);
        let path = dir.join(format!("{}.wav", self.name));
        std::fs::write(&path, wav)?;
        let labels: String = self.labels().iter().map(|(t, l)| format!("{t:.3}\t{l}\n")).collect();
        std::fs::write(dir.join(format!("{}.labels", self.name)), format!("# {}\n{labels}", self.about))?;
        Ok(path)
    }
}

fn plain(_: &mut Section) {}

/// A band behind the chords: bass, kick, hat, snare.
fn band(s: &mut Section) {
    s.kick = true;
    s.hat = true;
    s.snare = true;
}

/// The cadence that sets up C major as home: I IV V I.
fn home() -> Scenario {
    Scenario::new("", "", 100.0).chords("I=C IV=F V=G I=C", 4.0, plain)
}

pub fn harmony() -> Vec<Scenario> {
    vec![
        Scenario::new("syn-cadence", "I IV V I in C major, four times. Tension: I lowest, IV in between, V highest, and back down on I.", 100.0)
            .chords("I=C IV=F V=G I=C", 4.0, plain)
            .times(4),
        Scenario::new("syn-seventh", "I IV V V7 I: the seventh on the dominant adds a tritone. Tension: V7 above V.", 100.0)
            .chords("I=C IV=F V=G V7=G7 I=C I=C", 4.0, plain)
            .times(3),
        Scenario::new("syn-two-five-one", "ii7 V7 Imaj7 (jazz) in C. Tension: highest on V7, lowest on I.", 100.0)
            .chords("I=Cmaj7 ii7=Dm7 V7=G7 I=Cmaj7", 4.0, plain)
            .times(4),
        Scenario::new("syn-minor", "i iv V i in A minor, with the major dominant (E, its G sharp outside the scale). Tension: V highest, i lowest.", 100.0)
            .chords("i=Am iv=Dm V=E i=Am", 4.0, plain)
            .times(4),
        Scenario::new("syn-pop-loop", "I V vi IV, the four-chord loop, in C. Tension: I lowest, V highest; vi and IV in between.", 100.0)
            .chords("I=C V=G vi=Am IV=F", 4.0, plain)
            .times(5),
        Scenario::new("syn-deceptive", "I IV V vi (the deceptive cadence), then I IV V I. Tension falls after V both times, less far on vi.", 100.0)
            .chords("I=C IV=F V=G vi=Am I=C IV=F V=G I=C", 4.0, plain)
            .times(3),
        home().then(
            Scenario::new("", "", 100.0).chords("I=C bVI=Ab bVII=Bb I=C", 4.0, plain).times(3),
        )
        .named("syn-borrowed", "After a cadence in C: I bVI bVII I, chords borrowed from C minor. Tension: the borrowed chords above the plain IV and V."),
        home().then(
            Scenario::new("", "", 100.0).chords("I=C ii=Dm V=G I=C I=C V/V=D7 V=G I=C", 4.0, plain).times(2),
        )
        .named("syn-secondary", "ii V I, then the same with ii made a dominant (D7, its F sharp outside the key). Tension: V/V above ii."),
        home().then(
            Scenario::new("", "", 100.0).chords("I=C sus4=Csus4 I=C dim=Cdim7 I=C aug=Caug I=C cluster=Cclu I=C", 4.0, plain).times(2),
        )
        .named("syn-colours", "The home chord against its tense versions: sus4, diminished seventh, augmented, a cluster. Tension: each above I; the cluster highest."),
        Scenario::new("syn-pedal", "The home chord, and nothing else, for 40 s. Tension stays low and flat.", 100.0).chords("I=C", 4.0, plain).times(16),
        home()
            .times(2)
            .then(Scenario::new("", "", 100.0).chords("V7=G7 V7=G7 V7=G7 V7=G7 V7=G7 V7=G7 I=C I=C", 4.0, plain))
            .named("syn-hanging", "Two cadences, then the dominant seventh held for six bars before it resolves. Tension keeps growing while it hangs, and falls on I."),
        home()
            .times(3)
            .then(Scenario::new("", "", 100.0).chords("I'=F# IV'=B V'=C# I'=F#", 4.0, plain).times(8))
            .named("syn-modulation", "Three cadences in C, then the same cadence in F sharp, as far away as keys get, eight times. At first F sharp is tense; after half a minute it is home."),
        Scenario::new("syn-band", "I V vi IV with a band: bass, drums and a melody on top. The tension should read as in syn-pop-loop.", 100.0)
            .chords("I=C V=G vi=Am IV=F", 4.0, |s| {
                band(s);
                s.melody = true;
            })
            .times(5),
    ]
}

pub fn energy() -> Vec<Scenario> {
    let quiet = |s: &mut Section| {
        s.kick = true;
        s.overtones = 3;
        s.gain = (0.7, 0.7);
    };
    let full = |s: &mut Section| {
        band(s);
        s.sixteenths = true;
        s.melody = true;
        s.overtones = 8;
        s.gain = (1.1, 1.1);
    };
    let pad = |s: &mut Section| {
        s.bass = None;
        s.overtones = 3;
        s.gain = (0.6, 0.6);
    };
    let song = |label: &'static str, bars: usize, with: &dyn Fn(&mut Section)| {
        let row = ["C", "G", "Am", "F"].iter().cycle().take(bars).map(|c| format!("{label}={c}")).collect::<Vec<_>>().join(" ");
        Scenario::new("", "", 120.0).chords(&row, 4.0, with)
    };
    vec![
        song("verse", 8, &quiet)
            .then(song("chorus", 8, &full))
            .then(song("verse", 8, &quiet))
            .then(song("chorus", 8, &full))
            .then(song("breakdown", 4, &pad))
            .then(song("chorus", 8, &full))
            .named("syn-sections", "Verse (kick, bass, soft keys), chorus (the whole band, 4 dB louder, brighter), a breakdown (keys only). Energy: chorus high, verse in the middle, breakdown low."),
        song("kick+bass", 8, &|s| {
            s.notes.clear();
            s.kick = true;
        })
        .then(song("+keys", 8, &|s| s.kick = true))
        .then(song("+hats", 8, &|s| {
            s.kick = true;
            s.hat = true;
        }))
        .then(song("+snare+melody", 8, &full))
        .then(song("kick+bass", 8, &|s| {
            s.notes.clear();
            s.kick = true;
        }))
        .named("syn-layers", "An arrangement growing layer by layer, as a limited club track does, with hardly any change in level. Energy rises with each layer and falls back at the end."),
        song("groove", 8, &full)
            .then(song("build", 8, &|s| {
                s.bass = None;
                s.roll = true;
                s.riser = true;
                s.overtones = 3;
                s.gain = (0.35, 1.0);
            }))
            .then(Scenario::new("", "", 120.0).chords("gap=-", 2.0, plain))
            .then(song("drop", 8, &full))
            .named("syn-build-drop", "A groove; then eight bars of build-up: the kick and bass drop out, a snare roll gets faster, a riser sweeps up, everything swells; half a bar of silence; the drop. Tension climbs through the build, holds over the gap, and lets go at the drop. Energy: low at the start of the build, highest after the drop."),
        song("intro", 8, &|s| {
            s.bass = None;
            s.overtones = 5;
            s.held = true;
            s.gain = (0.25, 0.9);
        })
        .then(song("beat", 12, &full))
        .named("syn-intro", "Sixteen seconds of held chords alone, swelling, with no drums and no bass; then the whole band comes in. Energy: low through the intro, however much it grows, and up when the beat arrives."),
        song("steady", 24, &full).named("syn-steady", "The whole band, unchanged, for 48 s. Energy holds steady; no build-up tension."),
        song("groove", 8, &full)
            .then(song("fade", 8, &|s| {
                full(s);
                s.gain = (1.1, 0.02);
            }))
            .named("syn-fade", "A groove, then the same fading out over 16 s. Energy falls with it."),
        song("song", 8, &full)
            .then(song("fade", 4, &|s| {
                full(s);
                s.gain = (1.1, 0.02);
            }))
            .then(Scenario::new("", "", 120.0).chords("silence=- silence=- silence=- silence=-", 4.0, plain))
            .then(song("next", 8, &full))
            .named("syn-ending", "A song, fading out over 8 s; 8 s of silence; the next song. In the silence everything comes to rest (tempo, pulse, tension and harmony at 0, brightness and the pitches in the middle), and the next song starts as the first did."),
    ]
}

impl Scenario {
    fn named(mut self, name: &'static str, about: &'static str) -> Self {
        (self.name, self.about) = (name, about);
        // Each run of sections with the same label is one stretch of music.
        let mut i = 0;
        while i < self.sections.len() {
            let run = self.sections[i..].iter().take_while(|s| s.label == self.sections[i].label).count();
            let total: f32 = self.sections[i..i + run].iter().map(|s| s.beats).sum();
            let mut at = 0.0;
            for s in &mut self.sections[i..i + run] {
                s.span = (at / total, (at + s.beats) / total);
                at += s.beats;
            }
            i += run;
        }
        self
    }
}

pub fn all() -> Vec<Scenario> {
    let mut v = harmony();
    v.extend(energy());
    v
}

/// `--scenarios`: write them all to `dir`; returns the files.
pub fn write_all(dir: &Path) -> Vec<String> {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }
    let mut files = Vec::new();
    for s in all() {
        match s.write(dir) {
            Ok(path) => {
                println!("{}: {:.0} s. {}", s.name, s.seconds(), s.about);
                files.push(path.to_string_lossy().into_owned());
            }
            Err(e) => eprintln!("{}: {e}", s.name),
        }
    }
    files
}

/// What the analyser reads in each section of a scenario (tests).
#[cfg(test)]
pub mod measure {
    use super::*;
    use crate::analysis::{Analyzer, HOP};

    #[derive(Debug, Clone)]
    pub struct Reading {
        pub label: String,
        pub start: f32,
        /// Means over the section, leaving out its first 40% (the readings
        /// need a moment to arrive).
        pub tension: f32,
        pub energy: f32,
        /// Tension's parts: function, dissonance, suspense, build.
        pub parts: [f32; 4],
        /// Energy at the section's very end.
        pub energy_end: f32,
        pub tension_end: f32,
    }

    /// One reading per run of sections with the same label.
    pub fn run(s: &Scenario) -> Vec<Reading> {
        let samples = s.render();
        let dt = HOP as f32 / SAMPLE_RATE;
        let mut frames: Vec<(f32, f32, [f32; 4])> = Vec::new();
        let mut a = Analyzer::new(SAMPLE_RATE);
        for chunk in samples.chunks(HOP) {
            a.push(chunk, |f, raw| frames.push((f.tension, f.energy, raw.tension_parts)));
        }
        // The analyser's window puts its readings about 20 ms late: nothing to correct.
        let labels = s.labels();
        let mut spans: Vec<(String, f32, f32)> = Vec::new();
        for (i, (t, l)) in labels.iter().enumerate() {
            let end = labels.get(i + 1).map_or(s.seconds(), |n| n.0);
            match spans.last_mut() {
                Some(last) if last.0 == *l => last.2 = end,
                _ => spans.push((l.clone(), *t, end)),
            }
        }
        spans
            .into_iter()
            .map(|(label, start, end)| {
                let from = (((start + 0.4 * (end - start)) / dt) as usize).min(frames.len().saturating_sub(1));
                let to = ((end / dt) as usize).clamp(from + 1, frames.len());
                let n = (to - from) as f32;
                let mean = |get: &dyn Fn(&(f32, f32, [f32; 4])) -> f32| frames[from..to].iter().map(get).sum::<f32>() / n;
                Reading {
                    tension: mean(&|f| f.0),
                    energy: mean(&|f| f.1),
                    parts: std::array::from_fn(|k| mean(&|f| f.2[k])),
                    energy_end: frames[to - 1].1,
                    tension_end: frames[to - 1].0,
                    label,
                    start,
                }
            })
            .collect()
    }

    /// The mean of `get` over all readings with this label, skipping the
    /// first `skip` of them (while the key is still being found).
    pub fn mean(readings: &[Reading], label: &str, skip: usize, get: impl Fn(&Reading) -> f32) -> f32 {
        let v: Vec<f32> = readings.iter().filter(|r| r.label == label).skip(skip).map(get).collect();
        assert!(!v.is_empty(), "no section {label} after the first {skip}");
        v.iter().sum::<f32>() / v.len() as f32
    }

    /// A table of the readings, for failure messages and `--nocapture`.
    pub fn table(name: &str, readings: &[Reading]) -> String {
        let mut s = format!("{name}\n   time  label          tension (function dissonance suspense build)  energy\n");
        for r in readings {
            s += &format!(
                "  {:5.1}  {:14} {:.2}    ({:.2} {:.2} {:.2} {:.2})                      {:.2}\n",
                r.start, r.label, r.tension, r.parts[0], r.parts[1], r.parts[2], r.parts[3], r.energy
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_by_name() {
        let pcs = |name: &str| chord(name).0.iter().map(|n| n.rem_euclid(12)).collect::<Vec<_>>();
        assert_eq!(pcs("C"), [0, 4, 7]);
        assert_eq!(pcs("Am"), [9, 0, 4]);
        assert_eq!(pcs("G7"), [7, 11, 2, 5]);
        assert_eq!(pcs("F#"), [6, 10, 1]);
        assert_eq!(pcs("Bb"), [10, 2, 5]);
        assert_eq!(chord("C").1.rem_euclid(12), 0);
        assert_eq!(chord("A").1.rem_euclid(12), 9);
        // Chords sit around middle C, basses two octaves below.
        for name in ["C", "E", "G", "B"] {
            let (notes, bass) = chord(name);
            assert!((55..67).contains(&notes[0]), "{name}: {notes:?}");
            assert!((36..48).contains(&bass), "{name}: bass {bass}");
        }
    }

    /// Run by hand: `cargo test --release -- --ignored tables --nocapture`
    /// (`ONLY=syn-cadence` for one) prints every scenario's readings.
    #[test]
    #[ignore]
    fn tables() {
        let only = std::env::var("ONLY").ok();
        for s in all().into_iter().filter(|s| only.as_deref().is_none_or(|o| s.name.contains(o))) {
            println!("{}", measure::table(s.name, &measure::run(&s)));
        }
    }

    #[test]
    fn scenarios_render_without_clipping_much() {
        for s in all() {
            assert!(!s.name.is_empty() && !s.about.is_empty());
            let x = s.render();
            assert!((x.len() as f32 / SAMPLE_RATE - s.seconds()).abs() < 0.1, "{}", s.name);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.1 && peak < 2.0, "{}: peak {peak}", s.name);
        }
    }
}
