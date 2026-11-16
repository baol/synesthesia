//! The input spectrum as a small histogram at the bottom of the screen
//! (toggle with `H`, or from the preferences panel). Bars show the same 32
//! bands creation reads, with slowly falling peak marks; a coloured strip
//! underneath shows which of the four swirl bands each bar belongs to. Next to
//! it, a second histogram shows the driver signals the controls are routed from.

use bevy::prelude::*;

use crate::{
    config::Config,
    features::{AudioFeatures, SPECTRUM_BINS, SPECTRUM_HI_HZ, SPECTRUM_LO_HZ, spectrum_center},
    tuning::{Signal, Signals, Transformation, Transforms, Tuning},
};

const BAR_W: f32 = 10.0;
const GAP: f32 = 3.0;
const HEIGHT: f32 = 56.0;

/// Colours of the four bands, as in the debug overlay.
pub const BAND_COLORS: [Color; 4] = [
    Color::srgba(1.0, 0.45, 0.35, 0.7),
    Color::srgba(1.0, 0.8, 0.3, 0.7),
    Color::srgba(0.4, 1.0, 0.55, 0.7),
    Color::srgba(0.5, 0.65, 1.0, 0.7),
];

#[derive(Component)]
struct SpectrumRoot;

#[derive(Component)]
struct Bar(usize);

#[derive(Component)]
struct Peak(usize);

/// One bar of the driver histogram: a transform, or the pulse-clarity signal.
#[derive(Component, Clone, Copy)]
enum Driver {
    Transform(Transformation),
    Pulse,
    Beat,
}

const DRIVER_BAR_W: f32 = 34.0;
const DRIVER_LABELS: [&str; Transformation::COUNT] =
    ["energy", "groove", "low", "l-mid", "h-mid", "attack", "busy", "bright", "sparkle", "pitch", "tempo", "dance"];

#[derive(Resource, Default)]
struct Peaks([f32; SPECTRUM_BINS]);

pub struct SpectrumViewPlugin;

impl Plugin for SpectrumViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Peaks>()
            .add_systems(Startup, setup)
            .add_systems(Update, (toggle, update).chain().after(crate::features::FeatureSet));
    }
}

fn band_of(freq: f32) -> usize {
    crate::analysis::BANDS.iter().position(|&(_, hi)| freq < hi).unwrap_or(3)
}

fn setup(mut commands: Commands, tuning: Res<Tuning>) {
    let width = SPECTRUM_BINS as f32 * (BAR_W + GAP) - GAP;
    let label = |text: &str, freq: f32| {
        let u = (freq / SPECTRUM_LO_HZ).ln() / (SPECTRUM_HI_HZ / SPECTRUM_LO_HZ).ln();
        (
            Text::new(text),
            TextFont { font_size: FontSize::Px(10.0), ..default() },
            TextColor(Color::srgba(0.75, 0.77, 0.83, 0.85)),
            Node { position_type: PositionType::Absolute, left: px(u * width - 6.0), top: px(2), ..default() },
        )
    };
    commands
        .spawn((
            SpectrumRoot,
            if tuning.show_spectrum { Visibility::Inherited } else { Visibility::Hidden },
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(10),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexEnd,
                column_gap: px(8),
                ..default()
            },
        ))
        .with_children(|root| {
            spawn_drivers(root);
            root.spawn((
                Node {
                    width: px(width + 20.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::new(px(10), px(10), px(8), px(2)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
            ))
                .with_children(|col| {
                    // Bars.
                    col.spawn(Node {
                        height: px(HEIGHT),
                        column_gap: px(GAP),
                        align_items: AlignItems::FlexEnd,
                        ..default()
                    })
                    .with_children(|bars| {
                        for i in 0..SPECTRUM_BINS {
                            bars.spawn(Node { width: px(BAR_W), height: percent(100), ..default() })
                                .with_children(|slot| {
                                    slot.spawn((
                                        Bar(i),
                                        Node {
                                            position_type: PositionType::Absolute,
                                            bottom: px(0),
                                            width: percent(100),
                                            height: percent(0),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.78, 0.8, 0.86, 0.5)),
                                    ));
                                    slot.spawn((
                                        Peak(i),
                                        Node {
                                            position_type: PositionType::Absolute,
                                            bottom: percent(0),
                                            width: percent(100),
                                            height: px(2),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.92, 0.93, 0.97, 0.55)),
                                    ));
                                });
                        }
                    });
                    // Band strip.
                    col.spawn(Node { height: px(2), margin: UiRect::top(px(2)), column_gap: px(GAP), ..default() })
                        .with_children(|strip| {
                            for i in 0..SPECTRUM_BINS {
                                strip.spawn((
                                    Node { width: px(BAR_W), height: percent(100), ..default() },
                                    BackgroundColor(BAND_COLORS[band_of(spectrum_center(i))]),
                                ));
                            }
                        });
                    // Frequency labels.
                    col.spawn(Node { height: px(16), ..default() }).with_children(|labels| {
                        for (text, f) in [("50", 50.0), ("100", 100.0), ("300", 300.0), ("1k", 1000.0), ("3k", 3000.0), ("10k", 10_000.0)] {
                            labels.spawn(label(text, f));
                        }
                    });
                });
        });
}

/// The driver histogram: the shaped signals (groove, brightness, …) plus pulse clarity.
fn spawn_drivers(root: &mut ChildSpawnerCommands) {
    let drivers: Vec<(Driver, &str)> = Transformation::ALL
        .iter()
        .map(|&t| (Driver::Transform(t), DRIVER_LABELS[t as usize]))
        .chain([(Driver::Pulse, "pulse"), (Driver::Beat, "beat")])
        .collect();
    root.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::new(px(10), px(10), px(8), px(2)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
    ))
    .with_children(|col| {
        col.spawn(Node { height: px(HEIGHT), column_gap: px(GAP), align_items: AlignItems::FlexEnd, ..default() })
            .with_children(|bars| {
                for &(d, _) in &drivers {
                    bars.spawn(Node { width: px(DRIVER_BAR_W), height: percent(100), ..default() }).with_children(|slot| {
                        slot.spawn((
                            d,
                            Node {
                                position_type: PositionType::Absolute,
                                bottom: px(0),
                                width: percent(100),
                                height: percent(0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.55, 0.85, 0.95, 0.6)),
                        ));
                    });
                }
            });
        col.spawn(Node { height: px(16), column_gap: px(GAP), margin: UiRect::top(px(2)), ..default() })
            .with_children(|labels| {
                for &(_, name) in &drivers {
                    labels
                        .spawn(Node { width: px(DRIVER_BAR_W), overflow: Overflow::visible(), justify_content: JustifyContent::Center, ..default() })
                        .with_children(|l| {
                            l.spawn((
                                Text::new(name),
                                TextFont { font_size: FontSize::Px(8.0), ..default() },
                                TextColor(Color::srgba(0.75, 0.77, 0.83, 0.85)),
                            ));
                        });
                }
            });
    });
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<Config>,
    mut tuning: ResMut<Tuning>,
    mut root: Single<&mut Visibility, With<SpectrumRoot>>,
) {
    if keys.just_pressed(KeyCode::KeyH) {
        tuning.show_spectrum = !tuning.show_spectrum;
        crate::prefs::save(&tuning, &cfg);
    }
    if tuning.is_changed() {
        **root = if tuning.show_spectrum { Visibility::Inherited } else { Visibility::Hidden };
    }
}

fn update(
    time: Res<Time>,
    tuning: Res<Tuning>,
    features: Res<AudioFeatures>,
    mut peaks: ResMut<Peaks>,
    mut bars: Query<(&Bar, &mut Node), (Without<Peak>, Without<Driver>)>,
    mut marks: Query<(&Peak, &mut Node), (Without<Bar>, Without<Driver>)>,
    signals: Res<Signals>,
    transforms: Res<Transforms>,
    mut drivers: Query<(&Driver, &mut Node), (Without<Bar>, Without<Peak>)>,
) {
    if !tuning.show_spectrum {
        return;
    }
    let dt = time.delta_secs();
    for (p, &v) in peaks.0.iter_mut().zip(&features.spectrum) {
        // Peaks hold, then fall at half the height per second.
        *p = (*p - 0.5 * dt).max(v);
    }
    for (bar, mut node) in &mut bars {
        node.height = percent(features.spectrum[bar.0] * 100.0);
    }
    for (mark, mut node) in &mut marks {
        node.bottom = percent(peaks.0[mark.0] * 98.0);
    }
    for (d, mut node) in &mut drivers {
        let v = match *d {
            Driver::Transform(t) => transforms.get(t),
            Driver::Pulse => signals.get(Signal::PulseClarity),
            Driver::Beat => signals.get(Signal::Beat),
        };
        node.height = percent(v.clamp(0.0, 1.0) * 100.0);
    }
}
