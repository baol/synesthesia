//! Preferences panel (Cmd-, or Ctrl-, to toggle, Esc to close), docked on the
//! right, generated from the tables in `tuning.rs`:
//!
//! - **Transforms**: for each one, its live output; unfold it to set how much
//!   of each music signal goes in (with a meter of that signal), the response
//!   curve, and the attack and release times.
//! - **The scene's world** (for the bubbles: inflow, wind, vortices,
//!   turbulence, plate, bubbles; for the glass window: panes, light, glass):
//!   the constants, then each control with its live value; unfold it to set
//!   the base value and how much of each transform drives it (with a meter of
//!   that transform). Only the scene being shown is listed; the buttons at the
//!   top switch scene.
//!
//! Every change is saved to the preferences file when you let go of a slider.

use std::collections::HashSet;

use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    ui_widgets::{Activate, Button, Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick, ValueChange},
};

use crate::{
    config::Config,
    scene::{ControlId, Scene},
    tuning::{Knob, Signal, Signals, Transformation, Transforms, Tuning},
};

/// Save the tuning, unless the run started with `--no-prefs`.
pub fn save(tuning: &Tuning, cfg: &Config) {
    if !cfg.no_prefs {
        tuning.save();
    }
}

const PANEL_W: f32 = 400.0;
const LABEL_W: f32 = 150.0;
const VALUE_W: f32 = 54.0;
const THUMB_W: f32 = 6.0;

const BG: Color = Color::srgba(0.03, 0.035, 0.045, 0.95);
const TRACK: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
const FILL: Color = Color::srgba(0.62, 0.72, 0.9, 0.45);
const THUMB: Color = Color::srgb(0.88, 0.9, 0.95);
const TEXT: Color = Color::srgb(0.8, 0.82, 0.87);
const DIM: Color = Color::srgba(0.8, 0.82, 0.87, 0.4);
const HEADING: Color = Color::srgb(0.55, 0.62, 0.75);
const METER: Color = Color::srgba(0.95, 0.8, 0.45, 0.45);

/// A foldable block: a transform or a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Block {
    Transformation(Transformation),
    Control(ControlId),
}

/// What a row's meter shows.
#[derive(Clone, Copy)]
enum Source {
    Signal(Signal),
    Transformation(Transformation),
}

#[derive(Component)]
struct PanelRoot;

#[derive(Component)]
struct KnobSlider(Knob);

#[derive(Component)]
struct ThumbOf(Knob);

#[derive(Component)]
struct FillOf(Knob);

#[derive(Component)]
struct ValueOf(Knob);

#[derive(Component)]
struct LabelOf(Knob);

#[derive(Component)]
struct BlockBody(Block);

#[derive(Component)]
struct BlockHeader(Block);

#[derive(Component)]
struct BlockValue(Block);

#[derive(Component)]
struct Meter(Block, Source);

#[derive(Component)]
struct SpectrumButtonText;

/// The part of the panel that belongs to the current scene.
#[derive(Component)]
struct SceneSection;

/// A scene button and the scene it shows.
#[derive(Component)]
struct SceneButton(Scene);

/// Which blocks are unfolded.
#[derive(Resource)]
struct Expanded(HashSet<Block>);

pub struct PrefsPlugin;

impl Plugin for PrefsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Expanded(HashSet::new()))
            .add_systems(Startup, setup)
            .add_systems(Update, build_scene_section.run_if(state_changed::<Scene>))
            .add_observer(on_value_change)
            .add_systems(
                Update,
                (toggle, scroll, sync_from_tuning, fold, live_values).chain().after(crate::tuning::SignalSet),
            );
    }
}

fn font(size: f32) -> TextFont {
    TextFont { font_size: FontSize::Px(size), ..default() }
}

fn knob_label(k: Knob) -> String {
    match k {
        Knob::Param(p) => p.spec().label.to_string(),
        Knob::Weight(_, s) => s.label().into(),
        Knob::Curve(_) => "response curve".into(),
        Knob::Attack(_) => "attack (s)".into(),
        Knob::Release(_) => "release (s)".into(),
        Knob::Base(_) => "base".into(),
        Knob::Amount(_, t) => t.spec().label.into(),
    }
}

fn format_value(k: Knob, v: f32) -> String {
    format!("{:.*}", k.decimals().max(0) as usize, v)
}

fn heading(p: &mut ChildSpawnerCommands, text: &str) {
    p.spawn((Text::new(text), font(11.0), TextColor(HEADING), Node { margin: UiRect::top(px(10)), ..default() }));
}

fn setup(mut commands: Commands, tuning: Res<Tuning>, cfg: Res<Config>, expanded: Res<Expanded>) {
    let root = commands
        .spawn((
            PanelRoot,
            Node {
                display: if cfg.prefs_open { Display::Flex } else { Display::None },
                position_type: PositionType::Absolute,
                top: px(0),
                right: px(0),
                bottom: px(0),
                width: px(PANEL_W),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(px(14), px(12)),
                row_gap: px(3),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition::default(),
            BackgroundColor(BG),
            GlobalZIndex(10),
        ))
        .id();

    commands.entity(root).with_children(|p| {
        p.spawn((Text::new("PREFERENCES"), font(15.0), TextColor(TEXT)));
        let saved = if cfg.no_prefs {
            "not saved (--no-prefs)".to_string()
        } else {
            match crate::tuning::prefs_path() {
                Some(path) => format!("saved to {}", path.display()),
                None => "not saved".into(),
            }
        };
        p.spawn((
            Text::new(format!(
                "Music signals feed transformations; transformations drive the scene's world.\nCmd-, or Esc to close. Click a heading to unfold it.\n{saved}"
            )),
            font(10.0),
            TextColor(DIM),
            Node { margin: UiRect::bottom(px(6)), ..default() },
        ));
        p.spawn(Node { column_gap: px(8), margin: UiRect::bottom(px(4)), align_items: AlignItems::Center, ..default() })
            .with_children(|row| {
                row.spawn((Text::new("scene (Tab):"), font(11.0), TextColor(DIM)));
                for scene in Scene::ALL {
                    button(row, scene.label()).insert(SceneButton(scene)).observe(
                        move |_: On<Activate>,
                              mut next: ResMut<NextState<Scene>>,
                              mut tuning: ResMut<Tuning>,
                              cfg: Res<Config>| {
                            crate::scene::switch_to(scene, &mut next, &mut tuning, &cfg);
                        },
                    );
                }
            });
        p.spawn(Node { column_gap: px(8), margin: UiRect::bottom(px(4)), ..default() }).with_children(|row| {
            button(row, "reset all").observe(reset_all);
            button(row, "reset this scene").observe(reset_scene);
            button(row, spectrum_label(tuning.show_spectrum)).insert(SpectrumButtonText).observe(toggle_spectrum);
        });

        heading(p, "TRANSFORMATIONS (music signals, shaped)");
        for t in Transformation::ALL {
            let block = Block::Transformation(t);
            block_header(p, block, t.spec().label, expanded.0.contains(&block));
            block_body(p, block, expanded.0.contains(&block), |b| {
                slider_row(b, Knob::Curve(t), &tuning, None);
                slider_row(b, Knob::Attack(t), &tuning, None);
                slider_row(b, Knob::Release(t), &tuning, None);
                for s in Signal::ALL {
                    slider_row(b, Knob::Weight(t, s), &tuning, Some(Meter(block, Source::Signal(s))));
                }
            });
        }
    });

    commands.entity(root).with_children(|p| {
        p.spawn((SceneSection, Node { flex_direction: FlexDirection::Column, row_gap: px(3), ..default() }));
    });
}

/// (Re)build the current scene's groups: its constants and controls.
fn build_scene_section(
    mut commands: Commands,
    scene: Res<State<Scene>>,
    tuning: Res<Tuning>,
    expanded: Res<Expanded>,
    section: Single<Entity, With<SceneSection>>,
    mut buttons: Query<(&SceneButton, &mut BackgroundColor)>,
) {
    let scene = *scene.get();
    for (SceneButton(s), mut bg) in &mut buttons {
        bg.0 = if *s == scene { Color::srgba(0.62, 0.72, 0.9, 0.35) } else { Color::srgba(1.0, 1.0, 1.0, 0.1) };
    }
    commands.entity(*section).despawn_related::<Children>();
    let table = scene.table();
    commands.entity(*section).with_children(|p| {
        for &group in table.groups {
            heading(p, &format!("{group} ({})", scene.label()));
            for param in scene.params().filter(|p| p.spec().group == group) {
                slider_row(p, Knob::Param(param), &tuning, None);
            }
            for c in scene.controls().filter(|c| c.spec().group == group) {
                let block = Block::Control(c);
                block_header(p, block, c.spec().label, expanded.0.contains(&block));
                block_body(p, block, expanded.0.contains(&block), |b| {
                    slider_row(b, Knob::Base(c), &tuning, None);
                    for t in Transformation::ALL {
                        slider_row(b, Knob::Amount(c, t), &tuning, Some(Meter(block, Source::Transformation(t))));
                    }
                });
            }
        }
    });
}

fn spectrum_label(on: bool) -> String {
    format!("spectrum (H): {}", if on { "on" } else { "off" })
}

fn button<'a>(parent: &'a mut ChildSpawnerCommands, text: impl Into<String>) -> EntityCommands<'a> {
    parent.spawn((
        Button,
        Node { padding: UiRect::axes(px(8), px(3)), ..default() },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)),
        Text::new(text),
        font(11.0),
        TextColor(TEXT),
    ))
}

/// A clickable header with the block's live value.
fn block_header(p: &mut ChildSpawnerCommands, block: Block, label: &str, open: bool) {
    p.spawn((Button, BlockHeader(block), Node { margin: UiRect::top(px(4)), column_gap: px(6), ..default() }))
        .observe(move |_: On<Activate>, mut expanded: ResMut<Expanded>| {
            if !expanded.0.remove(&block) {
                expanded.0.insert(block);
            }
        })
        .with_children(|h| {
            h.spawn((Text::new(if open { "-" } else { "+" }), font(12.0), TextColor(HEADING), Node { width: px(10), ..default() }));
            h.spawn((Text::new(label), font(12.0), TextColor(TEXT), Node { flex_grow: 1.0, ..default() }));
            h.spawn((BlockValue(block), Text::new(""), font(12.0), TextColor(METER)));
        });
}

fn block_body(p: &mut ChildSpawnerCommands, block: Block, open: bool, rows: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn((
        BlockBody(block),
        Node {
            display: if open { Display::Flex } else { Display::None },
            flex_direction: FlexDirection::Column,
            row_gap: px(3),
            padding: UiRect::left(px(10)),
            ..default()
        },
    ))
    .with_children(rows);
}

/// Label, slider, value. Routing rows also get a meter of their input under the label.
fn slider_row(p: &mut ChildSpawnerCommands, k: Knob, tuning: &Tuning, meter: Option<Meter>) {
    let (lo, hi) = k.range();
    let v = tuning.knob(k);
    p.spawn(Node { height: px(17), align_items: AlignItems::Center, column_gap: px(6), ..default() })
        .with_children(|row| {
            row.spawn(Node { width: px(LABEL_W), flex_direction: FlexDirection::Column, ..default() })
                .with_children(|l| {
                    l.spawn((LabelOf(k), Text::new(knob_label(k)), font(11.0), TextColor(TEXT)));
                    if let Some(m) = meter {
                        l.spawn((m, Node { height: px(2), width: percent(0), ..default() }, BackgroundColor(METER)));
                    }
                });
            row.spawn((
                KnobSlider(k),
                Slider { track_click: TrackClick::Snap, ..default() },
                SliderValue(v),
                SliderRange::new(lo, hi),
                SliderPrecision(k.decimals()),
                SliderStep((hi - lo) / 100.0),
                Node { flex_grow: 1.0, height: px(12), ..default() },
                BackgroundColor(TRACK),
            ))
            .with_children(|s| {
                s.spawn((FillOf(k), Node { position_type: PositionType::Absolute, top: px(0), bottom: px(0), ..default() }, BackgroundColor(FILL)));
                if lo < 0.0 && hi > 0.0 {
                    let zero = -lo / (hi - lo);
                    s.spawn((
                        Node { position_type: PositionType::Absolute, top: px(0), bottom: px(0), left: percent(zero * 100.0), width: px(1), ..default() },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.25)),
                    ));
                }
                // The thumb travels the track minus its own width, as the slider expects.
                s.spawn(Node { position_type: PositionType::Absolute, left: px(0), right: px(THUMB_W), top: px(0), bottom: px(0), ..default() })
                    .with_children(|area| {
                        area.spawn((
                            ThumbOf(k),
                            SliderThumb,
                            Node { position_type: PositionType::Absolute, width: px(THUMB_W), top: px(0), bottom: px(0), ..default() },
                            BackgroundColor(THUMB),
                        ));
                    });
            });
            row.spawn((
                ValueOf(k),
                Text::new(format_value(k, v)),
                font(11.0),
                TextColor(TEXT),
                TextLayout::justify(Justify::Right),
                Node { width: px(VALUE_W), ..default() },
            ));
        });
}

fn on_value_change(
    change: On<ValueChange<f32>>,
    sliders: Query<&KnobSlider>,
    mut tuning: ResMut<Tuning>,
    cfg: Res<Config>,
    mut commands: Commands,
) {
    let Ok(KnobSlider(k)) = sliders.get(change.source) else { return };
    tuning.set_knob(*k, change.value);
    commands.entity(change.source).insert(SliderValue(tuning.knob(*k)));
    if change.is_final {
        save(&tuning, &cfg);
    }
}

fn reset_all(
    _: On<Activate>,
    mut tuning: ResMut<Tuning>,
    cfg: Res<Config>,
    sliders: Query<(Entity, &KnobSlider)>,
    mut commands: Commands,
) {
    *tuning = Tuning { show_spectrum: tuning.show_spectrum, scene: tuning.scene, ..default() };
    for (e, KnobSlider(k)) in &sliders {
        commands.entity(e).insert(SliderValue(tuning.knob(*k)));
    }
    save(&tuning, &cfg);
}

/// Back to the defaults for the scene being shown only (transformations and
/// other scenes are kept).
fn reset_scene(
    _: On<Activate>,
    scene: Res<State<Scene>>,
    mut tuning: ResMut<Tuning>,
    cfg: Res<Config>,
    sliders: Query<(Entity, &KnobSlider)>,
    mut commands: Commands,
) {
    let scene = *scene.get();
    let fresh = Tuning::default();
    *tuning.scene_mut(scene) = fresh.scene(scene).clone();
    for (e, KnobSlider(k)) in &sliders {
        commands.entity(e).insert(SliderValue(tuning.knob(*k)));
    }
    save(&tuning, &cfg);
}

fn toggle_spectrum(_: On<Activate>, mut tuning: ResMut<Tuning>, cfg: Res<Config>) {
    tuning.show_spectrum = !tuning.show_spectrum;
    save(&tuning, &cfg);
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut panel: Single<&mut Node, With<PanelRoot>>) {
    let modifier = keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ControlLeft, KeyCode::ControlRight]);
    let open = panel.display != Display::None;
    if (modifier && keys.just_pressed(KeyCode::Comma)) || (open && keys.just_pressed(KeyCode::Escape)) {
        panel.display = if open { Display::None } else { Display::Flex };
    }
}

fn scroll(mut wheel: MessageReader<MouseWheel>, mut panel: Single<(&Node, &ComputedNode, &mut ScrollPosition), With<PanelRoot>>) {
    let (node, computed, scroll) = &mut *panel;
    if node.display == Display::None {
        wheel.clear();
        return;
    }
    for ev in wheel.read() {
        let dy = match ev.unit {
            MouseScrollUnit::Line => ev.y * 24.0,
            MouseScrollUnit::Pixel => ev.y,
        };
        let max = ((computed.content_size.y - computed.size.y) * computed.inverse_scale_factor).max(0.0);
        scroll.0.y = (scroll.0.y - dy).clamp(0.0, max);
    }
}

/// Move thumbs, fills, values and label brightness to the current tuning.
fn sync_from_tuning(
    tuning: Res<Tuning>,
    mut thumbs: Query<(&ThumbOf, &mut Node), (Without<FillOf>,)>,
    mut fills: Query<(&FillOf, &mut Node), (Without<ThumbOf>,)>,
    mut values: Query<(&ValueOf, &mut Text), Without<SpectrumButtonText>>,
    mut labels: Query<(&LabelOf, &mut TextColor)>,
    mut spectrum_button: Single<&mut Text, With<SpectrumButtonText>>,
    new_rows: Query<(), Added<ThumbOf>>,
) {
    // Also when the scene's rows have just been (re)built.
    if !tuning.is_changed() && new_rows.is_empty() {
        return;
    }
    let pos = |k: Knob, v: f32| {
        let (lo, hi) = k.range();
        ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
    };
    for (ThumbOf(k), mut node) in &mut thumbs {
        node.left = percent(pos(*k, tuning.knob(*k)) * 100.0);
    }
    for (FillOf(k), mut node) in &mut fills {
        let (lo, hi) = k.range();
        let from = if lo < 0.0 && hi > 0.0 { pos(*k, 0.0) } else { 0.0 };
        let to = pos(*k, tuning.knob(*k));
        node.left = percent(from.min(to) * 100.0);
        node.width = percent((to - from).abs() * 100.0);
    }
    for (ValueOf(k), mut text) in &mut values {
        text.0 = format_value(*k, tuning.knob(*k));
    }
    for (LabelOf(k), mut color) in &mut labels {
        let off = matches!(k, Knob::Weight(..) | Knob::Amount(..)) && tuning.knob(*k) == 0.0;
        color.0 = if off { DIM } else { TEXT };
    }
    spectrum_button.0 = spectrum_label(tuning.show_spectrum);
}

/// Show or hide block bodies when their header is clicked.
fn fold(
    expanded: Res<Expanded>,
    mut bodies: Query<(&BlockBody, &mut Node)>,
    headers: Query<(&BlockHeader, &Children)>,
    mut texts: Query<&mut Text>,
) {
    if !expanded.is_changed() {
        return;
    }
    for (BlockBody(b), mut node) in &mut bodies {
        node.display = if expanded.0.contains(b) { Display::Flex } else { Display::None };
    }
    for (BlockHeader(b), children) in &headers {
        if let Some(&first) = children.first()
            && let Ok(mut text) = texts.get_mut(first)
        {
            text.0 = if expanded.0.contains(b) { "-" } else { "+" }.into();
        }
    }
}

/// Live values in the headers, and meters in unfolded blocks.
fn live_values(
    panel: Single<&Node, With<PanelRoot>>,
    tuning: Res<Tuning>,
    signals: Res<Signals>,
    transforms: Res<Transforms>,
    expanded: Res<Expanded>,
    mut values: Query<(&BlockValue, &mut Text)>,
    mut meters: Query<(&Meter, &mut Node), Without<PanelRoot>>,
) {
    if panel.display == Display::None {
        return;
    }
    for (BlockValue(b), mut text) in &mut values {
        let s = match b {
            Block::Transformation(t) => format!("{:.2}", transforms.get(*t)),
            Block::Control(c) => format!("{:.*}", c.spec().decimals.max(0) as usize, tuning.control_value(*c, &transforms)),
        };
        if text.0 != s {
            text.0 = s;
        }
    }
    for (Meter(block, source), mut node) in &mut meters {
        if !expanded.0.contains(block) {
            continue;
        }
        let level = match source {
            Source::Signal(s) => signals.get(*s),
            Source::Transformation(t) => transforms.get(*t),
        };
        node.width = percent(level.clamp(0.0, 1.0) * 100.0);
    }
}
