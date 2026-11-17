//! The conductor applies the wind, vortex and turbulence controls to the flow
//! field, and moves the vortices around.
//!
//! Four vortices of different sizes; spins alternate, so neighbours form
//! counter-rotating pairs. Each is carried along by part of the wind and
//! wanders on its own; a stronger vortex wanders faster.

use bevy::prelude::*;
use rand::Rng;

use crate::{
    bubbles::Band,
    features::follow,
    field::{FlowField, Vortex, noise1},
    bubble_controls::{Control, P, wind_dir},
    tuning::{Transforms, Tuning},
};

/// Static description of one vortex.
struct VortexSpec {
    name: &'static str,
    /// Core radius as a fraction of the screen height.
    core: f32,
    /// +1 counter-clockwise, −1 clockwise.
    spin: f32,
    /// Fraction of the wind velocity the vortex itself is carried by.
    drift: f32,
}

const VORTICES: [VortexSpec; 4] = [
    VortexSpec { name: "large", core: 0.22, spin: 1.0, drift: 0.35 },
    VortexSpec { name: "medium", core: 0.16, spin: -1.0, drift: 0.45 },
    VortexSpec { name: "small", core: 0.12, spin: 1.0, drift: 0.55 },
    VortexSpec { name: "tiny", core: 0.08, spin: -1.0, drift: 0.6 },
];

pub fn vortex_name(i: usize) -> &'static str {
    VORTICES[i].name
}

/// Moving state of each vortex.
#[derive(Resource)]
pub struct Swirls {
    pub pos: [Vec2; 4],
    /// Circulation, 0..1 (lightly smoothed control value).
    pub strength: [f32; 4],
    /// Position along each vortex's wander path.
    wander_phase: [f32; 4],
    initialised: bool,
}

impl Default for Swirls {
    fn default() -> Self {
        Self { pos: [Vec2::ZERO; 4], strength: [0.0; 4], wander_phase: [0.0, 10.0, 20.0, 30.0], initialised: false }
    }
}

pub struct ConductorPlugin;

impl Plugin for ConductorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlowField>()
            .init_resource::<Swirls>()
            .add_systems(FixedUpdate, conduct.in_set(crate::bubbles::SimSet::Conduct))
            // Vortices start afresh each time the bubbles come back.
            .add_systems(OnEnter(crate::scene::Scene::Bubbles), |mut swirls: ResMut<Swirls>| *swirls = Swirls::default());
    }
}

/// Reference speed for the vortices' wandering, screen widths per second.
const WANDER_SPEED: f32 = 0.10;

fn conduct(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    window: Single<&Window>,
    mut swirls: ResMut<Swirls>,
    mut field: ResMut<FlowField>,
) {
    let dt = time.delta_secs();
    let screen = window.size();
    let c = |control: Control| tuning.control(control, &transforms);

    // --- Wind.
    let speed = c(Control::WindSpeed) * screen.x;
    let angle = c(Control::WindHeading).to_radians();
    field.wind = Vec2::from_angle(angle) * speed;

    // --- Vortex circulation (lightly smoothed so slider moves don't jump).
    for (i, control) in Control::VORTICES.into_iter().enumerate() {
        swirls.strength[i] = follow(swirls.strength[i], c(control), dt, 0.03, 0.1);
    }

    // --- Vortex motion. Vortices live on the screen itself, not the whole band.
    let screen_band = Band::new(wind_dir(&tuning), screen, 0.0);
    let mut rng = rand::rng();
    if !swirls.initialised {
        swirls.initialised = true;
        for i in 0..4 {
            let s = screen_band.s_min + (screen_band.s_max - screen_band.s_min) * (0.2 + 0.2 * i as f32);
            let t = screen_band.t_min + (screen_band.t_max - screen_band.t_min) * [0.35, 0.65, 0.45, 0.55][i];
            swirls.pos[i] = screen_band.point(s, t);
        }
    }

    let drift_rate = c(Control::VortexDrift);
    let wander_unit = WANDER_SPEED * screen.x;
    let peak = tuning.get(P::VortexPeak) * screen.x;
    let small_y = c(Control::VortexSmallHeight) * screen.y;
    for (i, spec) in VORTICES.iter().enumerate() {
        let core = spec.core * screen.y;
        let st = swirls.strength[i];
        swirls.wander_phase[i] += dt * (0.05 + 0.35 * st) * drift_rate;
        let ph = swirls.wander_phase[i];
        let wander = Vec2::new(noise1(ph, i as f32 * 2.0), noise1(ph, i as f32 * 2.0 + 1.0)) * 2.0;
        let mut vel = field.wind * spec.drift + wander * wander_unit * (0.15 + 0.5 * st);
        // The small vortex is pulled towards its height control.
        if i == 2 {
            vel.y += (small_y - swirls.pos[i].y) * 0.8;
        }
        let mut p = swirls.pos[i] + vel * dt;

        // Recycle: once well past the screen downwind, re-enter upwind of it.
        let (mut s, mut t) = (p.dot(screen_band.along), p.dot(screen_band.across));
        if s > screen_band.s_max + 2.0 * core {
            s = screen_band.s_min - 1.5 * core;
            t = rng.random_range(screen_band.t_min..screen_band.t_max);
        }
        // Don't let a vortex wander far off the sides.
        t = t.clamp(screen_band.t_min - core, screen_band.t_max + core);
        p = screen_band.point(s, t);
        swirls.pos[i] = p;

        field.vortices[i] = Vortex { pos: p, core, peak_speed: spec.spin * st * peak };
    }
    field.swirl_limit = tuning.get(P::VortexCap) * speed;

    // --- Turbulence: drifts with the wind and evolves at its own rate.
    field.turbulence = c(Control::TurbulenceIntensity) * speed;
    field.turbulence_scale = 0.3 * screen.y;
    let drift = field.wind * 0.8 * dt;
    field.turbulence_offset += drift;
    field.turbulence_morph += dt * 0.15 * c(Control::TurbulenceEvolution);
}
