//! Sand pattern: the screen as a round vibrating plate, the bubbles as sand.
//!
//! A circular plate vibrates in modes `u(r, θ) = J_m(j_mn · r/R) · cos(m(θ − θ₀))`
//! (rings and spokes; `J_m` is a Bessel function, `j_mn` its n-th zero). Sand
//! is thrown off the parts that move and collects on the *nodal lines*, which
//! stay still: the Chladni figures. Here every bubble is pulled down the slope
//! of `u²`, towards the nearest nodal ring or spoke, while it is shaken.
//!
//! - **Which pattern**: the plate's vibration-mode control, 0 = simplest
//!   (a few rings) … 1 = most complex. By default it follows the music's
//!   dominant pitch. When it points at a new mode for a while, the old pattern
//!   fades into the new one, so nothing jumps.
//! - **How hard**: the vibration-amplitude control (by default the groove and
//!   the attacks: rhythmic, like sand dancing on a hi-hat).
//! - Bubbles on the moving parts also bounce a little bigger (drawing only).

use bevy::prelude::*;

use crate::{
    features::follow,
    field::FlowField,
    bubble_controls::{Control, P},
    tuning::{Transforms, Tuning},
};

/// Bessel functions J_0..J_7 tabulated on [0, X_MAX].
const ORDERS: usize = 8;
const X_MAX: f32 = 64.0;
const STEPS_PER_UNIT: f32 = 32.0;
const SAMPLES: usize = (X_MAX * STEPS_PER_UNIT) as usize + 2;

/// Highest angular order used in patterns (spokes = 2 × m).
const MAX_M: usize = 4;
/// Number of patterns, simplest first.
const MODES: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    /// Angular order: m nodal diameters.
    pub m: usize,
    /// Radial order: n-th zero, so n nodal circles out to R.
    pub n: usize,
    /// `j_mn`, the n-th positive zero of J_m.
    pub zero: f32,
}

struct Bessel {
    j: Vec<[f32; SAMPLES]>,
}

impl Bessel {
    fn new() -> Self {
        // J_m(x) = (1/π) ∫₀^π cos(mτ − x sin τ) dτ, by the midpoint rule.
        const N: usize = 600;
        let j = (0..ORDERS)
            .map(|m| {
                let mut t = [0.0f32; SAMPLES];
                for (i, v) in t.iter_mut().enumerate() {
                    let x = i as f64 / STEPS_PER_UNIT as f64;
                    let mut sum = 0.0f64;
                    for k in 0..N {
                        let tau = (k as f64 + 0.5) * std::f64::consts::PI / N as f64;
                        sum += (m as f64 * tau - x * tau.sin()).cos();
                    }
                    *v = (sum / N as f64) as f32;
                }
                t
            })
            .collect();
        Self { j }
    }

    /// J_m(x), linearly interpolated (0 beyond the table).
    #[inline]
    fn get(&self, m: usize, x: f32) -> f32 {
        let f = x * STEPS_PER_UNIT;
        let i = f as usize;
        if i + 1 >= SAMPLES {
            return 0.0;
        }
        let a = f - i as f32;
        self.j[m][i] * (1.0 - a) + self.j[m][i + 1] * a
    }

    /// J_m'(x) = (J_{m−1} − J_{m+1}) / 2, and J_0' = −J_1.
    #[inline]
    fn deriv(&self, m: usize, x: f32) -> f32 {
        if m == 0 { -self.get(1, x) } else { 0.5 * (self.get(m - 1, x) - self.get(m + 1, x)) }
    }

    /// Positive zeros of J_m, in order.
    fn zeros(&self, m: usize, count: usize) -> Vec<f32> {
        let t = &self.j[m];
        let mut out = Vec::new();
        for i in 1..SAMPLES - 1 {
            if out.len() == count {
                break;
            }
            let (a, b) = (t[i], t[i + 1]);
            if a != 0.0 && (a < 0.0) != (b < 0.0) {
                out.push((i as f32 + a / (a - b)) / STEPS_PER_UNIT);
            }
        }
        out
    }
}

/// Plate modes ordered from simple to complex (by frequency, ∝ j_mn).
fn modes(bessel: &Bessel) -> Vec<Mode> {
    let mut all = Vec::new();
    for m in 0..=MAX_M {
        for (k, zero) in bessel.zeros(m, 6).into_iter().enumerate() {
            // Skip the bare single ring of order 0 (no inner structure).
            if m == 0 && k == 0 {
                continue;
            }
            all.push(Mode { m, n: k + 1, zero });
        }
    }
    all.sort_by(|a, b| a.zero.total_cmp(&b.zero));
    all.truncate(MODES);
    all
}

#[derive(Resource)]
pub struct Sand {
    bessel: Bessel,
    pub modes: Vec<Mode>,
    /// Current and previous pattern, and how far the fade to the current one is (0..1).
    pub current: usize,
    previous: usize,
    blend: f32,
    /// The pattern the music points at, and how long it has done so.
    wanted: usize,
    wanted_for: f32,
    /// Orientation of the spokes (drifts very slowly).
    angle: f32,
    /// Plate radius in px.
    radius: f32,
    /// Current shaking, 0..~1.5.
    pub amp: f32,
    /// Peak pull towards the lines, px/s².
    accel: f32,
    swell: f32,
}

impl Default for Sand {
    fn default() -> Self {
        let bessel = Bessel::new();
        let modes = modes(&bessel);
        Self {
            bessel,
            modes,
            current: 0,
            previous: 0,
            blend: 1.0,
            wanted: 0,
            wanted_for: 0.0,
            angle: 0.4,
            radius: 800.0,
            amp: 0.0,
            accel: 0.0,
            swell: 0.0,
        }
    }
}

impl Sand {
    /// Displacement u and its gradient (per px) of mode `i` at `p`.
    #[inline]
    fn mode_at(&self, i: usize, p: Vec2) -> (f32, Vec2) {
        let mode = self.modes[i];
        let k = mode.zero / self.radius;
        let r = p.length();
        if r < 1e-3 {
            // At the centre only m = 0 moves, and its slope is 0.
            return (if mode.m == 0 { self.bessel.get(0, 0.0) } else { 0.0 }, Vec2::ZERO);
        }
        let radial = p / r;
        // cos(mφ), sin(mφ) for φ = θ − angle, by repeated rotation.
        let base = Vec2::from_angle(-self.angle).rotate(radial);
        let mut c = Vec2::X;
        for _ in 0..mode.m {
            c = c.rotate(base);
        }
        let x = k * r;
        let jm = self.bessel.get(mode.m, x);
        let u = jm * c.x;
        let du_dr = k * self.bessel.deriv(mode.m, x) * c.x;
        let du_dphi_over_r = -(mode.m as f32) * jm * c.y / r;
        let tangential = radial.perp();
        (u, radial * du_dr + tangential * du_dphi_over_r)
    }

    /// The blended pattern at `p`.
    #[inline]
    fn surface(&self, p: Vec2) -> (f32, Vec2) {
        let (u1, g1) = self.mode_at(self.current, p);
        if self.blend >= 1.0 {
            return (u1, g1);
        }
        let (u0, g0) = self.mode_at(self.previous, p);
        let a = self.blend * self.blend * (3.0 - 2.0 * self.blend);
        (u0 + (u1 - u0) * a, g0 + (g1 - g0) * a)
    }

    /// Pull (px/s²) on a bubble of base size at `p`: down the slope of u²,
    /// towards the nodal lines. Heavier bubbles divide by their mass.
    #[inline]
    pub fn force(&self, p: Vec2) -> Vec2 {
        if self.amp * self.accel < 0.5 {
            return Vec2::ZERO;
        }
        let (u, grad) = self.surface(p);
        let k = self.modes[self.current].zero / self.radius;
        // Bessel waves shrink away from the centre (amplitude ≈ √(2/πx)), so
        // divide that out: the pull is as strong at the edge as in the middle.
        // Then u·∇u peaks at about k/2; normalise so `accel` is the peak pull.
        let env = (2.0 / (std::f32::consts::PI * (k * p.length()).max(0.1))).min(1.0);
        -u * grad / (env * 0.5 * k) * self.accel * self.amp
    }

    /// Drawing-only bounce: bubbles on the moving parts look a little bigger.
    #[inline]
    pub fn swell_at(&self, p: Vec2) -> f32 {
        if self.amp * self.swell < 1e-3 {
            return 1.0;
        }
        let (u, _) = self.surface(p);
        let k = self.modes[self.current].zero / self.radius;
        let env = (2.0 / (std::f32::consts::PI * (k * p.length()).max(0.1))).min(1.0).sqrt();
        1.0 + self.swell * self.amp.min(1.0) * (u.abs() / env).min(1.0)
    }
}

pub struct SandPlugin;

impl Plugin for SandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sand>()
            .add_systems(Update, update_sand.after(crate::tuning::SignalSet).run_if(in_state(crate::scene::Scene::Bubbles)));
    }
}

pub fn update_sand(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    field: Res<FlowField>,
    window: Single<&Window>,
    mut sand: ResMut<Sand>,
) {
    let dt = time.delta_secs().min(0.05);
    sand.radius = 0.5 * window.size().length() * tuning.get(P::PlateSize);
    sand.angle += dt * 0.01;

    // --- Which pattern: the mode control, held for a while before switching.
    let mode = tuning.control(Control::PlateMode, &transforms);
    let wanted = ((mode * (sand.modes.len() - 1) as f32).round() as usize).min(sand.modes.len() - 1);
    if wanted != sand.wanted {
        sand.wanted = wanted;
        sand.wanted_for = 0.0;
    }
    sand.wanted_for += dt;
    let change = tuning.get(P::PlateModeChange);
    // Switch once the control has pointed at a new mode for a while, and the
    // last fade has finished.
    if sand.wanted != sand.current && sand.wanted_for > 0.5 * change && sand.blend >= 1.0 {
        sand.previous = sand.current;
        sand.current = sand.wanted;
        sand.blend = 0.0;
    }
    sand.blend = (sand.blend + dt / change).min(1.0);

    // --- How hard: the vibration control (already shaped by its transforms;
    // a very short smoothing only keeps slider moves from jumping).
    let target = tuning.control(Control::PlateVibration, &transforms);
    sand.amp = follow(sand.amp, target, dt, 0.02, 0.05);

    // At full vibration, a bubble drifts towards the lines at `pull` × the wind
    // speed (drag turns the pull into a speed of pull × drag time).
    let wind = field.wind.length().max(1.0);
    let pull = tuning.get(P::PlatePull);
    sand.accel = pull * wind / tuning.get(P::DragTime).max(0.03);
    sand.swell = if pull > 0.0 { tuning.get(P::PlateBounce) } else { 0.0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bessel_table_is_accurate() {
        let b = Bessel::new();
        assert!((b.get(0, 0.0) - 1.0).abs() < 1e-4);
        assert!((b.get(1, 1.0) - 0.440_051).abs() < 1e-3);
        // Known zeros: j_01 = 2.4048, j_11 = 3.8317, j_21 = 5.1356.
        assert!((b.zeros(0, 1)[0] - 2.4048).abs() < 2e-3);
        assert!((b.zeros(1, 1)[0] - 3.8317).abs() < 2e-3);
        assert!((b.zeros(2, 1)[0] - 5.1356).abs() < 2e-3);
    }

    #[test]
    fn modes_go_from_simple_to_complex() {
        let s = Sand::default();
        assert_eq!(s.modes.len(), MODES);
        assert!(s.modes.windows(2).all(|w| w[0].zero <= w[1].zero));
        // Plenty of pure ring patterns, as wanted.
        assert!(s.modes.iter().filter(|m| m.m == 0).count() >= 3);
    }

    #[test]
    fn pull_points_towards_the_nodal_lines() {
        let mut s = Sand::default();
        s.amp = 1.0;
        s.accel = 100.0;
        s.radius = 800.0;
        for i in 0..s.modes.len() {
            s.current = i;
            for p in [Vec2::new(130.0, 40.0), Vec2::new(-260.0, 310.0), Vec2::new(500.0, -90.0)] {
                let f = s.force(p);
                if f.length() < 1e-3 {
                    continue;
                }
                // A small step along the pull lowers u² (moves towards a node).
                let (u, _) = s.surface(p);
                let (u2, _) = s.surface(p + f.normalize() * 0.5);
                assert!(u2 * u2 <= u * u + 1e-6, "mode {i} at {p}: {u} -> {u2}");
            }
        }
    }
}
