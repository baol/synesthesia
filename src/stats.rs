use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

use crate::{bubbles::Bubble, config::Config};

/// Shows FPS and bubble count in the window title, and in `--bench` mode
/// quits after the given time and prints a summary.
pub struct StatsPlugin;

#[derive(Resource, Default)]
struct Bench {
    title_timer: f32,
    frames: u64,
    secs: f32,
    min_count: usize,
    max_count: usize,
    worst_frame_ms: f32,
    done: bool,
    next_capture: f32,
    captures: u32,
    exit_at: f32,
}

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Bench {
            min_count: usize::MAX,
            ..default()
        })
        .add_systems(Update, stats);
    }
}

/// Seconds to ignore at the start of a benchmark (shader compile, prefill).
const WARMUP: f32 = 3.0;

fn stats(
    mut bench: ResMut<Bench>,
    cfg: Res<Config>,
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    bubbles: Query<&Bubble>,
    mut window: Single<&mut Window>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let total = bubbles.iter().filter(|b| b.alive).count();
    // Bubbles also live off-screen (between the source line and the screen), so
    // report how many are actually visible.
    let half = window.size() / 2.0;
    let count = bubbles
        .iter()
        .filter(|b| b.alive && b.pos.x.abs() <= half.x && b.pos.y.abs() <= half.y)
        .count();

    bench.title_timer += dt;
    if bench.title_timer > 0.5 {
        bench.title_timer = 0.0;
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
            .unwrap_or(0.0);
        window.title = format!("synestesia — {fps:.0} fps — {count} visible / {total} bubbles");
    }

    let Some(limit) = cfg.bench else { return };
    let elapsed = time.elapsed_secs();
    if elapsed < WARMUP || (bench.done && elapsed < bench.exit_at) {
        return;
    }
    if let Some(dir) = &cfg.capture
        && !bench.done
        && elapsed >= bench.next_capture
    {
        bench.next_capture = elapsed + cfg.capture_every;
        let path = format!("{dir}/frame_{:03}.png", bench.captures);
        bench.captures += 1;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    bench.frames += 1;
    bench.secs += dt;
    bench.min_count = bench.min_count.min(count);
    bench.max_count = bench.max_count.max(count);
    bench.worst_frame_ms = bench.worst_frame_ms.max(dt * 1000.0);
    if elapsed >= WARMUP + limit && !bench.done {
        bench.done = true;
        bench.exit_at = elapsed;
        if cfg.capture.is_some() {
            // Let the last captures reach the disk.
            bench.exit_at = elapsed + 1.0;
        }
        if let Some(path) = &cfg.screenshot {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
            // Give the capture a moment to reach the disk.
            bench.exit_at = elapsed + 1.0;
        }
        println!(
            "BENCH frames={} avg_fps={:.1} worst_frame_ms={:.2} visible_min={} visible_max={} total={} window={:.0}x{:.0}",
            bench.frames,
            bench.frames as f32 / bench.secs,
            bench.worst_frame_ms,
            bench.min_count,
            bench.max_count,
            total,
            window.width(),
            window.height(),
        );
    }
    if bench.done && elapsed >= bench.exit_at {
        exit.write(AppExit::Success);
    }
}
