//! The pendulums scene: twelve glowing pendulums hang in a row over a dark,
//! glossy floor and dance in formation. This is the "dancing" kind of scene:
//! the motion is choreography, locked to the beat and the bar.
//!
//! - **The step**: every pendulum swings out on one beat and back on the next,
//!   lingering at the end of each swing, so the turn lands on the beat.
//! - **Figures**: who swings with whom. All together; a wave running along the
//!   row; the two halves mirrored (opening and closing like curtains);
//!   neighbours against each other; pairs against pairs; the wave the other
//!   way. The figure changes every 16 counts, on the 1.
//! - **The notes**: there is one pendulum per note (C, G, D, … in fifths, so
//!   the notes of a chord hang next to each other), each in its own colour.
//!   The notes being played are the pendulums that shine.
//! - How wide they swing follows how clear the beat is: with no beat to dance
//!   to (a solo piano) they barely sway, and only the notes light up.
//! - Havoc (a drop, a mosh pit) throws them out of step.
//!
//! Drawing: one full-screen quad; `shaders/pendulums.wgsl` draws strings,
//! trails, bobs and their reflection in the floor.

use bevy::{
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
};
use rand::Rng;

use crate::{
    scene::{Scene, SceneControl, SceneParam, SceneTable},
    tuning::{ControlSpec, MusicClock, Notes, ParamSpec, Transformation as T, Transforms, Tuning, ctl, param},
};

const SWING: &str = "SWING";
const LIGHT: &str = "LIGHT";

pub const TABLE: SceneTable = SceneTable { groups: &[SWING, LIGHT], controls: &CONTROLS, params: &PARAMS };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Swing,
    Chaos,
    Bounce,
    Glow,
    NoteGlow,
    Flare,
    Trail,
    Colour,
}

impl Control {
    pub const COUNT: usize = 8;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
    pub const ALL: [Control; Self::COUNT] =
        [Control::Swing, Control::Chaos, Control::Bounce, Control::Glow, Control::NoteGlow, Control::Flare, Control::Trail, Control::Colour];
}

impl SceneControl for Control {
    const SCENE: Scene = Scene::Pendulums;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    // A clear beat is what makes them dance; loud music a little more.
    ctl("swing", "swing (degrees)", SWING, 3.0, (0.0, 40.0), 40.0, (0.0, 42.0), 12.0, &[(T::Pulse, 12.0), (T::Energy, 5.0), (T::Drive, 5.0)], 1),
    ctl("chaos", "chaos: out of step (degrees)", SWING, 0.0, (0.0, 30.0), 40.0, (0.0, 40.0), 0.0, &[(T::Havoc, 24.0), (T::Heaviness, 6.0)], 1),
    ctl("bounce", "the rail dips on the beat (px)", SWING, 0.0, (0.0, 60.0), 80.0, (0.0, 80.0), 0.0, &[(T::Groove, 26.0)], 0),
    ctl("glow", "glow of every pendulum", LIGHT, 0.05, (0.0, 1.5), 1.5, (0.0, 2.0), 0.4, &[(T::Energy, 0.1), (T::Arrival, 0.4)], 2),
    ctl("note_glow", "glow of the notes being played", LIGHT, 1.3, (0.0, 3.0), 3.0, (0.0, 3.0), 0.0, &[], 2),
    ctl("flare", "flare on the beat", LIGHT, 0.0, (0.0, 1.5), 1.5, (0.0, 1.5), 0.0, &[(T::Dance, 0.7)], 2),
    ctl("trail", "trail length", LIGHT, 0.3, (0.0, 1.0), 1.0, (0.0, 1.0), 0.5, &[(T::Energy, 0.5)], 2),
    ctl("colour", "colour", LIGHT, 0.55, (0.0, 1.0), 1.0, (0.0, 1.0), 0.8, &[(T::Harmony, 0.4)], 2),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    FigureCounts,
    BobSize,
    Reflection,
}

impl P {
    pub const COUNT: usize = 3;
    #[cfg(test)]
    pub const ALL: [P; Self::COUNT] = [P::FigureCounts, P::BobSize, P::Reflection];
}

impl SceneParam for P {
    const SCENE: Scene = Scene::Pendulums;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    param("figure_counts", "counts per figure", SWING, 16.0, (8.0, 64.0), 0),
    param("bob_size", "pendulum size (px)", LIGHT, 24.0, (8.0, 60.0), 0),
    param("reflection", "floor reflection", LIGHT, 0.35, (0.0, 1.0), 2),
];

// ===========================================================================
// The dance.

/// Pendulums: one per pitch class.
pub const N: usize = 12;
/// Points kept of each pendulum's path, and the time between them (s).
pub const TRAIL: usize = 12;
const TRAIL_STEP: f32 = 0.014;
/// The spring that carries each pendulum to where the dance wants it: its
/// frequency (Hz) and damping. It gives the swing its weight and hides the
/// music clock's small corrections.
const SPRING_HZ: f32 = 5.0;
const SPRING_DAMPING: f32 = 0.8;
/// A figure change is eased in over about this long (s).
const FIGURE_EASE: f32 = 0.35;
/// In chaos, each pendulum is thrown a new way this often (s).
const CHAOS_STEP: f32 = 0.18;

/// Who swings with whom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Figure {
    Together,
    Wave,
    Mirror,
    Neighbours,
    Pairs,
    WaveBack,
}

impl Figure {
    pub const ALL: [Figure; 6] = [Figure::Together, Figure::Wave, Figure::Mirror, Figure::Neighbours, Figure::Pairs, Figure::WaveBack];

    /// How many beats pendulum `i` is behind the first (a swing out and back
    /// takes 2 beats, so 1 is the exact opposite).
    pub fn delay(self, i: usize) -> f32 {
        let along = i as f32 / (N - 1) as f32;
        match self {
            Figure::Together => 0.0,
            Figure::Wave => 2.0 * along,
            Figure::Mirror => (i >= N / 2) as u8 as f32,
            Figure::Neighbours => (i % 2) as f32,
            Figure::Pairs => (i / 2 % 2) as f32,
            Figure::WaveBack => 2.0 * (1.0 - along),
        }
    }
}

/// Where a pendulum is in its swing (−1..1) at `count` beats: out to one side
/// on each even count, to the other on each odd one, lingering at both ends.
pub fn step(count: f32) -> f32 {
    let c = (count * std::f32::consts::PI).cos();
    c.signum() * c.abs().powf(0.7)
}

#[derive(Resource)]
pub struct Dance {
    /// Each pendulum's angle from straight down (rad) and its speed.
    angle: [f32; N],
    speed: [f32; N],
    /// Each pendulum's delay (beats), easing towards the figure's.
    delay: [f32; N],
    pub figure: usize,
    /// The count within the figure, last frame (a drop back to 0 is the 1).
    last_count: f32,
    /// Chaos: where each pendulum is being thrown (−1..1), and until when.
    thrown: [f32; N],
    throw_in: f32,
    /// Each bob's recent path, newest first, and the time since the last point.
    trail: [[Vec2; TRAIL]; N],
    trail_clock: f32,
    started: bool,
}

impl Default for Dance {
    fn default() -> Self {
        Self {
            angle: [0.0; N],
            speed: [0.0; N],
            delay: [0.0; N],
            figure: 0,
            last_count: 0.0,
            thrown: [0.0; N],
            throw_in: 0.0,
            trail: [[Vec2::ZERO; TRAIL]; N],
            trail_clock: 0.0,
            started: false,
        }
    }
}

/// What the music asks of the dance this frame.
#[derive(Clone, Copy, Debug)]
pub struct Lead {
    /// Swing and chaos, in radians.
    pub swing: f32,
    pub chaos: f32,
    /// The count within the current figure (0 = on its 1).
    pub count: f32,
}

impl Dance {
    /// Advance the pendulums' angles by `dt`.
    pub fn step(&mut self, lead: Lead, dt: f32, rng: &mut impl Rng) {
        // A new figure on the 1.
        if lead.count < self.last_count - 1.0 {
            self.figure = (self.figure + 1) % Figure::ALL.len();
        }
        self.last_count = lead.count;
        self.throw_in -= dt;
        if self.throw_in <= 0.0 {
            self.throw_in = CHAOS_STEP;
            for t in &mut self.thrown {
                *t = rng.random_range(-1.0..1.0);
            }
        }
        let figure = Figure::ALL[self.figure];
        let ease = 1.0 - (-dt / FIGURE_EASE).exp();
        let w = std::f32::consts::TAU * SPRING_HZ;
        // Small steps keep the spring stable at low frame rates.
        let steps = (dt / 0.008).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for i in 0..N {
            self.delay[i] += (figure.delay(i) - self.delay[i]) * ease;
            let target = lead.swing * step(lead.count - self.delay[i]) + lead.chaos * self.thrown[i];
            for _ in 0..steps {
                self.speed[i] += (w * w * (target - self.angle[i]) - 2.0 * SPRING_DAMPING * w * self.speed[i]) * h;
                self.angle[i] += self.speed[i] * h;
            }
        }
    }

    pub fn angles(&self) -> &[f32; N] {
        &self.angle
    }
}

/// Where the row hangs on a screen of `size`: each pivot and string length,
/// and the height of the floor.
pub struct Stage {
    pub pivots: [Vec2; N],
    pub lengths: [f32; N],
    pub floor_y: f32,
}

impl Stage {
    pub fn new(size: Vec2, dip: f32) -> Self {
        let half = size / 2.0;
        let rail_y = 0.86 * half.y;
        let floor_y = -0.6 * half.y;
        let drop = rail_y - floor_y;
        let middle = (N - 1) as f32 / 2.0;
        Self {
            pivots: std::array::from_fn(|i| Vec2::new((i as f32 - middle) / (N as f32 / 2.0) * 0.8 * half.x, rail_y - dip)),
            // Longest in the middle, like a chandelier; never down to the floor.
            lengths: std::array::from_fn(|i| drop * (0.56 + 0.26 * (std::f32::consts::PI * (i as f32 - middle) / N as f32).cos())),
            floor_y,
        }
    }

    pub fn bob(&self, i: usize, angle: f32) -> Vec2 {
        self.pivots[i] + self.lengths[i] * Vec2::new(angle.sin(), -angle.cos())
    }
}

#[derive(Clone, Debug, ShaderType)]
pub struct PendulumUniform {
    /// xy: the bob (world px); z: its radius (px); w: its light (0..~3).
    pub bobs: [Vec4; N],
    /// rgb: the bob's colour (linear); a: the flare on the beat (0..1.5).
    pub colours: [Vec4; N],
    /// xy: where the string hangs from.
    pub pivots: [Vec4; N],
    /// xy: the bob's recent path, `TRAIL` points per bob, newest first.
    pub trail: [Vec4; N * TRAIL],
    /// xy: half the screen size; z: the floor's height; w: trail length (0..1).
    pub frame: Vec4,
    /// x: floor reflection (0..1); y: the rail's height; z: time (s).
    pub look: Vec4,
}

impl Default for PendulumUniform {
    fn default() -> Self {
        Self { bobs: [Vec4::ZERO; N], colours: [Vec4::ZERO; N], pivots: [Vec4::ZERO; N], trail: [Vec4::ZERO; N * TRAIL], frame: Vec4::ZERO, look: Vec4::ZERO }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct PendulumMaterial {
    #[uniform(0)]
    pub row: PendulumUniform,
}

impl Material2d for PendulumMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/pendulums.wgsl".into()
    }
}

/// The quad the scene is drawn on.
#[derive(Component)]
struct StageQuad;

pub struct PendulumsPlugin;

impl Plugin for PendulumsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<PendulumMaterial>::default())
            .init_resource::<Dance>()
            .add_systems(OnEnter(Scene::Pendulums), spawn_stage)
            .add_systems(Update, dance.after(crate::tuning::SignalSet).run_if(in_state(Scene::Pendulums)));
    }
}

fn spawn_stage(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<PendulumMaterial>>, mut state: ResMut<Dance>) {
    *state = Dance::default();
    commands.spawn((
        StageQuad,
        DespawnOnExit(Scene::Pendulums),
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(materials.add(PendulumMaterial::default())),
        Transform::default(),
    ));
}

/// The colour of the pendulum at place `i` in the row: once round the colour
/// wheel along the circle of fifths.
fn colour(i: usize, saturation: f32) -> Vec3 {
    let c = Color::hsv(360.0 * i as f32 / N as f32, saturation.clamp(0.0, 1.0), 1.0).to_linear();
    Vec3::new(c.red, c.green, c.blue)
}

#[allow(clippy::too_many_arguments)]
fn dance(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    clock: Res<MusicClock>,
    notes: Res<Notes>,
    screen: Single<&Window>,
    mut state: ResMut<Dance>,
    mut quad: Query<(&mut Transform, &MeshMaterial2d<PendulumMaterial>), With<StageQuad>>,
    mut materials: ResMut<Assets<PendulumMaterial>>,
) {
    let c = |control| tuning.control(control, &transforms);
    let dt = time.delta_secs().min(0.05);
    let size = screen.size();
    let counts = (tuning.get(P::FigureCounts) / 2.0).round().max(1.0) as u32 * 2;
    let lead = Lead { swing: c(Control::Swing).to_radians(), chaos: c(Control::Chaos).to_radians(), count: clock.count(counts) };
    state.step(lead, dt, &mut rand::rng());

    let stage = Stage::new(size, c(Control::Bounce));
    let bobs: [Vec2; N] = std::array::from_fn(|i| stage.bob(i, state.angles()[i]));
    if !state.started {
        state.started = true;
        state.trail = bobs.map(|b| [b; TRAIL]);
    }
    state.trail_clock += dt;
    while state.trail_clock >= TRAIL_STEP {
        state.trail_clock -= TRAIL_STEP;
        for (path, &bob) in state.trail.iter_mut().zip(&bobs) {
            path.rotate_right(1);
            path[0] = bob;
        }
    }

    let Ok((mut tf, handle)) = quad.single_mut() else { return };
    tf.scale = size.extend(1.0);
    let Some(mut material) = materials.get_mut(&handle.0) else { return };
    let u = &mut material.row;
    let (glow, note_glow, flare, saturation) = (c(Control::Glow), c(Control::NoteGlow), c(Control::Flare), c(Control::Colour));
    let radius = tuning.get(P::BobSize);
    for i in 0..N {
        // Squared: the notes of the chord stand out from what merely rings along.
        let note = notes.chroma[Notes::fifth(i)].powi(2);
        u.bobs[i] = bobs[i].extend(radius).extend(glow + note_glow * note);
        u.colours[i] = colour(i, saturation).extend(flare);
        u.pivots[i] = stage.pivots[i].extend(0.0).extend(0.0);
        for k in 0..TRAIL {
            u.trail[i * TRAIL + k] = state.trail[i][k].extend(0.0).extend(0.0);
        }
    }
    u.frame = Vec4::new(size.x / 2.0, size.y / 2.0, stage.floor_y, c(Control::Trail));
    u.look = Vec4::new(tuning.get(P::Reflection), stage.pivots[0].y, time.elapsed_secs(), 0.0);
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

    #[test]
    fn the_turn_lands_on_the_beat() {
        assert!((step(0.0) - 1.0).abs() < 1e-5 && (step(1.0) + 1.0).abs() < 1e-5 && (step(2.0) - 1.0).abs() < 1e-4);
        assert!(step(0.5).abs() < 1e-3);
        // Lingering: a quarter of a beat after the turn, still most of the way out.
        assert!(step(0.25) > 0.75);
    }

    #[test]
    fn figures_put_the_right_pendulums_against_each_other() {
        // Opposite: a whole beat apart.
        assert_eq!(Figure::Mirror.delay(0) - Figure::Mirror.delay(N - 1), -1.0);
        assert_eq!(Figure::Neighbours.delay(3) - Figure::Neighbours.delay(4), 1.0);
        assert_eq!(Figure::Pairs.delay(0), Figure::Pairs.delay(1));
        assert!((0..N).all(|i| Figure::Together.delay(i) == 0.0));
        // The wave ends where it began (a swing takes 2 beats).
        assert_eq!(Figure::Wave.delay(N - 1), 2.0);
    }

    /// At 120 BPM for a few bars: the pendulums swing nearly as wide as asked,
    /// turn close to the beat, change figure on the 1, and stay off the floor.
    #[test]
    fn they_dance_on_the_beat_and_change_figure() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut dance = Dance::default();
        let swing = 30f32.to_radians();
        let (dt, bps) = (1.0 / 120.0, 2.0);
        let stage = Stage::new(Vec2::new(1920.0, 1080.0), 0.0);
        let (mut widest, mut at_beat) = (0.0f32, Vec::new());
        for frame in 0..(40.0 / dt) as usize {
            let beats = frame as f32 * dt * bps;
            dance.step(Lead { swing, chaos: 0.0, count: beats % 16.0 }, dt, &mut rng);
            if beats < 14.0 {
                // Still the first figure: all together.
                assert_eq!(dance.figure, 0);
                widest = widest.max(dance.angles()[0].abs());
                if beats > 4.0 && (beats - beats.round()).abs() < dt * bps / 2.0 {
                    at_beat.push(dance.angles()[0].abs());
                }
            }
            for i in 0..N {
                assert!(stage.bob(i, dance.angles()[i]).y > stage.floor_y + 30.0, "pendulum {i} reaches the floor");
            }
        }
        assert!(widest > 0.85 * swing && widest < 1.15 * swing, "widest {} of {}", widest.to_degrees(), swing.to_degrees());
        let mean = at_beat.iter().sum::<f32>() / at_beat.len() as f32;
        assert!(mean > 0.8 * swing, "on the beat at {} of {} degrees", mean.to_degrees(), swing.to_degrees());
        // 80 beats = 5 figures of 16 counts.
        assert_eq!(dance.figure, 4);
    }
}
