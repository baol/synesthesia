use bevy::prelude::*;
use rand::Rng;

use crate::{
    field::FlowField,
    scene::Scene,
    render::BubbleAssets,
    sand::Sand,
    bubble_controls::{Control, P, wind_dir},
    tuning::{Transforms, Tuning},
};

#[derive(Component)]
pub struct Bubble {
    /// Simulated position; `Transform` is interpolated between `prev` and `pos`.
    pub pos: Vec2,
    pub prev: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    /// Per-bubble multiplier on the wind speed (about ±10%).
    pub drift: f32,
    /// Sideways wobble: phase (radians) and frequency (Hz).
    pub wobble_phase: f32,
    pub wobble_freq: f32,
    /// False once the bubble has left the band. Dead bubbles sit off-screen and
    /// are recycled at the source line: creating and destroying thousands of
    /// entities per second costs far more than moving them.
    pub alive: bool,
}

/// Dead bubbles kept for reuse beyond what the emitter needs right now; more
/// than this are despawned (a little at a time).
const POOL_SLACK: usize = 5000;

/// Order of the fixed-timestep simulation steps.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    Conduct,
    Spawn,
    Advance,
}

/// Spawning state: fractional bubbles carried over between steps.
#[derive(Resource, Default)]
struct Emitter {
    carry: f32,
}

/// Simulation rate. Rendering interpolates between steps.
const SIM_HZ: f64 = 120.0;

pub struct BubblesPlugin;

impl Plugin for BubblesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Emitter>()
            .insert_resource(Time::<Fixed>::from_hz(SIM_HZ))
            // The whole simulation only runs while the bubbles are shown.
            .configure_sets(
                FixedUpdate,
                (SimSet::Conduct, SimSet::Spawn, SimSet::Advance).chain().run_if(in_state(Scene::Bubbles)),
            )
            // On the first frame in the scene (not `OnEnter`: the first one runs
            // before PreStartup, when the bubble mesh and material don't exist yet).
            .add_systems(Update, prefill.run_if(in_state(Scene::Bubbles)).run_if(state_changed::<Scene>))
            .add_systems(FixedUpdate, emit.in_set(SimSet::Spawn))
            .add_systems(FixedUpdate, advance.in_set(SimSet::Advance))
            .init_resource::<BubbleScale>()
            .init_resource::<Jump>()
            .add_systems(
                Update,
                (update_scale, update_jump, interpolate)
                    .chain()
                    .after(crate::sand::update_sand)
                    .run_if(in_state(Scene::Bubbles)),
            );
    }
}

/// Size distribution: base × N(1, spread) clipped to ±clip, plus ~2% small outliers.
fn sample_radius(rng: &mut impl Rng, base: f32, spread: f32, clip: f32) -> f32 {
    if rng.random::<f32>() < 0.02 {
        return base * rng.random_range(0.3..0.5);
    }
    // Box–Muller.
    let u1: f32 = rng.random_range(f32::EPSILON..1.0);
    let u2: f32 = rng.random();
    let n = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
    base * (1.0 + spread * n).clamp(1.0 - clip, 1.0 + clip)
}

fn wind_velocity(tuning: &Tuning, width: f32) -> Vec2 {
    wind_dir(tuning) * tuning.base(Control::WindSpeed) * width
}

/// The band of space bubbles live in, in wind-aligned coordinates.
///
/// `s` runs along the wind, `t` across it. All bubbles are born on one straight
/// line off-screen, perpendicular to the wind (so slightly slanted), long enough
/// that every point of the screen is downwind of it. They die once they are past
/// the most downwind corner or have drifted out of the band sideways. One source
/// for everything means no seams where two emitters meet.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    /// Unit vector along the wind.
    pub along: Vec2,
    /// Unit vector across the wind.
    pub across: Vec2,
    /// Source line position (most upwind screen corner, minus a margin).
    pub s_min: f32,
    /// Sink position (most downwind screen corner, plus a margin).
    pub s_max: f32,
    pub t_min: f32,
    pub t_max: f32,
}

impl Band {
    pub fn new(wind_dir: Vec2, screen: Vec2, margin: f32) -> Self {
        let along = wind_dir.normalize_or(Vec2::NEG_X);
        let across = along.perp();
        let h = screen / 2.0;
        let corners = [
            Vec2::new(-h.x, -h.y),
            Vec2::new(h.x, -h.y),
            Vec2::new(h.x, h.y),
            Vec2::new(-h.x, h.y),
        ];
        let (mut s_min, mut s_max, mut t_min, mut t_max) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for c in corners {
            let (s, t) = (c.dot(along), c.dot(across));
            s_min = s_min.min(s);
            s_max = s_max.max(s);
            t_min = t_min.min(t);
            t_max = t_max.max(t);
        }
        Self {
            along,
            across,
            s_min: s_min - margin,
            s_max: s_max + margin,
            t_min: t_min - margin,
            t_max: t_max + margin,
        }
    }

    pub fn point(&self, s: f32, t: f32) -> Vec2 {
        self.along * s + self.across * t
    }

    pub fn contains(&self, p: Vec2) -> bool {
        let (s, t) = (p.dot(self.along), p.dot(self.across));
        s <= self.s_max && t >= self.t_min && t <= self.t_max
    }

    /// Where a bubble born on the source line at `t` would cross into the screen
    /// (grown by `margin`), travelling straight along the wind: the `s` of that
    /// point, or `None` if its path misses the screen.
    pub fn entry(&self, t: f32, screen: Vec2, margin: f32) -> Option<f32> {
        let origin = self.point(self.s_min, t);
        let half = screen / 2.0 + Vec2::splat(margin);
        // Slab test of the ray origin + along·d against the rectangle.
        let (mut d_in, mut d_out) = (0.0f32, f32::MAX);
        for axis in 0..2 {
            let (o, dir, h) = (origin[axis], self.along[axis], half[axis]);
            if dir.abs() < 1e-6 {
                if o.abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o) / dir, (h - o) / dir);
            d_in = d_in.max(a.min(b));
            d_out = d_out.min(a.max(b));
        }
        (d_in <= d_out).then_some(self.s_min + d_in)
    }
}

fn band(tuning: &Tuning, screen: Vec2) -> Band {
    Band::new(wind_dir(&tuning), screen, tuning.get(P::Radius) * 3.0)
}

/// Spawn rate that keeps `bubbles` on screen (before the music multiplier):
/// on-screen density × source length × inflow speed.
fn spawn_rate(tuning: &Tuning, band: &Band, wind_speed: f32, screen: Vec2) -> f32 {
    let density = tuning.get(P::Bubbles) / (screen.x * screen.y);
    density * (band.t_max - band.t_min) * wind_speed
}

type BubbleBundle = (
    Bubble,
    Mesh2d,
    MeshMaterial2d<crate::render::BubbleMaterial>,
    Transform,
    DespawnOnExit<Scene>,
);

/// A fresh bubble at `pos`, plus its seed (stored in z) and radius.
fn new_bubble(tuning: &Tuning, rng: &mut impl Rng, pos: Vec2, wind: Vec2) -> (Bubble, f32) {
    let radius = sample_radius(rng, tuning.get(P::Radius), tuning.get(P::SizeSpread), tuning.get(P::SizeClip));
    let drift = rng.random_range(0.9..1.1);
    // The shader reads a per-bubble seed from z (0..0.01); it also fixes draw order.
    let seed_z = rng.random_range(0.0..0.01);
    let bubble = Bubble {
        pos,
        prev: pos,
        vel: wind * drift,
        radius,
        drift,
        wobble_phase: rng.random_range(0.0..std::f32::consts::TAU),
        wobble_freq: rng.random_range(0.5..1.2),
        alive: true,
    };
    (bubble, seed_z)
}

fn bubble_transform(b: &Bubble, seed_z: f32) -> Transform {
    Transform::from_translation(b.pos.extend(seed_z)).with_scale(Vec3::splat(b.radius * 2.0))
}

fn spawn_bundle(
    assets: &BubbleAssets,
    tuning: &Tuning,
    rng: &mut impl Rng,
    pos: Vec2,
    wind: Vec2,
) -> BubbleBundle {
    let (bubble, seed_z) = new_bubble(tuning, rng, pos, wind);
    let tf = bubble_transform(&bubble, seed_z);
    (bubble, Mesh2d(assets.mesh.clone()), MeshMaterial2d(assets.material.clone()), tf, DespawnOnExit(Scene::Bubbles))
}

/// Fill the screen (plus a margin) at a moderate density whenever the scene
/// starts, so there is no empty start.
fn prefill(
    mut commands: Commands,
    assets: Res<BubbleAssets>,
    tuning: Res<Tuning>,
    window: Single<&Window>,
) {
    let size = window.size();
    let margin = tuning.get(P::Radius) * 3.0;
    let half = size / 2.0 + Vec2::splat(margin);
    let wind = wind_velocity(&tuning, size.x);
    let count = (0.6 * tuning.get(P::Bubbles) * (4.0 * half.x * half.y) / (size.x * size.y)) as usize;
    let mut rng = rand::rng();
    let batch: Vec<_> = (0..count)
        .map(|_| {
            let pos = Vec2::new(rng.random_range(-half.x..half.x), rng.random_range(-half.y..half.y));
            spawn_bundle(&assets, &tuning, &mut rng, pos, wind)
        })
        .collect();
    commands.spawn_batch(batch);
}

/// Emit bubbles, reusing dead ones first.
///
/// Conceptually every bubble is born on the one inflow line off-screen (so
/// density is even, with no seams), but it is placed straight away where its
/// straight path along the wind enters the screen. The empty stretch between
/// line and screen is skipped, so a change shows up at the screen edge at once.
///
/// How many: `bubbles` is the on-screen count at inflow rate 1; the inflow-rate
/// control scales it.
fn emit(
    mut commands: Commands,
    mut emitter: ResMut<Emitter>,
    mut bubbles: Query<(Entity, &mut Bubble, &mut Transform)>,
    assets: Res<BubbleAssets>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    field: Res<FlowField>,
    time: Res<Time>,
    window: Single<&Window>,
) {
    let size = window.size();
    let band = band(&tuning, size);
    let margin = tuning.get(P::Radius) * 3.0;
    let wind = field.wind;
    let dt = time.delta_secs();
    // Inflow across the line follows the current wind, so density on screen
    // doesn't change just because the wind picks up.
    let inflow = wind.dot(band.along).max(0.05 * wind.length());
    let rate = tuning.control(Control::InflowRate, &transforms);
    emitter.carry += spawn_rate(&tuning, &band, inflow, size) * rate * dt;
    let n = emitter.carry.floor() as usize;
    emitter.carry -= n as f32;

    let mut rng = rand::rng();
    // Jitter along the wind by up to one step of travel, so bubbles born in the
    // same step don't form a visible line. Paths that miss the screen are skipped.
    let source_point = |rng: &mut rand::rngs::ThreadRng| {
        let t = rng.random_range(band.t_min..band.t_max);
        band.entry(t, size, margin)
            .map(|s| band.point(s - rng.random_range(0.0..1.0) * inflow * dt, t))
    };

    let (mut reused, mut spare) = (0usize, 0usize);
    for (entity, mut b, mut tf) in &mut bubbles {
        if b.alive {
            continue;
        }
        if reused < n {
            reused += 1;
            let Some(pos) = source_point(&mut rng) else { continue };
            let (fresh, seed_z) = new_bubble(&tuning, &mut rng, pos, wind);
            *tf = bubble_transform(&fresh, seed_z);
            *b = fresh;
        } else {
            spare += 1;
            if spare > POOL_SLACK {
                commands.entity(entity).despawn();
            }
        }
    }

    if reused < n {
        let batch: Vec<_> = (reused..n)
            .filter_map(|_| {
                let pos = source_point(&mut rng)?;
                Some(spawn_bundle(&assets, &tuning, &mut rng, pos, wind))
            })
            .collect();
        commands.spawn_batch(batch);
    }
}

fn advance(
    mut bubbles: Query<&mut Bubble>,
    tuning: Res<Tuning>,
    field: Res<FlowField>,
    sand: Res<Sand>,
    time: Res<Time>,
    window: Single<&Window>,
) {
    let band = band(&tuning, window.size());
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let base_r = tuning.get(P::Radius);
    let tau0 = tuning.get(P::DragTime);
    let field = &*field;
    let sand = &*sand;
    let wind_speed = field.wind.length();
    bubbles.par_iter_mut().for_each(|mut b| {
        if !b.alive {
            return;
        }
        let u = field.velocity(b.pos);
        // Individual speed (drift) applies to the wind part only.
        let mut target = u + field.wind * (b.drift - 1.0);
        // A gentle sideways wobble, across the bubble's own direction of travel.
        let across = b.vel.normalize_or(Vec2::NEG_X).perp();
        let wobble = (t * b.wobble_freq * std::f32::consts::TAU + b.wobble_phase).sin();
        target += across * wobble * 0.06 * wind_speed;
        // Bigger bubbles respond more slowly (more inertia).
        let tau = tau0 * (b.radius / base_r).max(0.3);
        let k = 1.0 - (-dt / tau).exp();
        b.vel = b.vel.lerp(target, k);
        // Sand pattern: a real pull towards the nodal lines. Mass grows with
        // area, so big bubbles budge less.
        let mass = (b.radius / base_r).powi(2).max(0.2);
        let push = sand.force(b.pos) / mass;
        b.vel += push * dt;
        b.prev = b.pos;
        let v = b.vel;
        b.pos += v * dt;
        // Left the band (downwind or sideways): off-screen, ready for reuse.
        if !band.contains(b.pos) {
            b.alive = false;
        }
    });
}

/// Size factor every bubble is drawn with (1 = normal), following the
/// routed `BubbleSize` target through a critically damped spring.
#[derive(Resource)]
pub struct BubbleScale {
    pub factor: f32,
    vel: f32,
}

impl Default for BubbleScale {
    fn default() -> Self {
        Self { factor: 1.0, vel: 0.0 }
    }
}

/// Bubbles hop up and fall back (drawing only): one shared hop, which each
/// bubble scales by its own factor. A hop is launched each time the hop-impulse
/// control rises (by default: every beat), as high as it rose.
#[derive(Resource, Default)]
pub struct Jump {
    /// Height (px, for a base-size bubble at factor 1) and vertical speed.
    pub y: f32,
    vy: f32,
    /// The control's last value, and where its current rise started.
    prev: f32,
    rise_from: Option<f32>,
}

/// Rises smaller than this (in radii) don't launch a hop.
const HOP_THRESHOLD: f32 = 0.05;

fn update_jump(time: Res<Time>, tuning: Res<Tuning>, transforms: Res<Transforms>, mut jump: ResMut<Jump>) {
    let dt = time.delta_secs().min(0.05);
    let radius = tuning.get(P::Radius);
    let hop_time = tuning.get(P::HopTime);
    let value = tuning.control(Control::HopImpulse, &transforms);
    // Gravity such that a hop of 0.8 radius lasts `hop_time`.
    let g = 8.0 * 0.8 * radius / (hop_time * hop_time);
    // Launch at the top of each rise, as high as the control rose; a hop in
    // mid-air is relaunched rather than stacked.
    if value > jump.prev {
        let prev = jump.prev;
        jump.rise_from.get_or_insert(prev);
    } else if let Some(from) = jump.rise_from.take() {
        let height = (jump.prev - from) * radius;
        if height > HOP_THRESHOLD * radius {
            jump.vy = jump.vy.max((2.0 * g * height).sqrt());
        }
    }
    jump.prev = value;
    jump.vy -= g * dt;
    jump.y += jump.vy * dt;
    if jump.y <= 0.0 {
        jump.y = 0.0;
        jump.vy = jump.vy.max(0.0);
    }
}

/// Time constant of the size spring: quick enough to follow a beat, smooth
/// enough not to flicker.
const SCALE_TIME: f32 = 0.035;

fn update_scale(time: Res<Time>, tuning: Res<Tuning>, transforms: Res<Transforms>, mut scale: ResMut<BubbleScale>) {
    let dt = time.delta_secs().min(0.05);
    let target = tuning.control(Control::BubbleScale, &transforms);
    let w = 1.0 / SCALE_TIME;
    let steps = 8;
    let h = dt / steps as f32;
    for _ in 0..steps {
        let acc = w * w * (target - scale.factor) - 2.0 * w * scale.vel;
        scale.vel += acc * h;
        scale.factor += scale.vel * h;
    }
}

/// Place each bubble between its last two simulated positions, at the common
/// size factor and the sand pattern's bounce (drawing only).
fn interpolate(
    mut bubbles: Query<(&Bubble, &mut Transform)>,
    fixed: Res<Time<Fixed>>,
    sand: Res<Sand>,
    scale: Res<BubbleScale>,
    jump: Res<Jump>,
    tuning: Res<Tuning>,
) {
    let alpha = fixed.overstep_fraction();
    let sand = &*sand;
    let factor = scale.factor;
    let hop = jump.y;
    let base_r = tuning.get(P::Radius);
    bubbles.par_iter_mut().for_each(|(b, mut tf)| {
        if !b.alive {
            return;
        }
        let p = b.prev.lerp(b.pos, alpha);
        tf.translation.x = p.x;
        tf.translation.y = p.y;
        if hop > 0.0 {
            // Each bubble jumps 0.6–1.4× (from its seed in z), small ones
            // higher, so it reads as bubbles hopping, not the screen moving.
            let own = 0.6 + 0.8 * (tf.translation.z * 731.0).fract();
            tf.translation.y += hop * own * (base_r / b.radius).sqrt();
        }
        tf.scale = Vec3::splat(b.radius * 2.0 * factor * sand.swell_at(p));
    });
}
