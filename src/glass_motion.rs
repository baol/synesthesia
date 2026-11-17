//! How the glass window moves: each pane centre hangs on a damped spring from
//! its rest point, and the music moves the point the spring pulls towards.
//!
//! - **dance step**: every pane dances the salsa basic step "on 1", all on
//!   the same count, each along its own line: forward on 1, rock back a little
//!   on 2, together on 3, hold on 4; back on 5, rock forward on 6, together on
//!   7, hold on 8. **Dancers turn** slowly turns each pane's line (half of
//!   them each way), so over a phrase the steps wander round;
//! - **breath**: the whole window swells out from its centre (the edge panes
//!   most, the middle not at all) and settles back;
//! - **ripple**: each rise of the control sends a ring out from a random point,
//!   pushing the panes outward as its front passes;
//! - **sway**: the lower panes swing slowly from side to side;
//! - **havoc**: every pane jerks a new random way a dozen times a second (by
//!   default when rap bursts in after quiet), shaking the window to pieces
//!   within the panes' reach. The jerks go on top of the springs, which
//!   would smooth them away.
//!
//! All in pane spacings. The springs give the motion its weight: a pane
//! overshoots a little and settles. A centre never strays more than
//! [`MAX_OFFSET`] spacings from home, so the panes keep their places and the
//! shader's pane grid stays exact.

use bevy::prelude::*;
use rand::Rng;

use crate::{
    field::soft_limit,
    glass::{Control, P, Panes},
    scene::Scene,
    tuning::{MusicClock, Transforms, Tuning},
};

/// How far a pane centre can stray from home, in spacings. The pane grid is
/// tested up to this (see `Grid` in glass.rs).
pub const MAX_OFFSET: f32 = 0.35;
/// Targets are softly limited a bit inside that, so the springs' overshoot
/// has room.
const TARGET_LIMIT: f32 = 0.3;
/// Ripple fronts travel this many screen widths per second…
const RIPPLE_SPEED: f32 = 0.45;
/// …are this many spacings wide…
const RIPPLE_WIDTH: f32 = 1.3;
/// …and fade with this time constant (s).
const RIPPLE_FADE: f32 = 1.3;
/// Rises of the ripple control smaller than this don't start one.
const RIPPLE_THRESHOLD: f32 = 0.24;
const MAX_RIPPLES: usize = 8;
/// Sway period (s).
const SWAY_PERIOD: f32 = 4.0;

/// The salsa basic step: where the dancer is after each of the 8 counts
/// (+1 a full step forward along their line, −1 back).
const SALSA: [f32; 8] = [1.0, 0.35, 0.0, 0.0, -1.0, -0.35, 0.0, 0.0];
/// A step takes this share of a beat; then the dancer holds until the next count.
const STEP_TIME: f32 = 0.4;

/// Position along the dancer's line at `count` (0..8, 0 = on the 1).
pub fn salsa(count: f32) -> f32 {
    let k = (count.floor() as usize) % 8;
    let from = SALSA[(k + 7) % 8];
    let t = ((count - count.floor()) / STEP_TIME).clamp(0.0, 1.0);
    let ease = t * t * (3.0 - 2.0 * t);
    from + (SALSA[k] - from) * ease
}

/// A ring spreading over the window.
#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    pub origin: Vec2,
    pub age: f32,
    /// Peak push, in spacings.
    pub strength: f32,
}

impl Ripple {
    pub fn radius(&self, screen_w: f32) -> f32 {
        self.age * RIPPLE_SPEED * screen_w
    }

    /// Outward push at `p` (px), for panes `spacing` apart.
    fn push(&self, p: Vec2, spacing: f32, screen_w: f32) -> Vec2 {
        let d = p - self.origin;
        let dist = d.length();
        if dist < 1e-3 {
            return Vec2::ZERO;
        }
        let x = (dist - self.radius(screen_w)) / (RIPPLE_WIDTH * spacing);
        d / dist * self.strength * spacing * (-x * x).exp() * (-self.age / RIPPLE_FADE).exp()
    }
}

/// Finds the top of each rise of a value, and how much it rose.
#[derive(Default)]
pub struct Rises {
    prev: f32,
    from: Option<f32>,
}

impl Rises {
    /// Feed the next value; returns the height of a rise that just ended.
    pub fn step(&mut self, value: f32) -> Option<f32> {
        let mut done = None;
        if value > self.prev {
            let prev = self.prev;
            self.from.get_or_insert(prev);
        } else if let Some(from) = self.from.take() {
            done = Some(self.prev - from);
        }
        self.prev = value;
        done
    }
}

/// The window's moving state.
#[derive(Resource, Default)]
pub struct Motion {
    vel: Vec<Vec2>,
    /// How far the dancers have turned (turns).
    turned: f32,
    sway_phase: f32,
    pub ripples: Vec<Ripple>,
    ripple_rises: Rises,
    /// Ripples started so far.
    pub started: usize,
    /// Where the springs hold each pane (the panes are drawn there plus the shake).
    pos: Vec<Vec2>,
    /// Each pane's shake: where it is jerking to (redrawn a dozen times a
    /// second), and where it is now.
    shake: Vec<Vec2>,
    jerk: Vec<Vec2>,
    shake_clock: f32,
}

/// How often the shake directions change (per second).
const SHAKE_RATE: f32 = 12.0;

pub struct GlassMotionPlugin;

impl Plugin for GlassMotionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Motion>()
            .add_systems(OnEnter(Scene::Glass), |mut m: ResMut<Motion>| *m = Motion::default())
            .add_systems(
                Update,
                move_panes.after(crate::tuning::SignalSet).run_if(in_state(Scene::Glass)),
            );
    }
}

/// The music's values for one step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    /// Dance step length and how fast the dancers turn (turns per minute).
    pub step: f32,
    pub turn: f32,
    /// Where we are in the 8-count phrase (0..8, 0 = on the 1).
    pub count: f32,
    pub breath: f32,
    /// How hard the panes shake (havoc), in spacings.
    pub havoc: f32,
    pub ripple: f32,
    pub sway: f32,
}

impl Drive {
    pub fn from_tuning(tuning: &Tuning, transforms: &Transforms, clock: &MusicClock) -> Self {
        let c = |control| tuning.control(control, transforms);
        Self {
            step: c(Control::DanceStep),
            turn: c(Control::DanceTurn),
            count: clock.count(8),
            breath: c(Control::Breath),
            ripple: c(Control::Ripple),
            havoc: c(Control::Havoc),
            sway: c(Control::Sway),
        }
    }
}

/// The springs' (stiffness Hz, damping ratio).
pub fn spring(tuning: &Tuning) -> (f32, f32) {
    (tuning.get(P::Stiffness), tuning.get(P::Damping))
}

pub fn move_panes(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    clock: Res<MusicClock>,
    screen: Single<&Window>,
    mut panes: ResMut<Panes>,
    mut motion: ResMut<Motion>,
) {
    let drive = Drive::from_tuning(&tuning, &transforms, &clock);
    let spring = spring(&tuning);
    let dt = time.delta_secs().min(0.05);
    motion.step(&mut panes, drive, spring, screen.size(), dt, &mut rand::rng());
}

impl Motion {
    /// Advance the panes by `dt` seconds. `spring` is (stiffness Hz, damping ratio).
    pub fn step(&mut self, panes: &mut Panes, drive: Drive, spring: (f32, f32), screen: Vec2, dt: f32, rng: &mut impl Rng) {
        let n = panes.seeds.len();
        if self.vel.len() != n {
            self.vel = vec![Vec2::ZERO; n];
            self.pos = panes.homes.clone();
            self.shake = vec![Vec2::ZERO; n];
            self.jerk = vec![Vec2::ZERO; n];
        }
        // Havoc: every pane jerks a new way, a dozen times a second.
        self.shake_clock += dt * SHAKE_RATE;
        if self.shake_clock >= 1.0 && drive.havoc > 0.0 {
            self.shake_clock = self.shake_clock.fract();
            for s in &mut self.shake {
                *s = Vec2::from_angle(rng.random_range(0.0..std::f32::consts::TAU)) * rng.random_range(0.5..1.0);
            }
        }
        let a = panes.spacing;
        let half = screen / 2.0;

        // Ripples: start one at the top of each rise, as strong as it rose.
        if let Some(rise) = self.ripple_rises.step(drive.ripple)
            && rise > RIPPLE_THRESHOLD
        {
            if self.ripples.len() == MAX_RIPPLES {
                self.ripples.remove(0);
            }
            let origin = Vec2::new(rng.random_range(-0.8..0.8) * half.x, rng.random_range(-0.8..0.8) * half.y);
            self.ripples.push(Ripple { origin, age: 0.0, strength: rise });
            self.started += 1;
        }
        for r in &mut self.ripples {
            r.age += dt;
        }
        self.ripples.retain(|r| r.age < 5.0 * RIPPLE_FADE);

        self.turned += dt * drive.turn / 60.0;
        let step = salsa(drive.count) * drive.step * a;
        self.sway_phase = (self.sway_phase + dt / SWAY_PERIOD).fract();
        let sway = (self.sway_phase * std::f32::consts::TAU).sin() * drive.sway * a;

        let w = std::f32::consts::TAU * spring.0;
        let zeta = spring.1;
        let steps = 4;
        let h = dt / steps as f32;
        for i in 0..n {
            let home = panes.homes[i];
            // Each dancer's line: its own direction, turning (half each way).
            let way = if i % 2 == 0 { 1.0 } else { -1.0 };
            let facing = Vec2::from_angle((panes.grain[i] + way * self.turned) * std::f32::consts::TAU);
            let mut offset = facing * step;
            // Breath: out from the centre, in proportion to the distance from it.
            offset += home / half.max_element() * drive.breath * a;
            // Sway: the lower panes most (0 at the top edge, 1 at the bottom).
            let low = (0.5 - home.y / screen.y).clamp(0.0, 1.0);
            offset.x += sway * low * low;
            for r in &self.ripples {
                offset += r.push(home, a, screen.x);
            }
            let target = home + soft_limit(offset, TARGET_LIMIT * a);

            let (mut p, mut v) = (self.pos[i], self.vel[i]);
            for _ in 0..steps {
                let acc = w * w * (target - p) - 2.0 * zeta * w * v;
                v += acc * h;
                p += v * h;
            }
            // Never further than the grid allows.
            let off = p - home;
            if off.length() > MAX_OFFSET * a {
                p = home + off.normalize() * MAX_OFFSET * a;
                v = v.reject_from(off);
            }
            self.pos[i] = p;
            self.vel[i] = v;
            // Havoc goes on top of the springs (they would smooth it away):
            // quick jerks, eased over ~30 ms so they read as motion.
            self.jerk[i] = self.jerk[i].lerp(self.shake[i] * drive.havoc * a, 1.0 - (-dt / 0.03).exp());
            let shown = p + self.jerk[i] - home;
            panes.seeds[i] = home + shown.clamp_length_max(MAX_OFFSET * a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn panes_stay_within_reach_whatever_the_music() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut panes = Panes::layout(90, screen);
        let mut motion = Motion::default();
        let mut rng = StdRng::seed_from_u64(3);
        let max = 2.0; // well past every control's range
        for step in 0..3000 {
            // Slam everything up and down every few frames.
            let on = (step / 7) % 2 == 0;
            let v = if on { max } else { 0.0 };
            let drive = Drive { step: v, turn: 8.0, count: (step as f32 * 0.03) % 8.0, breath: v, ripple: v, sway: v, havoc: v };
            motion.step(&mut panes, drive, (6.0, 0.05), screen, 1.0 / 120.0, &mut rng);
            for (s, h) in panes.seeds.iter().zip(&panes.homes) {
                assert!(s.distance(*h) <= MAX_OFFSET * panes.spacing + 1e-3, "pane strayed {}", s.distance(*h) / panes.spacing);
            }
        }
    }

    #[test]
    fn the_salsa_goes_forward_on_one_and_back_on_five() {
        assert!((salsa(0.5) - 1.0).abs() < 1e-6, "forward after the 1");
        assert!(salsa(1.5) > 0.2 && salsa(1.5) < 0.5, "rock back on 2");
        assert!(salsa(2.5).abs() < 1e-6 && salsa(3.5).abs() < 1e-6, "together on 3, hold on 4");
        assert!((salsa(4.5) + 1.0).abs() < 1e-6, "back after the 5");
        assert!(salsa(7.9).abs() < 1e-6);
        // Continuous across counts (no jumps).
        for k in 0..800 {
            let c = k as f32 * 0.01;
            assert!((salsa(c + 0.01) - salsa(c)).abs() < 0.1, "jump at {c}");
        }
    }

    #[test]
    fn dancers_step_along_their_line_in_time() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut panes = Panes::layout(90, screen);
        let mut motion = Motion::default();
        let mut rng = StdRng::seed_from_u64(6);
        let dt = 1.0 / 120.0;
        let mut count = 0.0f32;
        let along = |panes: &Panes| {
            let facing = Vec2::from_angle(panes.grain[0] * std::f32::consts::TAU);
            (panes.seeds[0] - panes.homes[0]).dot(facing) / panes.spacing
        };
        let (mut fw, mut bw) = (0.0f32, 0.0f32);
        for _ in 0..(120 * 8) {
            count = (count + dt * 2.0) % 8.0; // 120 BPM
            let drive = Drive { step: 0.2, turn: 0.0, count, ..default() };
            motion.step(&mut panes, drive, (1.5, 0.5), screen, dt, &mut rng);
            if (1.5..2.0).contains(&count) {
                fw = fw.max(along(&panes));
            }
            if (5.5..6.0).contains(&count) {
                bw = bw.min(along(&panes));
            }
        }
        assert!(fw > 0.1 && bw < -0.1, "forward {fw}, back {bw}");
    }

    #[test]
    fn havoc_shakes_the_panes_hard_and_fast() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut panes = Panes::layout(90, screen);
        let mut motion = Motion::default();
        let mut rng = StdRng::seed_from_u64(7);
        let mut moved = 0.0f32;
        let mut prev = panes.seeds.clone();
        for k in 0..120 {
            motion.step(&mut panes, Drive { havoc: 0.3, ..default() }, (1.5, 0.5), screen, 1.0 / 120.0, &mut rng);
            if k > 30 {
                // Mean speed of the panes, in spacings per second.
                moved += panes.seeds.iter().zip(&prev).map(|(a, b)| a.distance(*b)).sum::<f32>() / panes.seeds.len() as f32 / panes.spacing * 120.0;
            }
            prev = panes.seeds.clone();
        }
        let speed = moved / 89.0;
        assert!(speed > 2.0, "panes move {speed} spacings/s under havoc");
    }

    #[test]
    fn still_music_leaves_the_window_at_rest() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut panes = Panes::layout(90, screen);
        let mut motion = Motion::default();
        let mut rng = StdRng::seed_from_u64(4);
        for _ in 0..600 {
            motion.step(&mut panes, Drive::default(), (1.5, 0.5), screen, 1.0 / 120.0, &mut rng);
        }
        for (s, h) in panes.seeds.iter().zip(&panes.homes) {
            assert!(s.distance(*h) < 1e-3);
        }
    }

    #[test]
    fn a_rise_of_the_ripple_control_starts_one_ripple_at_its_top() {
        let screen = Vec2::new(1920.0, 1080.0);
        let mut panes = Panes::layout(90, screen);
        let mut motion = Motion::default();
        let mut rng = StdRng::seed_from_u64(5);
        for v in [0.0, 0.1, 0.25, 0.4, 0.38, 0.3, 0.2] {
            let drive = Drive { ripple: v, ..default() };
            motion.step(&mut panes, drive, (1.5, 0.5), screen, 1.0 / 120.0, &mut rng);
        }
        assert_eq!(motion.ripples.len(), 1);
        assert!((motion.ripples[0].strength - 0.4).abs() < 1e-6);
    }
}
