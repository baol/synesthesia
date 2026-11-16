//! Synestesia — music-driven bubbles.
//!
//! Run:   cargo run --release                       (microphone)
//!        cargo run --release -- --file song.mp3    (play a file and react to it)
//!        cargo run --release -- --synthetic        (built-in 120 BPM test pattern)
//!        cargo run --release -- --list-devices / --device NAME
//! Keys:  Cmd-, preferences, H spectrum, D debug overlay, M cycle source (live/synthetic/silent),
//!        1-4 poke swirls, Space fake an onset, B fake a bass hit (synthetic/silent only)
//! Analyse: cargo run --release -- --analyze a.mp3 --analyze b.flac   (then open analysis/index.html)
//! Bench: cargo run --release -- --no-prefs --bubbles 30000 --bench 10

mod analysis;
mod audio;
mod beat;
mod bubbles;
mod conductor;
mod config;
mod debug;
mod features;
mod field;
mod offline;
mod prefs;
mod render;
mod sand;
mod solo;
mod spectrum_view;
mod stats;
mod tuning;

use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    diagnostic::FrameTimeDiagnosticsPlugin,
    post_process::bloom::Bloom,
    prelude::*,
    window::{PresentMode, WindowResolution},
};

fn main() {
    let cfg = config::Config::from_args();
    if !cfg.analyze.is_empty() {
        offline::run(&cfg.analyze, std::path::Path::new("analysis"));
        return;
    }
    let tuning = cfg.initial_tuning();
    let mut app = App::new();
    if let Some(solo) = cfg.solo.as_deref().and_then(solo::find) {
        app.insert_resource(solo::ActiveSolo(solo));
    }
    if let Some(fps) = cfg.fixed_fps {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(
            1.0 / fps as f64,
        )));
    }

    app.insert_resource(ClearColor(Color::BLACK))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "synestesia".into(),
                    resolution: match cfg.scale_factor {
                        Some(f) => WindowResolution::new(cfg.width, cfg.height).with_scale_factor_override(f),
                        None => WindowResolution::new(cfg.width, cfg.height),
                    },
                    // When recording clips, stay on top: macOS stops drawing
                    // covered windows, and the captured frames come out black.
                    window_level: if cfg.fixed_fps.is_some() {
                        bevy::window::WindowLevel::AlwaysOnTop
                    } else {
                        bevy::window::WindowLevel::Normal
                    },
                    present_mode: if cfg.novsync {
                        PresentMode::AutoNoVsync
                    } else {
                        PresentMode::AutoVsync
                    },
                    ..default()
                }),
                ..default()
            }),
            FrameTimeDiagnosticsPlugin::default(),
            render::BubbleRenderPlugin,
            features::FeaturesPlugin,
            tuning::TuningPlugin,
            audio::AudioPlugin,
            conductor::ConductorPlugin,
            sand::SandPlugin,
            bubbles::BubblesPlugin,
            debug::DebugPlugin,
            spectrum_view::SpectrumViewPlugin,
            prefs::PrefsPlugin,
            solo::SoloPlugin,
            stats::StatsPlugin,
        ))
        .insert_resource(cfg)
        .insert_resource(tuning)
        .add_systems(Startup, setup_camera)
        .add_systems(Update, toggle_fullscreen)
        .run();
}

/// `F` or Alt+Enter: toggle borderless fullscreen.
fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut window: Single<&mut Window>) {
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if keys.just_pressed(KeyCode::KeyF) || (alt && keys.just_pressed(KeyCode::Enter)) {
        window.mode = match window.mode {
            bevy::window::WindowMode::Windowed => {
                bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
            }
            _ => bevy::window::WindowMode::Windowed,
        };
    }
}

/// HDR camera with gentle bloom, so only the bright glints glow.
fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        // Desaturates very bright values to white, which suits grey glints.
        Tonemapping::TonyMcMapface,
        Bloom {
            intensity: 0.12,
            low_frequency_boost: 0.3,
            ..Bloom::NATURAL
        },
        DebandDither::Enabled,
    ));
}
