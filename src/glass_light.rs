//! The light through the glass window, pane by pane (the backlight and its
//! warmth are for the whole window; see `update_window` in glass.rs):
//!
//! - **glints**: each rise of the control flares random panes to full
//!   brightness for a moment, more panes for a bigger rise: bright enough to
//!   bloom, but small,
//!   so the window as a whole doesn't pump;
//! - **tiers**: four horizontal bands, bottom to top, each glowing with its
//!   own control (by default the low end, low mids, high mids and sparkle, so
//!   the window lights up where the sound is, low notes low). The tiers share
//!   the light: each glows by how strong its control is compared with the
//!   strongest of the last few seconds (cubed, for contrast), so the light
//!   moves between tiers as the balance shifts while the window as a whole
//!   stays as bright. A pane takes its glow from where its centre rests,
//!   blended between neighbouring tiers, so each pane is lit evenly, as one
//!   piece of glass.

use bevy::prelude::*;
use rand::Rng;

use crate::{
    glass::{Control, Panes},
    glass_motion::Rises,
    scene::Scene,
    tuning::{Transforms, Tuning},
};

/// A glint fades with this time constant (s).
const GLINT_FADE: f32 = 0.2;
/// Rises of the glints control smaller than this don't flare anything.
const GLINT_THRESHOLD: f32 = 0.07;
/// Panes flared per unit of rise (at least one; a rise of 0.5 flares 4).
const GLINTS_PER_RISE: f32 = 8.0;
/// How long the tiers' reference (the strongest tier lately) takes to follow (s).
const TIER_MEMORY: f32 = 3.0;

/// Per-pane light, 0..1 each: what the shader multiplies the glass by.
#[derive(Resource, Default)]
pub struct Light {
    pub glint: Vec<f32>,
    pub tier: Vec<f32>,
    /// The strongest tier control lately.
    reference: f32,
    rises: Rises,
    /// Glints flared so far (for the offline summary).
    pub flared: usize,
}

/// The four tier glows, bottom to top.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tiers(pub [f32; 4]);

impl Tiers {
    pub fn from_tuning(tuning: &Tuning, transforms: &Transforms) -> Self {
        let c = |control| tuning.control(control, transforms);
        Self([c(Control::TierLow), c(Control::TierLowMid), c(Control::TierHighMid), c(Control::TierHigh)])
    }

    /// The glow at height `y` (px from the screen centre, up), blended
    /// between the nearest tier centres.
    pub fn at(&self, y: f32, half_h: f32) -> f32 {
        let x = ((y / half_h + 1.0) * 0.5 * 4.0 - 0.5).clamp(0.0, 3.0);
        let k = (x.floor() as usize).min(2);
        let f = x - k as f32;
        self.0[k] * (1.0 - f) + self.0[k + 1] * f
    }
}

pub struct GlassLightPlugin;

impl Plugin for GlassLightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Light>()
            .add_systems(OnEnter(Scene::Glass), |mut l: ResMut<Light>| *l = Light::default())
            .add_systems(Update, light_panes.after(crate::tuning::SignalSet).run_if(in_state(Scene::Glass)));
    }
}

pub fn light_panes(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    screen: Single<&Window>,
    panes: Res<Panes>,
    mut light: ResMut<Light>,
) {
    let glints = tuning.control(Control::Glints, &transforms);
    let tiers = Tiers::from_tuning(&tuning, &transforms);
    let dt = time.delta_secs().min(0.05);
    light.step(&panes, glints, tiers, screen.size(), dt, &mut rand::rng());
}

impl Light {
    pub fn step(&mut self, panes: &Panes, glints: f32, tiers: Tiers, screen: Vec2, dt: f32, rng: &mut impl Rng) {
        let n = panes.homes.len();
        if self.glint.len() != n {
            self.glint = vec![0.0; n];
            self.tier = vec![0.0; n];
        }
        let half = screen / 2.0;
        let fade = (-dt / GLINT_FADE).exp();
        for g in &mut self.glint {
            *g *= fade;
        }
        if let Some(rise) = self.rises.step(glints)
            && rise > GLINT_THRESHOLD
            && n > 0
        {
            let count = (rise * GLINTS_PER_RISE).ceil() as usize;
            for _ in 0..count {
                // A pane that shows (its centre on screen).
                for _ in 0..8 {
                    let i = rng.random_range(0..n);
                    let h = panes.homes[i];
                    if h.x.abs() < half.x && h.y.abs() < half.y {
                        // Always a full flare: the rise says how many, not how bright.
                        self.glint[i] = 1.0;
                        self.flared += 1;
                        break;
                    }
                }
            }
        }
        // Each tier against the strongest lately (cubed, for contrast), half lit
        // on average.
        let strongest = tiers.0.iter().copied().fold(0.0, f32::max);
        self.reference = crate::features::follow(self.reference, strongest, dt, TIER_MEMORY, TIER_MEMORY);
        let reference = self.reference.max(0.05);
        // With the tiers off (all near 0), every pane gets the average share,
        // which leaves the light as it is.
        let on = (self.reference / 0.1).clamp(0.0, 1.0);
        let shared = Tiers(tiers.0.map(|t| 0.5 + on * (0.5 * (t / reference).powi(3).min(3.0) - 0.5)));
        for (t, h) in self.tier.iter_mut().zip(&panes.homes) {
            *t = shared.at(h.y, half.y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn tiers_light_the_window_bottom_to_top() {
        let t = Tiers([1.0, 0.0, 0.0, 0.5]);
        let h = 540.0;
        assert!((t.at(-540.0, h) - 1.0).abs() < 1e-6, "bottom edge is all the bottom tier");
        assert!((t.at(540.0, h) - 0.5).abs() < 1e-6, "top edge is all the top tier");
        // Halfway between the bottom two tier centres: half each.
        let y = (1.0 / 4.0 / 0.5 - 1.0) * h; // centre between tier 0 and 1 → x = 0.5
        assert!((t.at(y, h) - 0.5).abs() < 1e-3);
        // Smooth from bottom to top.
        for k in 0..1080 {
            let y = k as f32 - 540.0;
            assert!((t.at(y + 1.0, h) - t.at(y, h)).abs() < 0.01);
        }
    }

    #[test]
    fn tiers_off_leave_every_pane_at_the_average_share() {
        let screen = Vec2::new(1920.0, 1080.0);
        let panes = Panes::layout(90, screen);
        let mut light = Light::default();
        let mut rng = StdRng::seed_from_u64(10);
        for _ in 0..600 {
            light.step(&panes, 0.0, Tiers::default(), screen, 1.0 / 120.0, &mut rng);
        }
        assert!(light.tier.iter().all(|&t| (t - 0.5).abs() < 1e-4));
        // On, with only the bottom tier sounding: bottom panes above the share, top ones below.
        for _ in 0..1200 {
            light.step(&panes, 0.0, Tiers([0.8, 0.0, 0.0, 0.0]), screen, 1.0 / 120.0, &mut rng);
        }
        let (lo, hi): (Vec<_>, Vec<_>) = panes.homes.iter().zip(&light.tier).partition(|(h, _)| h.y < 0.0);
        assert!(lo.iter().map(|p| *p.1).fold(0.0, f32::max) > 0.5);
        assert!(hi.iter().filter(|(h, _)| h.y > 300.0).all(|p| *p.1 < 0.1));
    }

    #[test]
    fn a_rise_flares_a_few_panes_which_fade() {
        let screen = Vec2::new(1920.0, 1080.0);
        let panes = Panes::layout(90, screen);
        let mut light = Light::default();
        let mut rng = StdRng::seed_from_u64(9);
        let dt = 1.0 / 120.0;
        for v in [0.0, 0.2, 0.5, 0.45] {
            light.step(&panes, v, Tiers::default(), screen, dt, &mut rng);
        }
        let lit = light.glint.iter().filter(|&&g| g > 0.5).count();
        assert!((1..=4).contains(&lit), "{lit} panes flared for a rise of 0.5");
        for _ in 0..120 {
            light.step(&panes, 0.0, Tiers::default(), screen, dt, &mut rng);
        }
        assert!(light.glint.iter().all(|&g| g < 0.01), "glints fade within a second");
    }
}
