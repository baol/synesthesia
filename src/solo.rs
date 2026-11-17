//! `--solo CONTROL`: isolate one control of a scene for review. The run shows
//! that control's scene; every other control there is held at a neutral value
//! (for the bubbles: steady wind, no vortices, no turbulence, a still plate,
//! no hops), and only the chosen one keeps its transforms. A caption shows the
//! chain: the control, the transforms feeding it, and the music signals
//! feeding those.
//!
//! Name it `scene.control` (`glass.breath`) or just `control` when that is
//! unambiguous. `--solo list` prints the names.

use bevy::prelude::*;

use crate::{
    bubble_controls::{Control, P},
    field::FlowField,
    scene::{ControlId, Scene, SceneControl, SceneParam},
    tuning::{Knob, Signal, Signals, Transformation, Transforms, Tuning},
};

pub fn print_list() {
    for scene in Scene::ALL {
        for c in scene.controls() {
            let name = format!("{}.{}", scene.key(), c.spec().key);
            println!("{name:<30} {}", c.spec().label);
        }
    }
}

pub fn find(name: &str) -> Option<ControlId> {
    ControlId::find(name)
}

/// Synthetic-track start offset (s) for a solo: controls that follow the
/// music's energy or busyness start in a breakdown, so the clip shows the
/// music coming back.
pub fn synth_offset(c: ControlId) -> f32 {
    let slow = [Control::InflowRate, Control::WindSpeed, Control::VortexDrift, Control::TurbulenceEvolution].map(|c| c.id());
    use crate::glass::Control as G;
    // (The dance solos start on the beat: the dance is what they show.)
    let glass_slow = [G::Backlight].map(|c| c.id());
    if slow.contains(&c) || glass_slow.contains(&c) {
        26.0
    } else if c == G::Havoc.id() {
        // A little before the rap bursts in (every 32 s, after the breakdown;
        // clips start capturing ~2 s after launch).
        27.0
    } else if [G::Spotlight, G::SpotHeight, G::SpotSway].map(|c| c.id()).contains(&c) {
        // A little before the singer comes in (8 s into each 32).
        3.0
    } else {
        0.0
    }
}

/// Show `solo`'s scene and hold every other control there at its neutral value.
pub fn isolate(t: &mut Tuning, solo: ControlId) {
    t.scene = solo.scene;
    for c in solo.scene.controls() {
        if c == solo {
            continue;
        }
        let st = t.scene_mut(c.scene);
        st.base[c.index] = c.spec().neutral;
        st.amounts[c.index] = [0.0; Transformation::COUNT];
    }
    if solo.scene == Scene::Glass {
        // Turning only shows on panes that dance; the spotlight's moves on a lit spotlight.
        if solo == crate::glass::Control::DanceTurn.id() {
            t.set_knob(Knob::Base(crate::glass::Control::DanceStep.id()), 0.15);
        }
        if solo == crate::glass::Control::SpotHeight.id() || solo == crate::glass::Control::SpotSway.id() {
            t.set_knob(Knob::Base(crate::glass::Control::Spotlight.id()), 0.9);
        }
        return;
    }
    if solo.scene != Scene::Bubbles {
        return;
    }
    // Some bubble controls only show on something else that exists.
    let mut steady = |c: Control, v: f32| t.set_knob(Knob::Base(c.id()), v);
    let is = |c: Control| solo == c.id();
    if is(Control::VortexDrift) {
        Control::VORTICES.into_iter().for_each(|v| steady(v, 0.5));
    } else if is(Control::VortexSmallHeight) {
        steady(Control::VortexSmall, 0.7);
    } else if is(Control::TurbulenceEvolution) {
        steady(Control::TurbulenceIntensity, 0.25);
    } else if is(Control::PlateMode) {
        steady(Control::PlateVibration, 0.6);
    }
    if !is(Control::PlateVibration) && !is(Control::PlateMode) {
        t.set_knob(Knob::Param(P::PlatePull.id()), 0.0);
    }
}

#[derive(Resource)]
pub struct ActiveSolo(pub ControlId);

#[derive(Component)]
struct Caption;

pub struct SoloPlugin;

impl Plugin for SoloPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(
                Update,
                (caption, rings.run_if(in_state(Scene::Bubbles)), ripple_rings.run_if(in_state(Scene::Glass)))
                    .after(crate::tuning::SignalSet),
            );
    }
}

fn setup(mut commands: Commands, solo: Option<Res<ActiveSolo>>) {
    if solo.is_none() {
        return;
    }
    commands.spawn((
        Caption,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgb(0.85, 0.87, 0.92)),
        Node {
            position_type: PositionType::Absolute,
            top: px(14),
            left: px(16),
            padding: UiRect::axes(px(10), px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.92)),
    ));
}

fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 16.0).round() as usize;
    format!("{}{}", "#".repeat(n), "-".repeat(16 - n))
}

fn caption(
    solo: Option<Res<ActiveSolo>>,
    tuning: Res<Tuning>,
    signals: Res<Signals>,
    transforms: Res<Transforms>,
    features: Res<crate::features::AudioFeatures>,
    sand: Res<crate::sand::Sand>,
    clock: Res<crate::tuning::MusicClock>,
    mut text: Query<&mut Text, With<Caption>>,
) {
    let (Some(solo), Ok(mut text)) = (solo, text.single_mut()) else { return };
    let c = solo.0;
    let spec = c.spec();
    let value = tuning.control_value(c, &transforms);
    let mut s = format!("{} = {:.*}\n", spec.label, spec.decimals.max(0) as usize, value);
    for t in Transformation::ALL {
        let amount = tuning.knob(Knob::Amount(c, t));
        if amount == 0.0 {
            continue;
        }
        let out = transforms.get(t);
        s += &format!("  <- {:<12} {} {out:.2}  x{amount}\n", t.spec().label, bar(out));
        for sig in Signal::ALL {
            let w = tuning.weights[t as usize][sig as usize];
            if w != 0.0 {
                let v = signals.get(sig);
                s += &format!("       <- {:<14} {} {v:.2}  x{w}\n", sig.label(), bar(v));
            }
        }
    }
    if c == Control::PlateMode.id() || c == Control::PlateVibration.id() {
        let mode = sand.modes[sand.current];
        s += &format!("  plate: {} rings, {} spokes\n", mode.n, 2 * mode.m);
    } else if c == crate::glass::Control::DanceStep.id() || c == crate::glass::Control::DanceTurn.id() {
        let now = clock.count(8).floor() as usize;
        let counts: Vec<String> = (0..8).map(|k| if k == now { format!("[{}]", k + 1) } else { format!(" {} ", k + 1) }).collect();
        s += &format!("  count {}   (salsa on 1: forward 1, back 5)\n", counts.join(""));
    } else if c == Control::WindSpeed.id() || c == Control::HopImpulse.id() {
        s += &format!("  tempo {:.0} BPM, pulse clarity {:.2}\n", 60.0 + 120.0 * features.tempo, features.pulse_clarity);
    }
    text.0 = s.trim_end().to_string();
}

/// Vortex solos: a faint ring at the vortex core, so you can see where it is.
fn rings(solo: Option<Res<ActiveSolo>>, field: Res<FlowField>, mut gizmos: Gizmos) {
    let Some(solo) = solo else { return };
    let is = |c: Control| solo.0 == c.id();
    let vortices: &[usize] = if is(Control::VortexLarge) {
        &[0]
    } else if is(Control::VortexMedium) {
        &[1]
    } else if is(Control::VortexSmall) || is(Control::VortexSmallHeight) {
        &[2]
    } else if is(Control::VortexTiny) {
        &[3]
    } else if is(Control::VortexDrift) {
        &[0, 1, 2, 3]
    } else {
        &[]
    };
    for &i in vortices {
        let v = &field.vortices[i];
        gizmos.circle_2d(v.pos, v.core, LinearRgba::new(1.0, 0.75, 0.35, 0.35));
    }
}

/// Ripple solo: a faint ring on each ripple front.
fn ripple_rings(
    solo: Option<Res<ActiveSolo>>,
    motion: Res<crate::glass_motion::Motion>,
    screen: Single<&Window>,
    mut gizmos: Gizmos,
) {
    let Some(solo) = solo else { return };
    if solo.0 != crate::glass::Control::Ripple.id() {
        return;
    }
    for r in &motion.ripples {
        gizmos.circle_2d(r.origin, r.radius(screen.size().x), LinearRgba::new(1.0, 0.75, 0.35, 0.35));
    }
}
