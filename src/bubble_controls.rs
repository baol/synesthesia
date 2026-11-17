//! The bubbles scene's controls and constants: the physics of the bubble world
//! (inflow, wind, vortices, turbulence, the vibrating plate, the bubbles).

use bevy::prelude::*;

use crate::{
    scene::{Scene, SceneControl, SceneParam, SceneTable},
    tuning::{ControlSpec, ParamSpec, Transformation as T, Tuning, ctl, param},
};

const INFLOW: &str = "INFLOW";
const WIND: &str = "WIND";
const VORTICES: &str = "VORTICES";
const TURBULENCE: &str = "TURBULENCE";
const PLATE: &str = "VIBRATING PLATE";
const BUBBLES: &str = "BUBBLES";

pub const TABLE: SceneTable = SceneTable {
    groups: &[INFLOW, WIND, VORTICES, TURBULENCE, PLATE, BUBBLES],
    controls: &CONTROLS,
    params: &PARAMS,
};

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

impl Control {
    pub const COUNT: usize = 15;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
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
}

impl SceneControl for Control {
    const SCENE: Scene = Scene::Bubbles;
    fn index(self) -> usize {
        self as usize
    }
}

const fn vortex(key: &'static str, label: &'static str, routes: &'static [(T, f32)]) -> ControlSpec {
    ctl(key, label, VORTICES, 0.0, (0.0, 1.0), 1.5, (0.0, 1.0), 0.0, routes, 2)
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    ctl("inflow_rate", "inflow rate (x)", INFLOW, 0.2, (0.0, 2.0), 3.0, (0.0, 5.0), 1.0, &[(T::Energy, 1.6)], 2),
    ctl(
        "wind_speed",
        "wind speed (widths/s)",
        WIND,
        0.05,
        (0.0, 0.3),
        0.3,
        (0.0, 0.6),
        0.10,
        &[(T::Energy, 0.10), (T::Groove, 0.12), (T::Tempo, 0.08), (T::Dance, 0.05)],
        3,
    ),
    ControlSpec {
        bipolar: true,
        ..ctl("wind_heading", "wind heading (deg)", WIND, 160.0, (90.0, 270.0), 45.0, (-720.0, 720.0), 160.0, &[], 1)
    },
    vortex("vortex_large", "large vortex circulation", &[(T::LowEnd, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_medium", "medium vortex circulation", &[(T::LowMids, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_small", "small vortex circulation", &[(T::HighMids, 0.8), (T::Energy, 0.2)]),
    vortex("vortex_tiny", "tiny vortex circulation", &[(T::Attacks, 1.0)]),
    ctl("vortex_drift", "vortex drift speed (x)", VORTICES, 0.5, (0.0, 3.0), 4.0, (0.0, 6.0), 1.0, &[(T::Busyness, 1.5)], 2),
    ControlSpec {
        bipolar: true,
        ..ctl(
            "vortex_small_height",
            "small vortex height (x h)",
            VORTICES,
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
        TURBULENCE,
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
        TURBULENCE,
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
        PLATE,
        0.0,
        (0.0, 1.0),
        1.5,
        (0.0, 1.5),
        0.0,
        &[(T::Groove, 0.6), (T::Attacks, 0.4)],
        2,
    ),
    ctl("plate_mode", "vibration mode (simple..complex)", PLATE, 0.0, (0.0, 1.0), 1.0, (0.0, 1.0), 0.0, &[(T::Pitch, 1.0)], 2),
    ctl("hop_impulse", "hop impulse (radii)", BUBBLES, 0.0, (0.0, 2.0), 4.0, (0.0, 4.0), 0.0, &[(T::Groove, 0.8)], 2),
    ctl("bubble_scale", "bubble scale (x)", BUBBLES, 1.0, (0.3, 2.0), 1.5, (0.3, 3.0), 1.0, &[(T::Dance, 0.12)], 2),
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

impl P {
    pub const COUNT: usize = 12;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
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
}

impl SceneParam for P {
    const SCENE: Scene = Scene::Bubbles;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    param("bubbles", "bubbles on screen", INFLOW, 5000.0, (500.0, 40000.0), 0),
    param("radius", "radius (px)", BUBBLES, 12.0, (4.0, 32.0), 1),
    param("drag_time", "drag time (s)", BUBBLES, 0.25, (0.03, 1.5), 2),
    param("vortex_peak", "peak speed (widths/s)", VORTICES, 0.12, (0.0, 0.5), 3),
    param("vortex_cap", "speed cap (x wind)", VORTICES, 1.5, (0.2, 4.0), 2),
    param("plate_pull", "pull to nodal lines (x wind)", PLATE, 0.5, (0.0, 3.0), 2),
    param("plate_bounce", "bounce swell", PLATE, 0.10, (0.0, 0.5), 2),
    param("plate_mode_change", "mode change time (s)", PLATE, 1.5, (0.2, 6.0), 1),
    param("plate_size", "plate size", PLATE, 0.7, (0.4, 2.0), 2),
    param("hop_time", "hop duration (s)", BUBBLES, 0.3, (0.1, 1.0), 2),
    param("size_spread", "size spread (sigma, x radius)", BUBBLES, 0.05, (0.0, 0.3), 3),
    param("size_clip", "size clip (max deviation, x radius)", BUBBLES, 0.10, (0.0, 0.6), 2),
];

/// Base wind heading (the inflow line is perpendicular to it).
pub fn wind_dir(tuning: &Tuning) -> Vec2 {
    Vec2::from_angle(tuning.base(Control::WindHeading).to_radians())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_follow_enum_order() {
        for (i, p) in P::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
        }
        for (i, c) in Control::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
        }
    }
}
