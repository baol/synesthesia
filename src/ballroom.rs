//! The ballroom scene: a dance floor seen from above, where couples of
//! glowing dancers glide and turn. Like the pendulums, this is the "dancing"
//! kind of scene: the motion is choreography, locked to the beat and the bar.
//! Unlike them, nobody hangs from anything: every dancer moves freely.
//!
//! - **Couples**: each couple turns about its middle, a step at a time: the
//!   turn eases in and out of every beat, so it settles as the beat lands.
//!   On the beat the partners step apart a little and come back together.
//! - **Figures**: where the couples are on the floor. A promenade round the
//!   floor (anticlockwise, the line of dance); rows that step towards each
//!   other for two counts and back for two; two rings going opposite ways; a
//!   wheel of four spokes; and every couple wandering freely. The figure
//!   changes every 16 counts, on the 1, and the couples glide to their new
//!   places. They keep a little room between them.
//! - **The notes**: 24 couples, two per note (C, G, D, … in fifths, so the
//!   notes of a chord are neighbouring colours). The notes being played are the
//!   couples that shine.
//! - How far they travel and turn follows how clear the beat is: with no beat
//!   to dance to they barely move, and only the notes light up.
//! - Havoc (a drop, a mosh pit) breaks the holds: dancers are thrown apart.
//! - A singer sets the disco ball turning: flecks of light sweep round the
//!   room.
//! - **Grounds**: where they dance, a constant of the scene (`G` cycles it):
//!   on water (each dancer rings the water around it, and is mirrored in it),
//!   in space (among stars and a faint nebula), in the sky (over clouds that
//!   catch their light), on black, on a mirror (each dancer over its own
//!   reflection, a little nearer the middle, as seen from above).
//!
//! Drawing: one full-screen quad; `shaders/ballroom.wgsl`. A ballroom is only the
//! inspiration: nothing of the room is drawn but the disco ball's flecks.

use bevy::{
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
};
use rand::Rng;
use std::f32::consts::{PI, TAU};

use crate::{
    scene::{Scene, SceneControl, SceneParam, SceneTable},
    tuning::{ControlSpec, MusicClock, Notes, ParamSpec, Transformation as T, Transforms, Tuning, ctl, param},
};

const DANCE: &str = "DANCE";
const LIGHT: &str = "LIGHT";

pub const TABLE: SceneTable = SceneTable { groups: &[DANCE, LIGHT], controls: &CONTROLS, params: &PARAMS };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Travel,
    Turn,
    StepApart,
    Spread,
    Scatter,
    HouseLights,
    Glow,
    NoteGlow,
    Flare,
    Trail,
    Colour,
    DiscoBall,
}

impl Control {
    pub const COUNT: usize = 12;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
    pub const ALL: [Control; Self::COUNT] = [
        Control::Travel,
        Control::Turn,
        Control::StepApart,
        Control::Spread,
        Control::Scatter,
        Control::HouseLights,
        Control::Glow,
        Control::NoteGlow,
        Control::Flare,
        Control::Trail,
        Control::Colour,
        Control::DiscoBall,
    ];
}

impl SceneControl for Control {
    const SCENE: Scene = Scene::Ballroom;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    // A clear beat is what makes them dance; music that makes you move, more.
    ctl("travel", "travel (screen heights per bar)", DANCE, 0.02, (0.0, 0.4), 0.5, (0.0, 0.6), 0.15, &[(T::Pulse, 0.15), (T::Drive, 0.12), (T::Tempo, 0.05)], 2),
    ctl("turn", "turn (degrees per beat)", DANCE, 10.0, (0.0, 180.0), 180.0, (0.0, 270.0), 60.0, &[(T::Pulse, 60.0), (T::Energy, 30.0)], 0),
    ctl("step_apart", "partners step apart on the beat (px)", DANCE, 0.0, (0.0, 40.0), 60.0, (0.0, 80.0), 0.0, &[(T::Groove, 16.0)], 0),
    ctl("spread", "floor used (x)", DANCE, 0.6, (0.2, 1.0), 1.0, (0.2, 1.1), 0.85, &[(T::Fullness, 0.3), (T::Energy, 0.15)], 2),
    ctl("scatter", "scatter: holds broken (px)", DANCE, 0.0, (0.0, 200.0), 400.0, (0.0, 400.0), 0.0, &[(T::Havoc, 160.0), (T::Heaviness, 30.0)], 0),
    ctl("house_lights", "house lights (a glow in the middle)", LIGHT, 0.15, (0.0, 1.0), 1.5, (0.0, 1.5), 0.4, &[(T::Energy, 0.35), (T::Arrival, 0.4)], 2),
    ctl("glow", "glow of every dancer", LIGHT, 0.1, (0.0, 1.5), 1.5, (0.0, 2.0), 0.5, &[(T::Energy, 0.3)], 2),
    ctl("note_glow", "glow of the notes being played", LIGHT, 1.3, (0.0, 3.0), 3.0, (0.0, 3.0), 0.0, &[], 2),
    ctl("flare", "flare on the beat", LIGHT, 0.0, (0.0, 1.5), 1.5, (0.0, 1.5), 0.0, &[(T::Dance, 0.6)], 2),
    ctl("trail", "trail length", LIGHT, 0.3, (0.0, 1.0), 1.0, (0.0, 1.0), 0.6, &[(T::Energy, 0.5)], 2),
    ctl("colour", "colour", LIGHT, 0.55, (0.0, 1.0), 1.0, (0.0, 1.0), 0.8, &[(T::Harmony, 0.4)], 2),
    ctl("disco_ball", "disco ball (flecks of light)", LIGHT, 0.0, (0.0, 1.5), 1.5, (0.0, 1.5), 0.0, &[(T::Singing, 1.0)], 2),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    FigureCounts,
    Hold,
    DancerSize,
    Ground,
}

impl P {
    pub const COUNT: usize = 4;
    #[cfg(test)]
    pub const ALL: [P; Self::COUNT] = [P::FigureCounts, P::Hold, P::DancerSize, P::Ground];
}

impl SceneParam for P {
    const SCENE: Scene = Scene::Ballroom;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    param("figure_counts", "counts per figure", DANCE, 16.0, (8.0, 64.0), 0),
    param("hold", "partners' distance in hold (px)", DANCE, 26.0, (10.0, 60.0), 0),
    param("dancer_size", "dancer size (px)", LIGHT, 8.0, (3.0, 24.0), 1),
    param("ground", "ground (water, space, sky, black, mirror)", LIGHT, 0.0, (0.0, 4.0), 0),
];

// ===========================================================================
// The dance.

/// Couples on the floor (two per note), and dancers.
pub const COUPLES: usize = 24;
pub const DANCERS: usize = 2 * COUPLES;
/// Points kept of each dancer's path, and the time between them (s).
pub const TRAIL: usize = 10;
const TRAIL_STEP: f32 = 0.035;
/// The spring that carries each couple to its place: its frequency (Hz) and
/// damping. Slow, so changing places is a glide across the floor.
const GLIDE_HZ: f32 = 1.1;
const GLIDE_DAMPING: f32 = 0.9;
/// The spring that throws dancers apart in havoc, and how often each dancer
/// is thrown a new way (s).
const SCATTER_HZ: f32 = 2.5;
const SCATTER_DAMPING: f32 = 0.5;
const CHAOS_STEP: f32 = 0.3;
/// Couples closer than this many holds push each other away.
const ROOM: f32 = 2.4;
/// The dance floor, as a fraction of the half screen.
pub const FLOOR: Vec2 = Vec2::new(0.9, 0.86);
/// The grounds, in the order of the `ground` constant.
pub const GROUNDS: [&str; 5] = ["water", "space", "sky", "black", "mirror"];

/// Where the couples are on the floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Figure {
    Promenade,
    Rows,
    Rings,
    Wheel,
    Free,
}

impl Figure {
    pub const ALL: [Figure; 5] = [Figure::Promenade, Figure::Rows, Figure::Rings, Figure::Wheel, Figure::Free];

    /// Couple `i`'s place, in floor units (−1..1 on each axis), after
    /// travelling `travelled` (floor units along its path), at `stride` floor
    /// units a beat, `count` beats into the figure. `free` is where it has
    /// wandered to.
    pub fn place(self, i: usize, travelled: f32, stride: f32, count: f32, free: Vec2) -> Vec2 {
        let ring = |r: f32, angle: f32| r * Vec2::from_angle(angle);
        match self {
            // Round the floor, anticlockwise.
            Figure::Promenade => {
                let r = 0.8;
                ring(r, TAU * i as f32 / COUPLES as f32 + travelled / r)
            }
            // Four rows of six; the two halves step towards each other for two
            // counts and back for two (as far as they would travel in two beats).
            Figure::Rows => {
                let (row, col) = ((i / 6) as f32, (i % 6) as f32);
                let y = 0.72 * (row - 1.5) / 1.5;
                let towards = 0.5 - 0.5 * (PI * count / 2.0).cos();
                let step = (2.0 * stride).min(0.14) * towards;
                Vec2::new(0.8 * (col - 2.5) / 2.5, y - y.signum() * step)
            }
            // Eight inside going clockwise, sixteen outside anticlockwise.
            Figure::Rings => {
                if i.is_multiple_of(3) {
                    let r = 0.32;
                    ring(r, TAU * (i / 3) as f32 / 8.0 - travelled / r)
                } else {
                    let r = 0.82;
                    ring(r, TAU * (i - i / 3 - 1) as f32 / 16.0 + travelled / r)
                }
            }
            // Four spokes of six, turning together, clockwise.
            Figure::Wheel => {
                let (spoke, k) = ((i % 4) as f32, (i / 4) as f32);
                let r = 0.16 + 0.13 * k;
                ring(r, TAU * spoke / 4.0 - travelled / 0.81)
            }
            Figure::Free => free,
        }
    }
}

/// How far a couple has turned (in turns of the beat's worth) at `count`:
/// a whole step per beat, easing in and out of each beat.
pub fn stepped(count: f32) -> f32 {
    let f = count.fract();
    count.floor() + f - (TAU * f).sin() / TAU
}

#[derive(Resource)]
pub struct Dance {
    /// Each couple's middle (px) and its velocity.
    centre: [Vec2; COUPLES],
    velocity: [Vec2; COUPLES],
    /// How far each couple has turned (rad).
    turned: [f32; COUPLES],
    /// Each dancer's offset from its place in the hold (havoc), its velocity,
    /// and where it is being thrown (unit disc).
    scatter: [Vec2; DANCERS],
    scatter_velocity: [Vec2; DANCERS],
    thrown: [Vec2; DANCERS],
    throw_in: f32,
    /// Free figure: where each couple has wandered (floor units), and its heading.
    free: [Vec2; COUPLES],
    heading: [f32; COUPLES],
    /// Floor units travelled along the figures' paths.
    travelled: f32,
    pub figure: usize,
    /// The count within the figure, last frame (a drop back to 0 is the 1).
    last_count: f32,
    /// Each dancer's recent path, newest first, and the time since the last point.
    trail: [[Vec2; TRAIL]; DANCERS],
    trail_clock: f32,
    started: bool,
}

impl Default for Dance {
    fn default() -> Self {
        Self {
            centre: [Vec2::ZERO; COUPLES],
            velocity: [Vec2::ZERO; COUPLES],
            turned: std::array::from_fn(|i| i as f32 * 2.4),
            scatter: [Vec2::ZERO; DANCERS],
            scatter_velocity: [Vec2::ZERO; DANCERS],
            thrown: [Vec2::ZERO; DANCERS],
            throw_in: 0.0,
            // Spread over the floor to begin with (a golden-angle spiral).
            free: std::array::from_fn(|i| 0.8 * ((i as f32 + 0.5) / COUPLES as f32).sqrt() * Vec2::from_angle(i as f32 * 2.39996)),
            heading: std::array::from_fn(|i| i as f32 * 1.7),
            travelled: 0.0,
            figure: 0,
            last_count: 0.0,
            trail: [[Vec2::ZERO; TRAIL]; DANCERS],
            trail_clock: 0.0,
            started: false,
        }
    }
}

/// What the music asks of the dance this frame.
#[derive(Clone, Copy, Debug)]
pub struct Lead {
    /// Travel (screen heights per bar), turn (rad per beat), the partners'
    /// distance (px), how much of the floor is used, and scatter (px).
    pub travel: f32,
    pub turn: f32,
    pub hold: f32,
    pub spread: f32,
    pub scatter: f32,
    /// The count within the current figure (0 = on its 1), and its length.
    pub count: f32,
    pub counts: f32,
}

impl Dance {
    /// The floor's half size (px) on a screen of `size`, at `spread`.
    pub fn floor(size: Vec2, spread: f32) -> Vec2 {
        size / 2.0 * FLOOR * spread
    }

    /// Advance the dance by `dt` on a screen of `size`.
    pub fn step(&mut self, lead: Lead, size: Vec2, dt: f32, rng: &mut impl Rng) {
        // Beats gone by since last frame (a new figure starts the count again;
        // a jump of the 1 is not danced).
        let mut beats = lead.count - self.last_count;
        if beats < -1.0 {
            self.figure = (self.figure + 1) % Figure::ALL.len();
            beats += lead.counts;
        }
        let beats = beats.clamp(0.0, 0.5);
        let turn = stepped(lead.count) - stepped(self.last_count);
        let turn = if turn < -1.0 { turn + lead.counts } else { turn }.clamp(0.0, 1.0);
        self.last_count = lead.count;

        let floor = Self::floor(size, lead.spread);
        // Travel per bar is in screen heights; the figures go in floor units.
        let stride = lead.travel * size.y / 4.0 / floor.y.max(1.0);
        let along = stride * beats;
        self.travelled += along;
        self.wander(along, rng);

        let figure = Figure::ALL[self.figure];
        let w = TAU * GLIDE_HZ;
        let room = ROOM * lead.hold.max(10.0);
        let steps = (dt / 0.008).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        let targets: [Vec2; COUPLES] = std::array::from_fn(|i| figure.place(i, self.travelled, stride, lead.count % 4.0, self.free[i]) * floor);
        if !self.started {
            self.centre = targets;
        }
        for _ in 0..steps {
            #[allow(clippy::needless_range_loop)] // centre[i] against every centre[j]
            for i in 0..COUPLES {
                let mut push = Vec2::ZERO;
                for j in 0..COUPLES {
                    let d = self.centre[i] - self.centre[j];
                    let l = d.length();
                    if j != i && l < room {
                        push += d / l.max(1e-3) * (room - l) / room;
                    }
                }
                let a = w * w * (targets[i] - self.centre[i] + 0.5 * room * push) - 2.0 * GLIDE_DAMPING * w * self.velocity[i];
                self.velocity[i] += a * h;
                self.centre[i] += self.velocity[i] * h;
            }
        }
        // Natural turns: clockwise.
        for t in &mut self.turned {
            *t -= lead.turn * turn;
        }

        self.throw_in -= dt;
        if self.throw_in <= 0.0 {
            self.throw_in = CHAOS_STEP;
            for t in &mut self.thrown {
                *t = Vec2::from_angle(rng.random_range(0.0..TAU)) * rng.random_range(0.3f32..1.0).sqrt();
            }
        }
        let w = TAU * SCATTER_HZ;
        for k in 0..DANCERS {
            let target = lead.scatter * self.thrown[k];
            for _ in 0..steps {
                let a = w * w * (target - self.scatter[k]) - 2.0 * SCATTER_DAMPING * w * self.scatter_velocity[k];
                self.scatter_velocity[k] += a * h;
                self.scatter[k] += self.scatter_velocity[k] * h;
            }
        }
        self.started = true;
    }

    /// Free figure: every couple walks `along` (floor units) on its own
    /// heading, turning a little at random and away from the floor's edge.
    fn wander(&mut self, along: f32, rng: &mut impl Rng) {
        for (p, a) in self.free.iter_mut().zip(&mut self.heading) {
            *a += rng.random_range(-1.0..1.0) * 6.0 * along;
            let mut dir = Vec2::from_angle(*a);
            if p.length() > 0.8 && dir.dot(*p) > 0.0 {
                // Back towards the middle.
                dir = (dir - 2.0 * dir.dot(p.normalize()) * p.normalize()).normalize();
                *a = dir.to_angle();
            }
            *p += dir * along;
        }
    }

    /// Where each dancer is (px): the leader and follower of couple `c` are
    /// `2c` and `2c + 1`, `hold` apart.
    pub fn dancers(&self, hold: f32) -> [Vec2; DANCERS] {
        std::array::from_fn(|k| {
            let c = k / 2;
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            self.centre[c] + side * hold / 2.0 * Vec2::from_angle(self.turned[c]) + self.scatter[k]
        })
    }
}

#[derive(Clone, Debug, ShaderType)]
pub struct BallroomUniform {
    /// xy: the dancer (world px); z: its radius (px); w: its light (0..~3).
    pub dancers: [Vec4; DANCERS],
    /// rgb: the dancer's colour (linear); a: the flare on the beat (0..1.5).
    pub colours: [Vec4; DANCERS],
    /// xy: the dancer's recent path, `TRAIL` points per dancer, newest first.
    pub trail: [Vec4; DANCERS * TRAIL],
    /// xy: half the screen size; z: house lights; w: trail length (0..1).
    pub frame: Vec4,
    /// x: the ground (see [`GROUNDS`]); y: time (s); z: the disco ball; w: the partners' hold (px).
    pub look: Vec4,
}

impl Default for BallroomUniform {
    fn default() -> Self {
        Self { dancers: [Vec4::ZERO; DANCERS], colours: [Vec4::ZERO; DANCERS], trail: [Vec4::ZERO; DANCERS * TRAIL], frame: Vec4::ZERO, look: Vec4::ZERO }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct BallroomMaterial {
    #[uniform(0)]
    pub floor: BallroomUniform,
}

impl Material2d for BallroomMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/ballroom.wgsl".into()
    }
}

/// The quad the scene is drawn on.
#[derive(Component)]
struct FloorQuad;

pub struct BallroomPlugin;

impl Plugin for BallroomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<BallroomMaterial>::default())
            .init_resource::<Dance>()
            .add_systems(OnEnter(Scene::Ballroom), spawn_floor)
            .add_systems(Update, (dance.after(crate::tuning::SignalSet), next_ground).run_if(in_state(Scene::Ballroom)));
    }
}

fn spawn_floor(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<BallroomMaterial>>, mut state: ResMut<Dance>) {
    *state = Dance::default();
    commands.spawn((
        FloorQuad,
        DespawnOnExit(Scene::Ballroom),
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(materials.add(BallroomMaterial::default())),
        Transform::default(),
    ));
}

/// `G`: dance on the next ground (and remember it).
fn next_ground(keys: Res<ButtonInput<KeyCode>>, mut tuning: ResMut<Tuning>, cfg: Res<crate::config::Config>) {
    if keys.just_pressed(KeyCode::KeyG) {
        let next = (tuning.get(P::Ground).round() as usize + 1) % GROUNDS.len();
        tuning.set_knob(crate::tuning::Knob::Param(P::Ground.id()), next as f32);
        crate::prefs::save(&tuning, &cfg);
    }
}

/// The colour of couple `c`: its note's place on the circle of fifths, round
/// the colour wheel (so a chord's notes are neighbouring colours).
fn colour(c: usize, saturation: f32) -> Vec3 {
    let c = Color::hsv(360.0 * (c % 12) as f32 / 12.0, saturation.clamp(0.0, 1.0), 1.0).to_linear();
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
    mut quad: Query<(&mut Transform, &MeshMaterial2d<BallroomMaterial>), With<FloorQuad>>,
    mut materials: ResMut<Assets<BallroomMaterial>>,
) {
    let c = |control| tuning.control(control, &transforms);
    let dt = time.delta_secs().min(0.05);
    let size = screen.size();
    // Whole bars, so the rows' four-count step starts on the figure's 1.
    let counts = (tuning.get(P::FigureCounts) / 4.0).round().max(1.0) as u32 * 4;
    let hold = tuning.get(P::Hold) + c(Control::StepApart);
    let lead = Lead {
        travel: c(Control::Travel),
        turn: c(Control::Turn).to_radians(),
        hold,
        spread: c(Control::Spread),
        scatter: c(Control::Scatter),
        count: clock.count(counts),
        counts: counts as f32,
    };
    let started = state.started;
    state.step(lead, size, dt, &mut rand::rng());

    let dancers = state.dancers(hold);
    // The note each couple dances, squared: the chord stands out from what merely rings along.
    let note = |couple: usize| notes.chroma[Notes::fifth(couple % 12)].powi(2);
    if !started {
        state.trail = dancers.map(|d| [d; TRAIL]);
    }
    state.trail_clock += dt;
    while state.trail_clock >= TRAIL_STEP {
        state.trail_clock -= TRAIL_STEP;
        for (path, &d) in state.trail.iter_mut().zip(&dancers) {
            path.rotate_right(1);
            path[0] = d;
        }
    }

    let Ok((mut tf, handle)) = quad.single_mut() else { return };
    tf.scale = size.extend(1.0);
    let Some(mut material) = materials.get_mut(&handle.0) else { return };
    let u = &mut material.floor;
    let (glow, note_glow, flare, saturation) = (c(Control::Glow), c(Control::NoteGlow), c(Control::Flare), c(Control::Colour));
    let radius = tuning.get(P::DancerSize);
    for k in 0..DANCERS {
        let couple = k / 2;
        // The follower a little larger and brighter (the dress), the leader darker.
        let (r, tint) = if k % 2 == 0 { (radius, 0.75) } else { (radius * 1.15, 1.0) };
        u.dancers[k] = dancers[k].extend(r).extend(glow + note_glow * note(couple));
        u.colours[k] = (colour(couple, saturation) * tint).extend(flare);
        for i in 0..TRAIL {
            u.trail[k * TRAIL + i] = state.trail[k][i].extend(0.0).extend(0.0);
        }
    }
    u.frame = Vec4::new(size.x / 2.0, size.y / 2.0, c(Control::HouseLights), c(Control::Trail));
    u.look = Vec4::new(tuning.get(P::Ground).round(), time.elapsed_secs(), c(Control::DiscoBall), hold);
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
    fn the_turn_settles_on_the_beat() {
        assert!((stepped(3.0) - 3.0).abs() < 1e-5);
        assert!((stepped(2.5) - 2.5).abs() < 1e-5);
        // Barely moving just after and just before the beat.
        assert!(stepped(3.1) - 3.0 < 0.02 && 4.0 - stepped(3.9) < 0.02);
    }

    #[test]
    fn every_figure_stays_on_the_floor_and_apart() {
        let free: [Vec2; COUPLES] = Dance::default().free;
        for figure in Figure::ALL {
            for travelled in [0.0, 0.7, 5.0] {
                for count in [0.0, 1.0, 2.0, 3.0] {
                    let places: Vec<Vec2> = (0..COUPLES).map(|i| figure.place(i, travelled, 0.1, count, free[i])).collect();
                    for (i, p) in places.iter().enumerate() {
                        assert!(p.x.abs() <= 0.9 && p.y.abs() <= 0.9, "{figure:?}: couple {i} at {p}");
                        for q in &places[i + 1..] {
                            assert!(p.distance(*q) > 0.04, "{figure:?}: couples on top of each other at {p}");
                        }
                    }
                }
            }
        }
    }

    /// At 120 BPM for a few bars: the couples stay on the floor, keep their
    /// hold, turn as much as asked, travel, and change figure on the 1.
    #[test]
    fn they_dance_on_the_beat_and_change_figure() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut dance = Dance::default();
        let size = Vec2::new(1920.0, 1080.0);
        let (dt, bps, hold) = (1.0 / 120.0, 2.0, 26.0);
        let turn = 90f32.to_radians();
        let lead = |beats: f32| Lead { travel: 0.2, turn, hold, spread: 1.0, scatter: 0.0, count: beats % 16.0, counts: 16.0 };
        let mut start = dance.centre;
        for frame in 0..(40.0 / dt) as usize {
            let beats = frame as f32 * dt * bps;
            dance.step(lead(beats), size, dt, &mut rng);
            if frame == 0 {
                assert_eq!(dance.figure, 0);
                start = dance.centre;
            }
            let d = dance.dancers(hold);
            for (k, p) in d.iter().enumerate() {
                assert!(p.x.abs() < size.x / 2.0 && p.y.abs() < size.y / 2.0, "dancer {k} off the floor at {p}");
            }
            assert!((d[0].distance(d[1]) - hold).abs() < 1e-3);
        }
        // Just short of 80 beats: the fifth figure of 16 counts, wandering free.
        assert_eq!(Figure::ALL[dance.figure], Figure::Free);
        // A quarter turn per beat, clockwise, for 80 beats.
        let turned = Dance::default().turned[0] - dance.turned[0];
        assert!((turned - 80.0 * turn).abs() < 0.3, "turned {turned}");
        assert!(dance.travelled > 1.0);
        assert!(start.iter().zip(&dance.centre).any(|(a, b)| a.distance(*b) > 50.0));
    }
}
