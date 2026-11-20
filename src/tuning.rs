//! From music to motion, in three layers that never mix their languages:
//!
//! ```text
//! signals (music)  ──weights──▶  transforms  ──amounts──▶  controls (the scene's world)
//! loudness, bass,               weighted sum,              wind speed, vortex
//! beat, onsets, …               curve, attack/release      circulation, hop, …
//! ```
//!
//! - A **signal** is something we hear: loudness, the bands, brightness,
//!   onsets, the beat, tempo, pulse clarity, note density, pitch. All 0..1.
//! - A **transform** mixes signals (one weight each), bends the result with a
//!   response curve and gives it a time behaviour (attack and release): a
//!   section-long swell, a quick pulse on every beat that coasts down, …
//!   Its output is 0..1.
//! - A **control** is a quantity of the current scene's world, in that
//!   world's language: for the bubbles, wind speed, a vortex's circulation,
//!   the plate's vibration…; for the glass window, pane drift, breath, the
//!   backlight… It is a base value plus an amount of each transform.
//!
//! Signals and transforms are shared by every scene. Each scene has its own
//! controls and plain constants (parameters) as tables (see `scene.rs`): the
//! scene reads them, the preferences panel draws them, and the preferences
//! file stores them by key.

use std::path::PathBuf;

use bevy::prelude::*;

use crate::{
    features::{AudioFeatures, follow},
    scene::{ControlId, ParamId, Scene, SceneControl, SceneParam},
};

// ===========================================================================
// Signals: what we hear.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    Loudness,
    Bass,
    LowMids,
    HighMids,
    Treble,
    Brightness,
    Onsets,
    Beat,
    PulseClarity,
    Tempo,
    NoteDensity,
    BassPitch,
    DominantPitch,
    /// A kick locked to the beat clock: 1 on each beat, decaying until the next.
    BeatKick,
    /// The lead line's pitch; a melodic voice, rap, and rap bursting in.
    MelodyPitch,
    Singing,
    Rapping,
    RapEntry,
    /// The music crashing back in, and mosh-pit music.
    Drop,
    Mosh,
    /// Harmony: a chord change, major (1) or minor (0), how clear the chord
    /// is, and how far the harmony is from the song's centre.
    ChordChange,
    Major,
    Tonality,
    Tension,
    Energy,
    /// The aspects and moments of `aspects.rs`.
    Fullness,
    Drive,
    Heaviness,
    Solo,
    Entrance,
    EveryoneIn,
    Exit,
    Release,
}

impl Signal {
    pub const COUNT: usize = 33;
    pub const ALL: [Signal; Self::COUNT] = [
        Signal::Loudness,
        Signal::Bass,
        Signal::LowMids,
        Signal::HighMids,
        Signal::Treble,
        Signal::Brightness,
        Signal::Onsets,
        Signal::Beat,
        Signal::PulseClarity,
        Signal::Tempo,
        Signal::NoteDensity,
        Signal::BassPitch,
        Signal::DominantPitch,
        Signal::BeatKick,
        Signal::MelodyPitch,
        Signal::Singing,
        Signal::Rapping,
        Signal::RapEntry,
        Signal::Drop,
        Signal::Mosh,
        Signal::ChordChange,
        Signal::Major,
        Signal::Tonality,
        Signal::Tension,
        Signal::Energy,
        Signal::Fullness,
        Signal::Drive,
        Signal::Heaviness,
        Signal::Solo,
        Signal::Entrance,
        Signal::EveryoneIn,
        Signal::Exit,
        Signal::Release,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Signal::Loudness => "loudness",
            Signal::Bass => "bass",
            Signal::LowMids => "low_mids",
            Signal::HighMids => "high_mids",
            Signal::Treble => "treble",
            Signal::Brightness => "brightness",
            Signal::Onsets => "onsets",
            Signal::Beat => "beat",
            Signal::PulseClarity => "pulse_clarity",
            Signal::Tempo => "tempo",
            Signal::NoteDensity => "note_density",
            Signal::BassPitch => "bass_pitch",
            Signal::DominantPitch => "dominant_pitch",
            Signal::BeatKick => "beat_kick",
            Signal::MelodyPitch => "melody_pitch",
            Signal::Singing => "singing",
            Signal::Rapping => "rapping",
            Signal::RapEntry => "rap_entry",
            Signal::Drop => "drop",
            Signal::Mosh => "mosh",
            Signal::ChordChange => "chord_change",
            Signal::Major => "major",
            Signal::Tonality => "tonality",
            Signal::Tension => "tension",
            Signal::Energy => "energy",
            Signal::Fullness => "fullness",
            Signal::Drive => "drive",
            Signal::Heaviness => "heaviness",
            Signal::Solo => "solo",
            Signal::Entrance => "entrance",
            Signal::EveryoneIn => "everyone_in",
            Signal::Exit => "exit",
            Signal::Release => "let_go",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Signal::Loudness => "loudness",
            Signal::Bass => "bass",
            Signal::LowMids => "low mids",
            Signal::HighMids => "high mids",
            Signal::Treble => "treble",
            Signal::Brightness => "brightness",
            Signal::Onsets => "onsets",
            Signal::Beat => "beat",
            Signal::PulseClarity => "pulse clarity",
            Signal::Tempo => "tempo",
            Signal::NoteDensity => "note density",
            Signal::BassPitch => "bass pitch",
            Signal::DominantPitch => "dominant pitch",
            Signal::BeatKick => "beat kick",
            Signal::MelodyPitch => "lead pitch",
            Signal::Singing => "singing",
            Signal::Rapping => "rapping",
            Signal::RapEntry => "rap entry",
            Signal::Drop => "drop",
            Signal::Mosh => "mosh pit",
            Signal::ChordChange => "chord change",
            Signal::Major => "major chord",
            Signal::Tonality => "tonality",
            Signal::Tension => "tension",
            Signal::Energy => "energy",
            Signal::Fullness => "fullness (how much of the band plays)",
            Signal::Drive => "drive (makes you move)",
            Signal::Heaviness => "heaviness",
            Signal::Solo => "solo (an instrument in front)",
            Signal::Entrance => "entrance (a layer comes in)",
            Signal::EveryoneIn => "everyone in",
            Signal::Exit => "exit (layers leave)",
            Signal::Release => "release (tension lets go)",
        }
    }
}

/// Current value of every signal.
#[derive(Resource, Debug, Clone, Copy)]
pub struct Signals(pub [f32; Signal::COUNT]);

impl Default for Signals {
    fn default() -> Self {
        Self([0.0; Signal::COUNT])
    }
}

impl Signals {
    pub fn get(&self, s: Signal) -> f32 {
        self.0[s as usize]
    }

    pub fn from_features(f: &AudioFeatures) -> Self {
        Self(Signal::ALL.map(|s| match s {
            Signal::Loudness => f.loudness,
            Signal::Bass => f.bass,
            Signal::LowMids => f.low_mids,
            Signal::HighMids => f.high_mids,
            Signal::Treble => f.treble,
            Signal::Brightness => f.brightness,
            Signal::Onsets => f.onset,
            Signal::Beat => f.beat,
            Signal::PulseClarity => f.pulse_clarity,
            Signal::Tempo => f.tempo,
            Signal::NoteDensity => f.note_density,
            Signal::BassPitch => f.bass_pitch,
            Signal::DominantPitch => f.dominant_pitch,
            // Filled in by `update_signals`, which owns the beat clock.
            Signal::BeatKick => 0.0,
            Signal::MelodyPitch => f.melody_pitch,
            Signal::Singing => f.singing,
            Signal::Rapping => f.rapping,
            Signal::RapEntry => f.rap_entry,
            Signal::Drop => f.drop,
            Signal::Mosh => f.mosh,
            Signal::ChordChange => f.chord_change,
            Signal::Major => f.major,
            Signal::Tonality => f.tonality,
            Signal::Tension => f.tension,
            Signal::Energy => f.energy,
            Signal::Fullness => f.fullness,
            Signal::Drive => f.drive,
            Signal::Heaviness => f.heaviness,
            Signal::Solo => f.solo,
            Signal::Entrance => f.entrance,
            Signal::EveryoneIn => f.everyone_in,
            Signal::Exit => f.exit,
            Signal::Release => f.release,
        }))
    }
}

// ===========================================================================
// Transforms: shaping signals.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transformation {
    Energy,
    Groove,
    LowEnd,
    LowMids,
    HighMids,
    Attacks,
    Busyness,
    Brightness,
    Sparkle,
    Pitch,
    Tempo,
    Dance,
    Pulse,
    Singing,
    Havoc,
    Melody,
    Chords,
    Mood,
    Tension,
    Harmony,
    /// The aspects (`aspects.rs`) and their moments.
    Fullness,
    Drive,
    Heaviness,
    Solo,
    Arrival,
    LetGo,
}

pub struct TransformSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub routes: &'static [(Signal, f32)],
    pub curve: f32,
    pub attack: f32,
    pub release: f32,
}

use Signal as S;

const fn tf(
    key: &'static str,
    label: &'static str,
    routes: &'static [(Signal, f32)],
    attack: f32,
    release: f32,
) -> TransformSpec {
    TransformSpec { key, label, routes, curve: 1.0, attack, release }
}

impl Transformation {
    pub const COUNT: usize = 26;
    pub const ALL: [Transformation; Self::COUNT] = [
        Transformation::Energy,
        Transformation::Groove,
        Transformation::LowEnd,
        Transformation::LowMids,
        Transformation::HighMids,
        Transformation::Attacks,
        Transformation::Busyness,
        Transformation::Brightness,
        Transformation::Sparkle,
        Transformation::Pitch,
        Transformation::Tempo,
        Transformation::Dance,
        Transformation::Pulse,
        Transformation::Singing,
        Transformation::Havoc,
        Transformation::Melody,
        Transformation::Chords,
        Transformation::Mood,
        Transformation::Tension,
        Transformation::Harmony,
        Transformation::Fullness,
        Transformation::Drive,
        Transformation::Heaviness,
        Transformation::Solo,
        Transformation::Arrival,
        Transformation::LetGo,
    ];

    pub fn spec(self) -> &'static TransformSpec {
        &TRANSFORMS[self as usize]
    }
}

/// In the order of [`Transformation`].
pub const TRANSFORMS: [TransformSpec; Transformation::COUNT] = [
    tf("energy", "energy", &[(S::Energy, 1.0)], 0.4, 1.2),
    tf("groove", "groove", &[(S::Beat, 1.0)], 0.03, 0.45),
    tf("low_end", "low end", &[(S::Bass, 1.0)], 0.01, 0.25),
    tf("low_mids", "low mids", &[(S::LowMids, 1.0)], 0.05, 0.4),
    tf("high_mids", "high mids", &[(S::HighMids, 1.0)], 0.05, 0.4),
    tf("attacks", "attacks", &[(S::Onsets, 1.0)], 0.01, 0.3),
    tf("busyness", "busyness", &[(S::NoteDensity, 1.0)], 1.5, 3.0),
    tf("brightness", "brightness", &[(S::Brightness, 1.0)], 0.1, 0.3),
    tf("sparkle", "sparkle", &[(S::Treble, 1.0)], 0.01, 0.2),
    tf("pitch", "pitch", &[(S::DominantPitch, 1.0)], 0.5, 0.5),
    // Tempo only changes in steps, and slowly: follow it gently.
    tf("tempo", "tempo", &[(S::Tempo, 1.0)], 1.5, 3.0),
    // Beat-locked kick, left nearly raw so motion lands on the beat.
    tf("dance", "dance (beat-locked)", &[(S::BeatKick, 1.0)], 0.005, 0.12),
    // How clearly there is a beat to move to: slow, like a section.
    // Curved, so only a clear beat counts.
    TransformSpec { curve: 2.0, ..tf("pulse", "pulse (how clear the beat is)", &[(S::PulseClarity, 1.0)], 1.0, 2.0) },
    // A singer: there for the phrase, not the syllable.
    tf("singing", "singing", &[(S::Singing, 1.0)], 0.5, 1.5),
    // When a crowd would start a mosh pit: the music crashing back in (a drop,
    // or rap bursting in after quiet), and for as long as it stays loud, heavy
    // and on a beat. Sudden, and it lingers a few seconds.
    tf("havoc", "havoc (mosh pit)", &[(S::Drop, 1.0), (S::Mosh, 0.5), (S::RapEntry, 0.6), (S::EveryoneIn, 0.6)], 0.02, 3.0),
    // The lead line's pitch, smoothed over a note or two.
    tf("melody", "melody (lead pitch)", &[(S::MelodyPitch, 1.0)], 0.3, 0.5),
    // Harmony. Each chord change: sudden, fading over a second.
    tf("chords", "chords (each change)", &[(S::ChordChange, 1.0)], 0.02, 0.8),
    // Minor (0) to major (1), over a chord or two.
    tf("mood", "mood (minor..major)", &[(S::Major, 1.0)], 0.8, 0.8),
    tf("tension", "tension (wanting to resolve)", &[(S::Tension, 1.0)], 0.4, 0.6),
    tf("harmony", "harmony (how clear the chord is)", &[(S::Tonality, 1.0)], 0.5, 1.0),
    // The aspects: how much of the band plays, how much it makes you move,
    // how heavy it sounds, an instrument in front.
    tf("fullness", "fullness (how much of the band plays)", &[(S::Fullness, 1.0)], 0.5, 1.0),
    tf("drive", "drive (makes you move)", &[(S::Drive, 1.0)], 0.4, 0.8),
    tf("heaviness", "heaviness", &[(S::Heaviness, 1.0)], 0.5, 2.0),
    tf("solo", "solo (an instrument in front)", &[(S::Solo, 1.0)], 0.5, 1.5),
    // Someone comes in (a layer, or the whole band at once): sudden, and it
    // lingers a second or two. And the opposite: tension lets go, layers leave.
    tf("arrival", "arrival (a layer or the band comes in)", &[(S::Entrance, 0.6), (S::EveryoneIn, 1.0)], 0.02, 1.5),
    tf("let_go", "letting go (tension released, layers leave)", &[(S::Release, 1.0), (S::Exit, 0.5)], 0.02, 1.5),
];

/// Current output of every transform, 0..1.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct Transforms(pub [f32; Transformation::COUNT]);

impl Transforms {
    pub fn get(&self, t: Transformation) -> f32 {
        self.0[t as usize]
    }

    /// Advance every transform by `dt` seconds.
    pub fn step(&mut self, tuning: &Tuning, signals: &Signals, dt: f32) {
        for t in Transformation::ALL {
            let i = t as usize;
            let mix: f32 = Signal::ALL.iter().map(|&s| tuning.weights[i][s as usize] * signals.get(s)).sum();
            let target = mix.clamp(0.0, 1.0).powf(tuning.curve[i]);
            self.0[i] = follow(self.0[i], target, dt, tuning.attack[i], tuning.release[i]);
        }
    }
}

// ===========================================================================
// Controls and constants: each scene has its own (see `scene.rs`), described
// by these specs.

/// A quantity of a scene's world that the music can move: a base value plus
/// an amount of each transformation, in the scene's own units.
pub struct ControlSpec {
    pub key: &'static str,
    pub label: &'static str,
    /// Heading in the preferences panel (one of the scene's groups).
    pub group: &'static str,
    pub base: f32,
    pub base_range: (f32, f32),
    /// Amount sliders go from −max to +max.
    pub amount_max: f32,
    pub clamp: (f32, f32),
    /// Transforms enter as −1..1 (0.5 neutral): for directions and positions.
    pub bipolar: bool,
    /// Value used when another control is soloed (holds this one still).
    pub neutral: f32,
    pub routes: &'static [(Transformation, f32)],
    pub decimals: i32,
}

#[allow(clippy::too_many_arguments)]
pub const fn ctl(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    base: f32,
    base_range: (f32, f32),
    amount_max: f32,
    clamp: (f32, f32),
    neutral: f32,
    routes: &'static [(Transformation, f32)],
    decimals: i32,
) -> ControlSpec {
    ControlSpec { key, label, group, base, base_range, amount_max, clamp, bipolar: false, neutral, routes, decimals }
}

/// A plain constant of a scene's world (bubble count, lead width, …).
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub default: f32,
    pub range: (f32, f32),
    pub decimals: i32,
}

pub const fn param(key: &'static str, label: &'static str, group: &'static str, default: f32, range: (f32, f32), decimals: i32) -> ParamSpec {
    ParamSpec { key, label, group, default, range, decimals }
}

// ===========================================================================
// The live values.

/// One scene's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTuning {
    pub params: Vec<f32>,
    /// Per control: base value and transform amounts.
    pub base: Vec<f32>,
    pub amounts: Vec<[f32; Transformation::COUNT]>,
}

impl SceneTuning {
    fn defaults(scene: Scene) -> Self {
        let t = scene.table();
        let amounts = t
            .controls
            .iter()
            .map(|spec| {
                let mut a = [0.0; Transformation::COUNT];
                for &(tr, v) in spec.routes {
                    a[tr as usize] = v;
                }
                a
            })
            .collect();
        Self {
            params: t.params.iter().map(|p| p.default).collect(),
            base: t.controls.iter().map(|c| c.base).collect(),
            amounts,
        }
    }
}

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Tuning {
    /// Per transform: signal weights, response curve, attack and release (s).
    pub weights: [[f32; Signal::COUNT]; Transformation::COUNT],
    pub curve: [f32; Transformation::COUNT],
    pub attack: [f32; Transformation::COUNT],
    pub release: [f32; Transformation::COUNT],
    /// Per scene, in the order of [`Scene::ALL`].
    pub scenes: [SceneTuning; Scene::COUNT],
    /// The scene shown at start (the last one chosen).
    pub scene: Scene,
    /// Show the spectrum histogram at the bottom.
    pub show_spectrum: bool,
}

/// Written to the preferences file. 2: energy is its own signal (it was
/// loudness), and reads lower.
const PREFS_VERSION: u32 = 2;


impl Default for Tuning {
    fn default() -> Self {
        let mut weights = [[0.0; Signal::COUNT]; Transformation::COUNT];
        for (i, spec) in TRANSFORMS.iter().enumerate() {
            for &(s, w) in spec.routes {
                weights[i][s as usize] = w;
            }
        }
        Self {
            weights,
            curve: TRANSFORMS.map(|t| t.curve),
            attack: TRANSFORMS.map(|t| t.attack),
            release: TRANSFORMS.map(|t| t.release),
            scenes: Scene::ALL.map(SceneTuning::defaults),
            scene: Scene::default(),
            show_spectrum: false,
        }
    }
}

/// One adjustable number: what a slider or a preferences line refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Knob {
    Param(ParamId),
    Weight(Transformation, Signal),
    Curve(Transformation),
    Attack(Transformation),
    Release(Transformation),
    Base(ControlId),
    Amount(ControlId, Transformation),
}

impl Knob {
    pub fn range(self) -> (f32, f32) {
        match self {
            Knob::Param(p) => p.spec().range,
            Knob::Weight(..) => (-1.0, 2.0),
            Knob::Curve(_) => (0.25, 4.0),
            Knob::Attack(_) => (0.005, 5.0),
            Knob::Release(_) => (0.01, 10.0),
            Knob::Base(c) => c.spec().base_range,
            Knob::Amount(c, _) => (-c.spec().amount_max, c.spec().amount_max),
        }
    }

    pub fn decimals(self) -> i32 {
        match self {
            Knob::Param(p) => p.spec().decimals,
            Knob::Weight(..) | Knob::Curve(_) => 2,
            Knob::Attack(_) | Knob::Release(_) => 3,
            Knob::Base(c) | Knob::Amount(c, _) => c.spec().decimals,
        }
    }

    pub fn key(self) -> String {
        match self {
            Knob::Param(p) => format!("{}.param.{}", p.scene.key(), p.spec().key),
            Knob::Weight(t, s) => format!("transform.{}.{}", t.spec().key, s.key()),
            Knob::Curve(t) => format!("transform.{}.curve", t.spec().key),
            Knob::Attack(t) => format!("transform.{}.attack", t.spec().key),
            Knob::Release(t) => format!("transform.{}.release", t.spec().key),
            Knob::Base(c) => format!("{}.control.{}.base", c.scene.key(), c.spec().key),
            Knob::Amount(c, t) => format!("{}.control.{}.{}", c.scene.key(), c.spec().key, t.spec().key),
        }
    }

    /// Every knob of every scene.
    pub fn all() -> Vec<Knob> {
        let mut v = Vec::new();
        for t in Transformation::ALL {
            v.extend([Knob::Curve(t), Knob::Attack(t), Knob::Release(t)]);
            v.extend(Signal::ALL.map(|s| Knob::Weight(t, s)));
        }
        for scene in Scene::ALL {
            v.extend(scene.params().map(Knob::Param));
            for c in scene.controls() {
                v.push(Knob::Base(c));
                v.extend(Transformation::ALL.map(|t| Knob::Amount(c, t)));
            }
        }
        v
    }
}

impl Tuning {
    pub fn scene(&self, scene: Scene) -> &SceneTuning {
        &self.scenes[scene as usize]
    }

    pub fn scene_mut(&mut self, scene: Scene) -> &mut SceneTuning {
        &mut self.scenes[scene as usize]
    }

    /// A scene's constant.
    #[inline]
    pub fn get<P: SceneParam>(&self, p: P) -> f32 {
        self.scene(P::SCENE).params[p.index()]
    }

    /// A control's base value (before the music).
    pub fn base<C: SceneControl>(&self, c: C) -> f32 {
        self.scene(C::SCENE).base[c.index()]
    }

    pub fn knob(&self, k: Knob) -> f32 {
        match k {
            Knob::Param(p) => self.scene(p.scene).params[p.index],
            Knob::Weight(t, s) => self.weights[t as usize][s as usize],
            Knob::Curve(t) => self.curve[t as usize],
            Knob::Attack(t) => self.attack[t as usize],
            Knob::Release(t) => self.release[t as usize],
            Knob::Base(c) => self.scene(c.scene).base[c.index],
            Knob::Amount(c, t) => self.scene(c.scene).amounts[c.index][t as usize],
        }
    }

    pub fn set_knob(&mut self, k: Knob, v: f32) {
        let (lo, hi) = k.range();
        let v = v.clamp(lo, hi);
        match k {
            Knob::Param(p) => self.scene_mut(p.scene).params[p.index] = v,
            Knob::Weight(t, s) => self.weights[t as usize][s as usize] = v,
            Knob::Curve(t) => self.curve[t as usize] = v,
            Knob::Attack(t) => self.attack[t as usize] = v,
            Knob::Release(t) => self.release[t as usize] = v,
            Knob::Base(c) => self.scene_mut(c.scene).base[c.index] = v,
            Knob::Amount(c, t) => self.scene_mut(c.scene).amounts[c.index][t as usize] = v,
        }
    }

    /// A control's value: base + Σ amount × transform (bipolar: × (2·out − 1)).
    pub fn control<C: SceneControl>(&self, c: C, transforms: &Transforms) -> f32 {
        self.control_value(c.id(), transforms)
    }

    pub fn control_value(&self, c: ControlId, transforms: &Transforms) -> f32 {
        let spec = c.spec();
        let st = self.scene(c.scene);
        let mut v = st.base[c.index];
        for t in Transformation::ALL {
            let a = st.amounts[c.index][t as usize];
            if a != 0.0 {
                let out = transforms.get(t);
                v += a * if spec.bipolar { 2.0 * out - 1.0 } else { out };
            }
        }
        v.clamp(spec.clamp.0, spec.clamp.1)
    }

    // --- Preferences file -------------------------------------------------

    pub fn to_text(&self) -> String {
        let mut s = String::from("# synestesia preferences (written by the app; Cmd-, to edit)\n");
        s += &format!("prefs.version = {PREFS_VERSION}\n");
        s += &format!("ui.spectrum = {}\n", self.show_spectrum as u8);
        s += &format!("ui.scene = {}\n", self.scene.key());
        for k in Knob::all() {
            s += &format!("{} = {}\n", k.key(), self.knob(k));
        }
        s
    }

    /// Apply `key = value` lines; unknown keys and bad lines are skipped.
    /// Keys from before scenes (`control.…`, `param.…`) are the bubbles' own.
    ///
    /// A file from before energy was measured on its own (no `prefs.version`)
    /// is brought along: what its energy transformation took from loudness it
    /// now takes from energy. (Amounts stay: energy sits about where loudness
    /// did, near 0.75 on most music.)
    pub fn apply_text(&mut self, text: &str) {
        let knobs: std::collections::HashMap<String, Knob> = Knob::all().into_iter().map(|k| (k.key(), k)).collect();
        let version = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == "prefs.version")
            .and_then(|(_, v)| v.trim().parse::<u32>().ok())
            .unwrap_or(1);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else { continue };
            let (key, val) = (key.trim(), val.trim());
            if key == "ui.scene" {
                if let Some(scene) = Scene::find(val) {
                    self.scene = scene;
                }
                continue;
            }
            let Ok(v) = val.parse::<f32>() else { continue };
            let key = if key.starts_with("control.") || key.starts_with("param.") {
                format!("{}.{key}", Scene::Bubbles.key())
            } else {
                key.to_string()
            };
            if key == "ui.spectrum" {
                self.show_spectrum = v != 0.0;
            } else if let Some(&k) = knobs.get(&key) {
                match k {
                    Knob::Weight(Transformation::Energy, Signal::Loudness) if version < 2 => {
                        self.set_knob(Knob::Weight(Transformation::Energy, Signal::Energy), v);
                        self.set_knob(k, 0.0);
                    }
                    _ => self.set_knob(k, v),
                }
            }
        }
    }

    pub fn load() -> Self {
        let mut t = Self::default();
        if let Some(path) = prefs_path()
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            t.apply_text(&text);
            info!("preferences loaded from {}", path.display());
        }
        t
    }

    pub fn save(&self) {
        let Some(path) = prefs_path() else { return };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, self.to_text()));
        if let Err(e) = result {
            warn!("could not save preferences to {}: {e}", path.display());
        }
    }
}

/// `~/Library/Application Support/synestesia/prefs.txt` on macOS,
/// `$XDG_CONFIG_HOME/synestesia/prefs.txt` (or `~/.config/…`) elsewhere.
pub fn prefs_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))?
    };
    Some(dir.join("synestesia").join("prefs.txt"))
}

// ===========================================================================
// Plugin: refresh signals and transforms every frame.

/// Systems that produce `Signals` and `Transforms`; consumers run after this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignalSet;

pub struct TuningPlugin;

impl Plugin for TuningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Signals>()
            .init_resource::<Transforms>()
            .init_resource::<MusicClock>()
            .init_resource::<Notes>()
            .configure_sets(Update, SignalSet.after(crate::features::FeatureSet))
            .add_systems(Update, update_signals.in_set(SignalSet));
    }
}

/// Musical time: a clock at the detected tempo, re-synchronised on every
/// detected beat, that counts the beats and guesses where each bar starts
/// (the "1"). Scenes use it to time motion to the music (a dance step on each
/// count); like a signal, it is music, not motion.
///
/// The 1 is the position in the bar (of 4) where the strongest beats have
/// fallen lately; it only moves when another position is clearly stronger.
#[derive(Resource, Default, Clone, Debug)]
pub struct MusicClock {
    /// Beats since the start: integers are beats.
    beats: f64,
    /// Recent beat strength at each position in the bar.
    accent: [f32; 4],
    /// The position in the bar taken to be the 1.
    one: usize,
}

/// Per beat, how much of the accent memory is kept (~50 beats, a dozen bars).
const ACCENT_KEEP: f32 = 0.98;
/// Another position becomes the 1 only when this much stronger. A steady 1
/// matters more than the right one: dancing "on 3" looks the same as on 1,
/// but every move of the 1 makes the dancers stumble.
const ONE_SWITCH: f32 = 1.6;

impl MusicClock {
    /// Advance by `dt` and return the beat kick: 1 on the beat, decaying to ~0
    /// before the next one, scaled by how clear the pulse is.
    pub fn step(&mut self, f: &AudioFeatures, dt: f32) -> f32 {
        let bpm = 60.0 + 120.0 * f.tempo as f64;
        self.beats += dt as f64 * bpm / 60.0;
        if f.beats > 0 {
            // Pull towards the detected beat (it can be a little early or late).
            let nearest = self.beats.round();
            self.beats += 0.8 * (nearest - self.beats);
            let pos = (nearest as i64).rem_euclid(4) as usize;
            for a in &mut self.accent {
                *a *= ACCENT_KEEP;
            }
            self.accent[pos] += f.beat_strength;
            let best = (0..4).max_by(|&a, &b| self.accent[a].total_cmp(&self.accent[b])).unwrap();
            if self.accent[best] > ONE_SWITCH * self.accent[self.one] {
                self.one = best;
            }
        }
        let phase = self.beats.fract() as f32;
        (-5.0 * phase).exp() * f.pulse_clarity.clamp(0.0, 1.0).sqrt()
    }

    /// The position in the bar (0..4) taken to be the 1.
    pub fn one(&self) -> usize {
        self.one
    }

    /// Where we are in a phrase of `counts` beats: 0 on the 1, then 1, 2, …
    /// (fractions between beats).
    pub fn count(&self, counts: u32) -> f32 {
        (self.beats - self.one as f64).rem_euclid(counts as f64) as f32
    }
}

/// The shape of the music, for scenes that draw it: the twelve pitch classes,
/// the harmony's place on the circle of fifths, and the spectrum. Like the
/// music clock, this is music, not motion; it is here because none of it fits
/// a transformation (many values, or a value that goes round).
#[derive(Resource, Default, Clone, Debug)]
pub struct Notes {
    /// How strongly each pitch class sounds (C … B), 0..1: quick to light,
    /// slower to fade, and dark when there is no chord to hear.
    pub chroma: [f32; 12],
    /// Place on the circle of fifths, 0..1 around (see
    /// `AudioFeatures::harmony_hue`), moving the short way round.
    pub hue: f32,
    /// The hue as a point on the circle (shorter when the harmony is unclear).
    point: Vec2,
    /// The spectrum (`AudioFeatures::spectrum`), eased: quick up, slow down.
    pub spectrum: [f32; crate::features::SPECTRUM_BINS],
}

impl Notes {
    pub fn step(&mut self, f: &AudioFeatures, dt: f32) {
        for (v, &c) in self.chroma.iter_mut().zip(&f.chroma) {
            *v = follow(*v, c * f.tonality, dt, 0.08, 0.5);
        }
        for (v, &s) in self.spectrum.iter_mut().zip(&f.spectrum) {
            *v = follow(*v, s, dt, 0.06, 0.7);
        }
        let angle = f.harmony_hue * std::f32::consts::TAU;
        let target = Vec2::from_angle(angle) * f.tonality;
        self.point += (target - self.point) * (1.0 - (-dt / 0.6).exp());
        if self.point.length() > 0.02 {
            self.hue = (self.point.to_angle() / std::f32::consts::TAU).rem_euclid(1.0);
        }
    }

    /// The pitch class at place `i` (0..12) on the circle of fifths: C, G, D, …
    pub fn fifth(i: usize) -> usize {
        i * 7 % 12
    }
}

fn update_signals(
    mut clock: ResMut<MusicClock>,
    mut notes: ResMut<Notes>,
    time: Res<Time>,
    features: Res<AudioFeatures>,
    tuning: Res<Tuning>,
    mut signals: ResMut<Signals>,
    mut transforms: ResMut<Transforms>,
) {
    *signals = Signals::from_features(&features);
    signals.0[Signal::BeatKick as usize] = clock.step(&features, time.delta_secs().min(0.1));
    notes.step(&features, time.delta_secs().min(0.1));
    transforms.step(&tuning, &signals, time.delta_secs().min(0.1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bubble_controls as bubbles, glass};

    #[test]
    fn tables_follow_enum_order_and_keys_are_unique() {
        let keys: Vec<String> = Knob::all().iter().map(|k| k.key()).collect();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(keys.len(), unique.len(), "duplicate preference keys");
        for (i, t) in Transformation::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i);
        }
        for (i, s) in Signal::ALL.iter().enumerate() {
            assert_eq!(*s as usize, i);
        }
        for (i, s) in Scene::ALL.iter().enumerate() {
            assert_eq!(*s as usize, i);
        }
    }

    #[test]
    fn preferences_round_trip() {
        let mut t = Tuning::default();
        t.set_knob(Knob::Param(bubbles::P::Bubbles.id()), 12000.0);
        t.set_knob(Knob::Param(glass::P::Panes.id()), 120.0);
        t.set_knob(Knob::Weight(Transformation::Groove, Signal::Onsets), 0.5);
        t.set_knob(Knob::Release(Transformation::Energy), 4.0);
        t.set_knob(Knob::Amount(bubbles::Control::WindSpeed.id(), Transformation::Sparkle), 0.05);
        t.set_knob(Knob::Amount(glass::Control::Breath.id(), Transformation::Dance), -0.4);
        t.show_spectrum = true;
        t.scene = Scene::Glass;
        let mut back = Tuning::default();
        back.apply_text(&t.to_text());
        assert_eq!(t, back);
    }

    #[test]
    fn preferences_from_before_scenes_are_the_bubbles() {
        let mut t = Tuning::default();
        t.apply_text("param.bubbles = 9000\ncontrol.wind_speed.base = 0.2\ncontrol.wind_speed.groove = 0.3\ntransform.energy.attack = 1.5\n");
        assert_eq!(t.get(bubbles::P::Bubbles), 9000.0);
        assert_eq!(t.base(bubbles::Control::WindSpeed), 0.2);
        assert_eq!(t.knob(Knob::Amount(bubbles::Control::WindSpeed.id(), Transformation::Groove)), 0.3);
        assert_eq!(t.attack[Transformation::Energy as usize], 1.5);
        assert_eq!(t.scene, Scene::Bubbles);
    }

    #[test]
    fn preferences_from_before_energy_follow_it() {
        let wind = Knob::Amount(bubbles::Control::WindSpeed.id(), Transformation::Energy);
        let light = Knob::Amount(glass::Control::Backlight.id(), Transformation::Energy);
        let from_loudness = Knob::Weight(Transformation::Energy, Signal::Loudness);
        let from_energy = Knob::Weight(Transformation::Energy, Signal::Energy);
        let old = "transform.energy.loudness = 0.8\nbubbles.control.wind_speed.energy = 0.1\nglass.control.backlight.energy = 0.9\n";
        let mut t = Tuning::default();
        t.apply_text(old);
        assert_eq!(t.knob(from_loudness), 0.0);
        assert_eq!(t.knob(from_energy), 0.8);
        assert_eq!(t.knob(wind), 0.1);
        assert_eq!(t.knob(light), 0.9);
        // Saved again, it is read as it is.
        let mut back = Tuning::default();
        back.apply_text(&t.to_text());
        assert_eq!(t, back);
        let mut new = Tuning::default();
        new.apply_text(&format!("prefs.version = 2\n{old}"));
        assert_eq!(new.knob(from_loudness), 0.8);
        assert_eq!(new.knob(wind), 0.1);
    }

    /// Run by hand (`cargo test -- --ignored`): every value in your saved
    /// preferences file is understood (old keys included) and kept.
    #[test]
    #[ignore]
    fn saved_preferences_load_without_loss() {
        let Some(text) = prefs_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return };
        let mut t = Tuning::default();
        t.apply_text(&text);
        let written = t.to_text();
        let mut lost = Vec::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let Some((key, val)) = line.split_once('=') else { continue };
            let (key, val) = (key.trim(), val.trim());
            let key = if key.starts_with("control.") || key.starts_with("param.") { format!("bubbles.{key}") } else { key.to_string() };
            if !written.lines().any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim() == key && v.trim() == val)) {
                lost.push(format!("{key} = {val}"));
            }
        }
        assert!(lost.is_empty(), "not kept: {lost:#?}");
    }

    #[test]
    fn music_clock_counts_beats_and_finds_the_one() {
        let mut clock = MusicClock::default();
        let mut f = AudioFeatures { tempo: 0.5, pulse_clarity: 1.0, ..Default::default() }; // 120 BPM
        let dt = 1.0 / 120.0;
        // Beats every 0.5 s, the strongest on every 4th, starting on the 3rd
        // beat of the clock's own count, so the 1 is at position 2.
        let mut next_beat = 0.5f32;
        let mut n = 0;
        let mut t = 0.0;
        while t < 40.0 {
            t += dt;
            f.beats = 0;
            if t >= next_beat {
                next_beat += 0.5;
                n += 1;
                f.beats = 1;
                f.beat_strength = if n % 4 == 2 { 1.0 } else { 0.5 };
            }
            clock.step(&f, dt);
            // Just after a beat the count is near a whole number.
            if f.beats > 0 && t > 20.0 {
                let c = clock.count(4);
                let off = (c - c.round()).abs();
                assert!(off < 0.1, "count {c} off the beat");
            }
        }
        // Just after an accented beat, the count is 0 (on the 1).
        let mut t2 = 0.0;
        loop {
            t2 += dt;
            f.beats = 0;
            if t + t2 >= next_beat {
                next_beat += 0.5;
                n += 1;
                f.beats = 1;
                f.beat_strength = if n % 4 == 2 { 1.0 } else { 0.5 };
            }
            clock.step(&f, dt);
            if f.beats > 0 && n % 4 == 2 {
                let c = clock.count(4);
                assert!(c < 0.1 || c > 3.9, "accented beat at count {c}");
                break;
            }
        }
    }

    #[test]
    fn signals_flow_through_transforms_to_controls() {
        use bubbles::Control;
        let tuning = Tuning::default();
        let mut signals = Signals::default();
        let mut tr = Transforms::default();
        // Silence: wind at base.
        tr.step(&tuning, &signals, 0.01);
        assert!((tuning.control(Control::WindSpeed, &tr) - 0.05).abs() < 1e-4);
        // Full energy for a long time: the transformation reaches 1, wind gains 0.10.
        signals.0[Signal::Energy as usize] = 1.0;
        for _ in 0..2000 {
            tr.step(&tuning, &signals, 0.01);
        }
        assert!((tr.get(Transformation::Energy) - 1.0).abs() < 1e-3);
        assert!((tuning.control(Control::WindSpeed, &tr) - 0.15).abs() < 1e-3);
        // A bipolar control: brightness 0.5 is neutral for the small vortex height.
        signals.0[Signal::Brightness as usize] = 0.5;
        for _ in 0..500 {
            tr.step(&tuning, &signals, 0.01);
        }
        assert!(tuning.control(Control::VortexSmallHeight, &tr).abs() < 1e-3);
        // The same transformations drive the glass window.
        assert!((tuning.control(glass::Control::Backlight, &tr) - 1.0).abs() < 1e-3);
        // Attack and release: a beat pulse rises fast and coasts down.
        let mut tr = Transforms::default();
        signals.0[Signal::Beat as usize] = 1.0;
        tr.step(&tuning, &signals, 0.03);
        let up = tr.get(Transformation::Groove);
        signals.0[Signal::Beat as usize] = 0.0;
        tr.step(&tuning, &signals, 0.03);
        assert!(up > 0.6 && tr.get(Transformation::Groove) > 0.9 * up, "{up} -> {}", tr.get(Transformation::Groove));
    }
}
