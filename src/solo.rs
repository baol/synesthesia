//! `--solo CONTROL`: isolate one motion control for review. Every other
//! control is held at a neutral value (steady wind, no vortices, no
//! turbulence, a still plate, no hops); only the chosen one keeps its
//! transforms. A caption shows the chain: the control, the transforms feeding
//! it, and the music signals feeding those.
//!
//! `--solo list` prints the names.

use bevy::prelude::*;

use crate::{
    field::FlowField,
    tuning::{Control, P, Signal, Signals, Transformation, Transforms, Tuning},
};

pub fn print_list() {
    for c in Control::ALL {
        println!("{:<22} {}", c.spec().key, c.spec().label);
    }
}

pub fn find(key: &str) -> Option<Control> {
    Control::find(key)
}

/// Synthetic-track start offset (s) for a solo: controls that follow the
/// music's energy or busyness start in a breakdown, so the clip shows the
/// music coming back.
pub fn synth_offset(c: Control) -> f32 {
    match c {
        Control::InflowRate | Control::WindSpeed | Control::VortexDrift | Control::TurbulenceEvolution => 26.0,
        _ => 0.0,
    }
}

/// Hold every control but `solo` at its neutral value.
pub fn isolate(t: &mut Tuning, solo: Control) {
    for c in Control::ALL {
        if c == solo {
            continue;
        }
        let i = c as usize;
        t.base[i] = c.spec().neutral;
        t.amounts[i] = [0.0; Transformation::COUNT];
    }
    // Some controls only show on something else that exists.
    let mut steady = |c: Control, v: f32| t.base[c as usize] = v;
    match solo {
        Control::VortexDrift => Control::VORTICES.into_iter().for_each(|v| steady(v, 0.5)),
        Control::VortexSmallHeight => steady(Control::VortexSmall, 0.7),
        Control::TurbulenceEvolution => steady(Control::TurbulenceIntensity, 0.25),
        Control::PlateMode => steady(Control::PlateVibration, 0.6),
        _ => {}
    }
    if !matches!(solo, Control::PlateVibration | Control::PlateMode) {
        t.params[P::PlatePull as usize] = 0.0;
    }
}

#[derive(Resource)]
pub struct ActiveSolo(pub Control);

#[derive(Component)]
struct Caption;

pub struct SoloPlugin;

impl Plugin for SoloPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (caption, rings).after(crate::tuning::SignalSet));
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
    mut text: Query<&mut Text, With<Caption>>,
) {
    let (Some(solo), Ok(mut text)) = (solo, text.single_mut()) else { return };
    let c = solo.0;
    let spec = c.spec();
    let value = tuning.control(c, &transforms);
    let mut s = format!("{} = {:.*}\n", spec.label, spec.decimals.max(0) as usize, value);
    for t in Transformation::ALL {
        let amount = tuning.amounts[c as usize][t as usize];
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
    match c {
        Control::PlateMode | Control::PlateVibration => {
            let mode = sand.modes[sand.current];
            s += &format!("  plate: {} rings, {} spokes\n", mode.n, 2 * mode.m);
        }
        Control::WindSpeed | Control::HopImpulse => {
            s += &format!("  tempo {:.0} BPM, pulse clarity {:.2}\n", 60.0 + 120.0 * features.tempo, features.pulse_clarity);
        }
        _ => {}
    }
    text.0 = s.trim_end().to_string();
}

/// Vortex solos: a faint ring at the vortex core, so you can see where it is.
fn rings(solo: Option<Res<ActiveSolo>>, field: Res<FlowField>, mut gizmos: Gizmos) {
    let Some(solo) = solo else { return };
    let vortices: &[usize] = match solo.0 {
        Control::VortexLarge => &[0],
        Control::VortexMedium => &[1],
        Control::VortexSmall | Control::VortexSmallHeight => &[2],
        Control::VortexTiny => &[3],
        Control::VortexDrift => &[0, 1, 2, 3],
        _ => &[],
    };
    for &i in vortices {
        let v = &field.vortices[i];
        gizmos.circle_2d(v.pos, v.core, LinearRgba::new(1.0, 0.75, 0.35, 0.35));
    }
}
