//! The flow field that carries the bubbles:
//!
//! ```text
//! u(x) = wind  +  limit( Σ swirlᵢ(x) )  +  turbulence(x, t)
//! ```
//!
//! Every term is divergence-free (uniform wind, Lamb–Oseen vortices, curl
//! noise), so the flow itself neither bunches nor thins the bubbles. Visible
//! gaps in swirl cores come from the bubbles' inertia, which is the effect we
//! want. The soft limit keeps the swirls from overpowering the wind, so
//! bubbles always keep moving left / up overall.

use bevy::prelude::*;

/// One swirl: a Lamb–Oseen vortex (smooth core, no singularity, decays with distance).
#[derive(Clone, Copy, Debug)]
pub struct Vortex {
    pub pos: Vec2,
    /// Core radius in pixels: the swirl is fastest at about 1.12 × core.
    pub core: f32,
    /// Peak tangential speed in px/s (signed: positive = counter-clockwise).
    pub peak_speed: f32,
}

impl Vortex {
    /// u_θ(r) = Γ / (2πr) · (1 − e^(−r²/rc²)); its peak is ≈ 0.638·Γ/(2π·rc) at r ≈ 1.121·rc.
    #[inline]
    pub fn velocity(&self, p: Vec2) -> Vec2 {
        let d = p - self.pos;
        let r2 = d.length_squared();
        let rc2 = self.core * self.core;
        // Beyond ~6 cores the contribution is negligible and the field stays cheap.
        if r2 > 36.0 * rc2 {
            return Vec2::ZERO;
        }
        // Γ/(2π) expressed through the peak speed.
        let k = self.peak_speed * self.core / 0.638;
        let factor = if r2 < 1e-6 {
            // Limit as r → 0: u_θ / r → Γ/(2π rc²).
            k / rc2
        } else {
            k * (1.0 - (-r2 / rc2).exp()) / r2
        };
        // Tangential direction: perp(d)/r, and u_θ/r already folded into factor.
        d.perp() * factor
    }
}

/// Everything the bubble simulation needs to evaluate the flow. Small and `Copy`,
/// so the parallel bubble update can read it without locking.
#[derive(Resource, Clone, Debug)]
pub struct FlowField {
    pub wind: Vec2,
    pub vortices: [Vortex; 4],
    /// Upper bound on the summed swirl speed, px/s (soft, via tanh).
    pub swirl_limit: f32,
    /// Turbulence speed, px/s.
    pub turbulence: f32,
    /// Turbulence feature size, px.
    pub turbulence_scale: f32,
    /// Turbulence time, advances with the wind so eddies travel with the flow.
    pub turbulence_offset: Vec2,
    pub turbulence_morph: f32,
}

impl Default for FlowField {
    fn default() -> Self {
        Self {
            wind: Vec2::new(-140.0, 50.0),
            vortices: [Vortex {
                pos: Vec2::ZERO,
                core: 100.0,
                peak_speed: 0.0,
            }; 4],
            swirl_limit: 200.0,
            turbulence: 20.0,
            turbulence_scale: 320.0,
            turbulence_offset: Vec2::ZERO,
            turbulence_morph: 0.0,
        }
    }
}

impl FlowField {
    #[inline]
    pub fn velocity(&self, p: Vec2) -> Vec2 {
        self.wind + self.swirl(p) + self.turbulence_at(p)
    }

    #[inline]
    pub fn swirl(&self, p: Vec2) -> Vec2 {
        let mut s = Vec2::ZERO;
        for v in &self.vortices {
            s += v.velocity(p);
        }
        soft_limit(s, self.swirl_limit)
    }

    #[inline]
    pub fn turbulence_at(&self, p: Vec2) -> Vec2 {
        if self.turbulence <= 0.0 {
            return Vec2::ZERO;
        }
        let q = (p - self.turbulence_offset) / self.turbulence_scale;
        curl_noise(q, self.turbulence_morph) * self.turbulence
    }
}

/// Scale `v` so its length approaches `limit` smoothly instead of exceeding it.
#[inline]
pub fn soft_limit(v: Vec2, limit: f32) -> Vec2 {
    let len = v.length();
    if len < 1e-6 || limit <= 0.0 {
        return Vec2::ZERO;
    }
    v * (limit * (len / limit).tanh() / len)
}

// ---------------------------------------------------------------------------
// Noise

#[inline]
fn hash2(i: i32, j: i32) -> u32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

/// 16 evenly spaced unit directions (k · 22.5°).
const DIRS: [Vec2; 16] = [
    Vec2::new(1.0, 0.0),
    Vec2::new(0.92388, 0.38268),
    Vec2::new(0.70711, 0.70711),
    Vec2::new(0.38268, 0.92388),
    Vec2::new(0.0, 1.0),
    Vec2::new(-0.38268, 0.92388),
    Vec2::new(-0.70711, 0.70711),
    Vec2::new(-0.92388, 0.38268),
    Vec2::new(-1.0, 0.0),
    Vec2::new(-0.92388, -0.38268),
    Vec2::new(-0.70711, -0.70711),
    Vec2::new(-0.38268, -0.92388),
    Vec2::new(0.0, -1.0),
    Vec2::new(0.38268, -0.92388),
    Vec2::new(0.70711, -0.70711),
    Vec2::new(0.92388, -0.38268),
];

#[inline]
fn gradient(i: i32, j: i32) -> Vec2 {
    DIRS[(hash2(i, j) & 15) as usize]
}

/// 2D Perlin gradient noise with its analytic gradient. Returns (value, ∂/∂x, ∂/∂y);
/// value is roughly in [-0.7, 0.7].
#[inline]
pub fn perlin_grad(p: Vec2) -> (f32, Vec2) {
    let i = p.floor();
    let f = p - i;
    let (ix, iy) = (i.x as i32, i.y as i32);

    // Quintic fade and its derivative.
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);

    let ga = gradient(ix, iy);
    let gb = gradient(ix + 1, iy);
    let gc = gradient(ix, iy + 1);
    let gd = gradient(ix + 1, iy + 1);

    let va = ga.dot(f);
    let vb = gb.dot(f - Vec2::new(1.0, 0.0));
    let vc = gc.dot(f - Vec2::new(0.0, 1.0));
    let vd = gd.dot(f - Vec2::new(1.0, 1.0));

    let k1 = vb - va;
    let k2 = vc - va;
    let k3 = va - vb - vc + vd;
    let value = va + u.x * k1 + u.y * k2 + u.x * u.y * k3;

    let grad = ga
        + u.x * (gb - ga)
        + u.y * (gc - ga)
        + u.x * u.y * (ga - gb - gc + gd)
        + du * Vec2::new(k1 + u.y * k3, k2 + u.x * k3);
    (value, grad)
}

/// Divergence-free 2D noise: the curl of a scalar potential ψ, u = (∂ψ/∂y, −∂ψ/∂x).
/// Two octaves; `morph` slides the second octave so the pattern evolves in place.
/// Output length is typically 0..~2.
#[inline]
pub fn curl_noise(q: Vec2, morph: f32) -> Vec2 {
    let (_, g1) = perlin_grad(q);
    let (_, g2) = perlin_grad(q * 2.03 + Vec2::new(17.3 + morph, -9.1 - 0.7 * morph));
    let g = g1 + 0.5 * 2.03 * g2;
    Vec2::new(g.y, -g.x)
}

/// Smooth 1D noise in roughly [-0.7, 0.7], for wandering paths.
#[inline]
pub fn noise1(t: f32, channel: f32) -> f32 {
    perlin_grad(Vec2::new(t, channel * 13.37)).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perlin_gradient_matches_finite_difference() {
        let eps = 1e-3;
        for k in 0..50 {
            let p = Vec2::new(k as f32 * 0.731 - 7.0, k as f32 * 0.377 + 3.0);
            let (_, g) = perlin_grad(p);
            let dx = (perlin_grad(p + Vec2::X * eps).0 - perlin_grad(p - Vec2::X * eps).0) / (2.0 * eps);
            let dy = (perlin_grad(p + Vec2::Y * eps).0 - perlin_grad(p - Vec2::Y * eps).0) / (2.0 * eps);
            assert!((g.x - dx).abs() < 1e-2 && (g.y - dy).abs() < 1e-2, "at {p}: {g} vs ({dx}, {dy})");
        }
    }

    #[test]
    fn curl_noise_is_divergence_free() {
        let eps = 1e-3;
        for k in 0..50 {
            let q = Vec2::new(k as f32 * 0.53 - 4.0, k as f32 * 0.29 + 1.0);
            let div = (curl_noise(q + Vec2::X * eps, 0.3).x - curl_noise(q - Vec2::X * eps, 0.3).x
                + curl_noise(q + Vec2::Y * eps, 0.3).y
                - curl_noise(q - Vec2::Y * eps, 0.3).y)
                / (2.0 * eps);
            assert!(div.abs() < 0.05, "divergence {div} at {q}");
        }
    }

    #[test]
    fn vortex_peaks_near_1_12_core_at_peak_speed() {
        let v = Vortex {
            pos: Vec2::ZERO,
            core: 100.0,
            peak_speed: 50.0,
        };
        let at_peak = v.velocity(Vec2::new(112.1, 0.0)).length();
        assert!((at_peak - 50.0).abs() < 0.5, "{at_peak}");
        // Counter-clockwise for positive speed: at +x the flow points +y.
        assert!(v.velocity(Vec2::new(100.0, 0.0)).y > 0.0);
        assert!(v.velocity(Vec2::new(50.0, 0.0)).length() < at_peak);
        assert!(v.velocity(Vec2::new(300.0, 0.0)).length() < at_peak);
    }

    #[test]
    fn soft_limit_bounds_length() {
        let v = soft_limit(Vec2::new(1000.0, 0.0), 200.0);
        assert!(v.length() <= 200.0 + 1e-3);
        let small = soft_limit(Vec2::new(10.0, 0.0), 200.0);
        assert!((small.length() - 10.0).abs() < 0.1);
    }
}
