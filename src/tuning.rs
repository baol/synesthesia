//! From music to motion, in three layers that never mix their languages:
//!
//! ```text
//! signals (music)  ──weights──▶  transforms  ──amounts──▶  controls (bubble physics)
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
//! - A **control** is a physical quantity of the bubble world: wind speed,
//!   a vortex's circulation, the plate's vibration, the hop impulse… It is a
//!   base value plus an amount of each transform, in physical units.
//!
//! Plain physical constants (bubble count, radius, drag time, …) are
//! parameters. All of it is tables below: the simulation reads them, the
//! preferences panel draws them, and the preferences file stores them by key.

use std::path::PathBuf;

use bevy::prelude::*;

use crate::features::{AudioFeatures, follow};

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
}

impl Signal {
    pub const COUNT: usize = 13;
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
        }
    }
}

/// Current value of every signal.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct Signals(pub [f32; Signal::COUNT]);

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
    pub const COUNT: usize = 10;
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
    ];

    pub fn spec(self) -> &'static TransformSpec {
        &TRANSFORMS[self as usize]
    }
}

/// In the order of [`Transformation`].
pub const TRANSFORMS: [TransformSpec; Transformation::COUNT] = [
    tf("energy", "energy", &[(S::Loudness, 1.0)], 0.8, 2.0),
    tf("groove", "groove", &[(S::Beat, 1.0)], 0.03, 0.45),
    tf("low_end", "low end", &[(S::Bass, 1.0)], 0.01, 0.25),
    tf("low_mids", "low mids", &[(S::LowMids, 1.0)], 0.05, 0.4),
    tf("high_mids", "high mids", &[(S::HighMids, 1.0)], 0.05, 0.4),
    tf("attacks", "attacks", &[(S::Onsets, 1.0)], 0.01, 0.3),
    tf("busyness", "busyness", &[(S::NoteDensity, 1.0)], 1.5, 3.0),
    tf("brightness", "brightness", &[(S::Brightness, 1.0)], 0.1, 0.3),
    tf("sparkle", "sparkle", &[(S::Treble, 1.0)], 0.01, 0.2),
    tf("pitch", "pitch", &[(S::DominantPitch, 1.0)], 0.5, 0.5),
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
// Controls: the physics of the bubble world.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Inflow,
    Wind,
    Vortices,
    Turbulence,
    Plate,
    Bubbles,
}

impl Group {
    pub const ALL: [Group; 6] = [Group::Inflow, Group::Wind, Group::Vortices, Group::Turbulence, Group::Plate, Group::Bubbles];

    pub fn label(self) -> &'static str {
        match self {
            Group::Inflow => "INFLOW",
            Group::Wind => "WIND",
            Group::Vortices => "VORTICES",
            Group::Turbulence => "TURBULENCE",
            Group::Plate => "VIBRATING PLATE",
            Group::Bubbles => "BUBBLES",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    InflowRate,
    WindSpeed,
    WindHeading,
    VortexLarge,
    VortexMedium,
    VortexSmall,
    VortexTiny,
    VortexDrift,
    VortexSmallHeight,
    TurbulenceIntensity,
    TurbulenceEvolution,
    PlateVibration,
    PlateMode,
    HopImpulse,
    BubbleScale,
}

pub struct ControlSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
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

use Transformation as T;

#[allow(clippy::too_many_arguments)]
const fn ctl(
    key: &'static str,
    label: &'static str,
    group: Group,
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

const fn vortex(key: &'static str, label: &'static str, routes: &'static [(Transformation, f32)]) -> ControlSpec {
    ctl(key, label, Group::Vortices, 0.0, (0.0, 1.0), 1.5, (0.0, 1.0), 0.0, routes, 2)
}

impl Control {
    pub const COUNT: usize = 15;
    pub const ALL: [Control; Self::COUNT] = [
        Control::InflowRate,
        Control::WindSpeed,
        Control::WindHeading,
        Control::VortexLarge,
        Control::VortexMedium,
        Control::VortexSmall,
        Control::VortexTiny,
        Control::VortexDrift,
        Control::VortexSmallHeight,
        Control::TurbulenceIntensity,
        Control::TurbulenceEvolution,
        Control::PlateVibration,
        Control::PlateMode,
        Control::HopImpulse,
        Control::BubbleScale,
    ];
    /// The four vortices, largest first.
    pub const VORTICES: [Control; 4] = [Control::VortexLarge, Control::VortexMedium, Control::VortexSmall, Control::VortexTiny];

    pub fn spec(self) -> &'static ControlSpec {
        &CONTROLS[self as usize]
    }

    pub fn find(key: &str) -> Option<Control> {
        Control::ALL.into_iter().find(|c| c.spec().key == key)
    }
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    ctl("inflow_rate", "inflow rate (x)", Group::Inflow, 0.2, (0.0, 2.0), 3.0, (0.0, 5.0), 1.0, &[(T::Energy, 1.6)], 2),
    ctl(
        "wind_speed",
        "wind speed (widths/s)",
        Group::Wind,
        0.05,
        (0.0, 0.3),
        0.3,
        (0.0, 0.6),
        0.10,
        &[(T::Energy, 0.10), (T::Groove, 0.12)],
        3,
    ),
    ControlSpec {
        bipolar: true,
        ..ctl("wind_heading", "wind heading (deg)", Group::Wind, 160.0, (90.0, 270.0), 45.0, (-720.0, 720.0), 160.0, &[], 1)
    },
    vortex("vortex_large", "large vortex circulation", &[(T::LowEnd, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_medium", "medium vortex circulation", &[(T::LowMids, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_small", "small vortex circulation", &[(T::HighMids, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_tiny", "tiny vortex circulation", &[(T::Attacks, 1.0)]),
    ctl("vortex_drift", "vortex drift speed (x)", Group::Vortices, 0.5, (0.0, 3.0), 4.0, (0.0, 6.0), 1.0, &[(T::Busyness, 1.5)], 2),
    ControlSpec {
        bipolar: true,
        ..ctl(
            "vortex_small_height",
            "small vortex height (x h)",
            Group::Vortices,
            0.0,
            (-0.5, 0.5),
            1.0,
            (-0.6, 0.6),
            0.0,
            &[(T::Brightness, 0.4)],
            2,
        )
    },
    ctl(
        "turbulence_intensity",
        "turbulence intensity (x wind)",
        Group::Turbulence,
        0.10,
        (0.0, 0.6),
        0.6,
        (0.0, 1.0),
        0.0,
        &[(T::Sparkle, 0.09)],
        3,
    ),
    ctl(
        "turbulence_evolution",
        "eddy evolution rate (x)",
        Group::Turbulence,
        0.5,
        (0.0, 3.0),
        4.0,
        (0.0, 6.0),
        1.0,
        &[(T::Busyness, 2.0)],
        2,
    ),
    ctl(
        "plate_vibration",
        "vibration amplitude",
        Group::Plate,
        0.0,
        (0.0, 1.0),
        1.5,
        (0.0, 1.5),
        0.0,
        &[(T::Groove, 0.6), (T::Attacks, 0.4)],
        2,
    ),
    ctl("plate_mode", "vibration mode (simple..complex)", Group::Plate, 0.0, (0.0, 1.0), 1.0, (0.0, 1.0), 0.0, &[(T::Pitch, 1.0)], 2),
    ctl("hop_impulse", "hop impulse (radii)", Group::Bubbles, 0.0, (0.0, 2.0), 4.0, (0.0, 4.0), 0.0, &[(T::Groove, 0.8)], 2),
    ctl("bubble_scale", "bubble scale (x)", Group::Bubbles, 1.0, (0.3, 2.0), 1.5, (0.3, 3.0), 1.0, &[], 2),
];

// ===========================================================================
// Physical constants.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    Bubbles,
    Radius,
    DragTime,
    VortexPeak,
    VortexCap,
    PlatePull,
    PlateBounce,
    PlateModeChange,
    PlateSize,
    HopTime,
    SizeSpread,
    SizeClip,
}

pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub default: f32,
    pub range: (f32, f32),
    pub decimals: i32,
}

impl P {
    pub const COUNT: usize = 12;
    pub const ALL: [P; Self::COUNT] = [
        P::Bubbles,
        P::Radius,
        P::DragTime,
        P::VortexPeak,
        P::VortexCap,
        P::PlatePull,
        P::PlateBounce,
        P::PlateModeChange,
        P::PlateSize,
        P::HopTime,
        P::SizeSpread,
        P::SizeClip,
    ];

    pub fn spec(self) -> &'static ParamSpec {
        &PARAMS[self as usize]
    }
}

const fn param(key: &'static str, label: &'static str, group: Group, default: f32, range: (f32, f32), decimals: i32) -> ParamSpec {
    ParamSpec { key, label, group, default, range, decimals }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    param("bubbles", "bubbles on screen", Group::Inflow, 5000.0, (500.0, 40000.0), 0),
    param("radius", "radius (px)", Group::Bubbles, 12.0, (4.0, 32.0), 1),
    param("drag_time", "drag time (s)", Group::Bubbles, 0.25, (0.03, 1.5), 2),
    param("vortex_peak", "peak speed (widths/s)", Group::Vortices, 0.12, (0.0, 0.5), 3),
    param("vortex_cap", "speed cap (x wind)", Group::Vortices, 1.5, (0.2, 4.0), 2),
    param("plate_pull", "pull to nodal lines (x wind)", Group::Plate, 0.5, (0.0, 3.0), 2),
    param("plate_bounce", "bounce swell", Group::Plate, 0.10, (0.0, 0.5), 2),
    param("plate_mode_change", "mode change time (s)", Group::Plate, 1.5, (0.2, 6.0), 1),
    param("plate_size", "plate size", Group::Plate, 0.7, (0.4, 2.0), 2),
    param("hop_time", "hop duration (s)", Group::Bubbles, 0.3, (0.1, 1.0), 2),
    param("size_spread", "size spread (sigma, x radius)", Group::Bubbles, 0.05, (0.0, 0.3), 3),
    param("size_clip", "size clip (max deviation, x radius)", Group::Bubbles, 0.10, (0.0, 0.6), 2),
];

// ===========================================================================
// The live values.

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Tuning {
    pub params: [f32; P::COUNT],
    /// Per transform: signal weights, response curve, attack and release (s).
    pub weights: [[f32; Signal::COUNT]; Transformation::COUNT],
    pub curve: [f32; Transformation::COUNT],
    pub attack: [f32; Transformation::COUNT],
    pub release: [f32; Transformation::COUNT],
    /// Per control: base value and transform amounts.
    pub base: [f32; Control::COUNT],
    pub amounts: [[f32; Transformation::COUNT]; Control::COUNT],
    /// Show the spectrum histogram at the bottom.
    pub show_spectrum: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        let mut weights = [[0.0; Signal::COUNT]; Transformation::COUNT];
        for (i, spec) in TRANSFORMS.iter().enumerate() {
            for &(s, w) in spec.routes {
                weights[i][s as usize] = w;
            }
        }
        let mut amounts = [[0.0; Transformation::COUNT]; Control::COUNT];
        for (i, spec) in CONTROLS.iter().enumerate() {
            for &(t, a) in spec.routes {
                amounts[i][t as usize] = a;
            }
        }
        Self {
            params: PARAMS.map(|p| p.default),
            weights,
            curve: TRANSFORMS.map(|t| t.curve),
            attack: TRANSFORMS.map(|t| t.attack),
            release: TRANSFORMS.map(|t| t.release),
            base: CONTROLS.map(|c| c.base),
            amounts,
            show_spectrum: false,
        }
    }
}

/// One adjustable number: what a slider or a preferences line refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Knob {
    Param(P),
    Weight(Transformation, Signal),
    Curve(Transformation),
    Attack(Transformation),
    Release(Transformation),
    Base(Control),
    Amount(Control, Transformation),
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
            Knob::Param(p) => format!("param.{}", p.spec().key),
            Knob::Weight(t, s) => format!("transform.{}.{}", t.spec().key, s.key()),
            Knob::Curve(t) => format!("transform.{}.curve", t.spec().key),
            Knob::Attack(t) => format!("transform.{}.attack", t.spec().key),
            Knob::Release(t) => format!("transform.{}.release", t.spec().key),
            Knob::Base(c) => format!("control.{}.base", c.spec().key),
            Knob::Amount(c, t) => format!("control.{}.{}", c.spec().key, t.spec().key),
        }
    }

    /// Every knob.
    pub fn all() -> Vec<Knob> {
        let mut v: Vec<Knob> = P::ALL.into_iter().map(Knob::Param).collect();
        for t in Transformation::ALL {
            v.extend([Knob::Curve(t), Knob::Attack(t), Knob::Release(t)]);
            v.extend(Signal::ALL.map(|s| Knob::Weight(t, s)));
        }
        for c in Control::ALL {
            v.push(Knob::Base(c));
            v.extend(Transformation::ALL.map(|t| Knob::Amount(c, t)));
        }
        v
    }
}

impl Tuning {
    #[inline]
    pub fn get(&self, p: P) -> f32 {
        self.params[p as usize]
    }

    pub fn knob(&self, k: Knob) -> f32 {
        match k {
            Knob::Param(p) => self.params[p as usize],
            Knob::Weight(t, s) => self.weights[t as usize][s as usize],
            Knob::Curve(t) => self.curve[t as usize],
            Knob::Attack(t) => self.attack[t as usize],
            Knob::Release(t) => self.release[t as usize],
            Knob::Base(c) => self.base[c as usize],
            Knob::Amount(c, t) => self.amounts[c as usize][t as usize],
        }
    }

    pub fn set_knob(&mut self, k: Knob, v: f32) {
        let (lo, hi) = k.range();
        let v = v.clamp(lo, hi);
        match k {
            Knob::Param(p) => self.params[p as usize] = v,
            Knob::Weight(t, s) => self.weights[t as usize][s as usize] = v,
            Knob::Curve(t) => self.curve[t as usize] = v,
            Knob::Attack(t) => self.attack[t as usize] = v,
            Knob::Release(t) => self.release[t as usize] = v,
            Knob::Base(c) => self.base[c as usize] = v,
            Knob::Amount(c, t) => self.amounts[c as usize][t as usize] = v,
        }
    }

    /// A control's value: base + Σ amount × transform (bipolar: × (2·out − 1)).
    pub fn control(&self, c: Control, transforms: &Transforms) -> f32 {
        let spec = c.spec();
        let i = c as usize;
        let mut v = self.base[i];
        for t in Transformation::ALL {
            let a = self.amounts[i][t as usize];
            if a != 0.0 {
                let out = transforms.get(t);
                v += a * if spec.bipolar { 2.0 * out - 1.0 } else { out };
            }
        }
        v.clamp(spec.clamp.0, spec.clamp.1)
    }

    /// Base wind heading (the inflow line is perpendicular to it).
    pub fn wind_dir(&self) -> Vec2 {
        Vec2::from_angle(self.base[Control::WindHeading as usize].to_radians())
    }

    // --- Preferences file -------------------------------------------------

    pub fn to_text(&self) -> String {
        let mut s = String::from("# synestesia preferences (written by the app; Cmd-, to edit)\n");
        s += &format!("ui.spectrum = {}\n", self.show_spectrum as u8);
        for k in Knob::all() {
            s += &format!("{} = {}\n", k.key(), self.knob(k));
        }
        s
    }

    /// Apply `key = value` lines; unknown keys and bad lines are skipped.
    pub fn apply_text(&mut self, text: &str) {
        let knobs: std::collections::HashMap<String, Knob> = Knob::all().into_iter().map(|k| (k.key(), k)).collect();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else { continue };
            let Ok(v) = val.trim().parse::<f32>() else { continue };
            let key = key.trim();
            if key == "ui.spectrum" {
                self.show_spectrum = v != 0.0;
            } else if let Some(&k) = knobs.get(key) {
                self.set_knob(k, v);
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
            .configure_sets(Update, SignalSet.after(crate::features::FeatureSet))
            .add_systems(Update, update_signals.in_set(SignalSet));
    }
}

fn update_signals(
    time: Res<Time>,
    features: Res<AudioFeatures>,
    tuning: Res<Tuning>,
    mut signals: ResMut<Signals>,
    mut transforms: ResMut<Transforms>,
) {
    *signals = Signals::from_features(&features);
    transforms.step(&tuning, &signals, time.delta_secs().min(0.1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_follow_enum_order_and_keys_are_unique() {
        let keys: Vec<String> = Knob::all().iter().map(|k| k.key()).collect();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(keys.len(), unique.len(), "duplicate preference keys");
        for (i, p) in P::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
        }
        for (i, t) in Transformation::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i);
        }
        for (i, c) in Control::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
        }
        for (i, s) in Signal::ALL.iter().enumerate() {
            assert_eq!(*s as usize, i);
        }
        for spec in &CONTROLS {
            for &(_, a) in spec.routes {
                assert!(a.abs() <= spec.amount_max, "{} default amount out of range", spec.key);
            }
            assert!(spec.base >= spec.base_range.0 && spec.base <= spec.base_range.1, "{}", spec.key);
        }
    }

    #[test]
    fn preferences_round_trip() {
        let mut t = Tuning::default();
        t.set_knob(Knob::Param(P::Bubbles), 12000.0);
        t.set_knob(Knob::Weight(Transformation::Groove, Signal::Onsets), 0.5);
        t.set_knob(Knob::Release(Transformation::Energy), 4.0);
        t.set_knob(Knob::Amount(Control::WindSpeed, Transformation::Sparkle), 0.05);
        t.show_spectrum = true;
        let mut back = Tuning::default();
        back.apply_text(&t.to_text());
        assert_eq!(t, back);
    }

    #[test]
    fn signals_flow_through_transforms_to_controls() {
        let tuning = Tuning::default();
        let mut signals = Signals::default();
        let mut tr = Transforms::default();
        // Silence: wind at base.
        tr.step(&tuning, &signals, 0.01);
        assert!((tuning.control(Control::WindSpeed, &tr) - 0.05).abs() < 1e-4);
        // Full loudness for a long time: energy reaches 1, wind gains 0.10.
        signals.0[Signal::Loudness as usize] = 1.0;
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
