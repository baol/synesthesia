//! Debug overlay (toggle with `D`): swirl centres and cores, a sparse grid of
//! flow arrows, and text meters for the musical features.

use bevy::prelude::*;

use crate::{
    conductor::{Swirls, vortex_name},
    config::Config,
    features::{AudioFeatures, FeatureSource},
    field::FlowField,
};

#[derive(Resource)]
pub struct DebugView(pub bool);

#[derive(Component)]
struct Meters;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (toggle, draw_field, update_meters).chain());
    }
}

fn setup(mut commands: Commands, cfg: Res<Config>) {
    commands.insert_resource(DebugView(cfg.debug));
    commands.spawn((
        Meters,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(0.75, 0.8, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            top: px(10),
            left: px(12),
            ..default()
        },
    ));
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut view: ResMut<DebugView>) {
    if keys.just_pressed(KeyCode::KeyD) {
        view.0 = !view.0;
    }
}

fn draw_field(
    view: Res<DebugView>,
    field: Res<FlowField>,
    swirls: Res<Swirls>,
    window: Single<&Window>,
    mut gizmos: Gizmos,
) {
    if !view.0 {
        return;
    }
    let size = window.size();
    let half = size / 2.0;

    // Flow arrows on a coarse grid; length is the local velocity over ~0.25 s.
    let (cols, rows) = (24, 14);
    let wind_len = field.wind.length().max(1.0);
    for j in 0..rows {
        for i in 0..cols {
            let p = Vec2::new(
                -half.x + (i as f32 + 0.5) * size.x / cols as f32,
                -half.y + (j as f32 + 0.5) * size.y / rows as f32,
            );
            let u = field.velocity(p);
            // Brighter where the flow departs from the plain wind.
            let dev = ((u - field.wind).length() / wind_len).min(1.0);
            let color = LinearRgba::new(0.2 + 0.8 * dev, 0.45, 1.0 - 0.6 * dev, 0.9);
            gizmos.arrow_2d(p, p + u * 0.25, color);
        }
    }

    // Swirls: core circle and peak-speed ring; brightness follows strength.
    for (i, v) in field.vortices.iter().enumerate() {
        let st = swirls.strength[i];
        let hue = [
            LinearRgba::new(1.0, 0.35, 0.25, 1.0),
            LinearRgba::new(1.0, 0.8, 0.2, 1.0),
            LinearRgba::new(0.3, 1.0, 0.5, 1.0),
            LinearRgba::new(0.4, 0.6, 1.0, 1.0),
        ][i];
        let c = hue * (0.3 + 1.7 * st);
        gizmos.circle_2d(v.pos, v.core, c);
        gizmos.circle_2d(v.pos, v.core * 1.121, c * 0.5);
        // Spin direction marker.
        let tip = v.pos + Vec2::new(v.core * 1.121, 0.0);
        gizmos.arrow_2d(tip, tip + Vec2::Y * v.peak_speed.signum() * 0.3 * v.core, c);
    }
}

fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 20.0).round() as usize;
    format!("{}{}", "#".repeat(n), "-".repeat(20 - n))
}

fn update_meters(
    view: Res<DebugView>,
    features: Res<AudioFeatures>,
    source: Res<FeatureSource>,
    swirls: Res<Swirls>,
    field: Res<FlowField>,
    signals: Res<crate::tuning::Signals>,
    transforms: Res<crate::tuning::Transforms>,
    sand: Res<crate::sand::Sand>,
    jump: Res<crate::bubbles::Jump>,
    live: Option<Res<crate::audio::LiveAudio>>,
    mut text: Single<&mut Text, With<Meters>>,
) {
    use crate::tuning::{Signal, Transformation};
    if !view.0 {
        if !text.0.is_empty() {
            text.0.clear();
        }
        return;
    }
    let f = &*features;
    let label = match (*source, &live) {
        (FeatureSource::Live, Some(l)) => l.label.clone(),
        (s, _) => format!("{s:?}"),
    };
    let mut s = format!("source   {label}   (M: cycle source, 1-4/Space/B: poke, H: spectrum, Cmd-,: prefs, D: hide)\n");
    s += &if f.level_db.is_finite() { format!("input    {:.1} dBFS\n", f.level_db) } else { "\n".into() };
    s += &format!("tempo    {:.0} BPM\n\nSIGNALS (music)\n", 60.0 + 120.0 * f.tempo);
    for sig in Signal::ALL {
        let v = signals.get(sig);
        s += &format!("{:<14} {} {v:.2}\n", sig.label(), bar(v));
    }
    s += "\nTRANSFORMATIONS\n";
    for t in Transformation::ALL {
        let v = transforms.get(t);
        s += &format!("{:<14} {} {v:.2}\n", t.spec().label, bar(v));
    }
    s += "\nPHYSICS\n";
    for i in 0..4 {
        s += &format!("vortex {:<7} {} {:.2}\n", vortex_name(i), bar(swirls.strength[i]), swirls.strength[i]);
    }
    let mode = sand.modes[sand.current];
    s += &format!("plate          {} {:.2} ({} rings, {} spokes)\n", bar(sand.amp), sand.amp, mode.n, 2 * mode.m);
    s += &format!("hop            {:.1} px\n", jump.y);
    s += &format!(
        "wind           {:.0} px/s at {:.0} deg, turbulence {:.0} px/s",
        field.wind.length(),
        field.wind.to_angle().to_degrees(),
        field.turbulence
    );
    text.0 = s;
}
