//! The glass window scene: a stained-glass window glowing in the dark. Voronoi
//! cells are panes of coloured glass, their edges are lead cames, and light
//! comes through from behind. The music moves the pane centres and the light.
//!
//! Its controls speak the window's language: panes dance, the window breathes,
//! ripples run through it, it sways; light comes through, warms or cools, and
//! single panes glint.
//!
//! Drawing: one full-screen quad; `shaders/glass.wgsl` finds each pixel's
//! pane (nearest centre) and its distance to the lead (the nearest cell edge).
//! The pane centres rest on a jittered hexagonal lattice; `glass_motion.rs`
//! moves them with the music.

use bevy::{
    post_process::bloom::Bloom,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
};

use crate::{
    scene::{Scene, SceneControl, SceneParam, SceneTable},
    tuning::{ControlSpec, ParamSpec, Transformation as T, Transforms, Tuning, ctl, param},
};

const PANES: &str = "PANES";
const LIGHT: &str = "LIGHT";
const GLASS: &str = "GLASS AND LEAD";

pub const TABLE: SceneTable = SceneTable { groups: &[PANES, LIGHT, GLASS], controls: &CONTROLS, params: &PARAMS };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    DanceStep,
    DanceTurn,
    Breath,
    Ripple,
    Sway,
    Backlight,
    LightHue,
    Glints,
    TierLow,
    TierLowMid,
    TierHighMid,
    TierHigh,
    Havoc,
    Spotlight,
    SpotHeight,
    SpotSway,
}

impl Control {
    pub const COUNT: usize = 16;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
    pub const ALL: [Control; Self::COUNT] = [
        Control::DanceStep,
        Control::DanceTurn,
        Control::Breath,
        Control::Ripple,
        Control::Sway,
        Control::Backlight,
        Control::LightHue,
        Control::Glints,
        Control::TierLow,
        Control::TierLowMid,
        Control::TierHighMid,
        Control::TierHigh,
        Control::Havoc,
        Control::Spotlight,
        Control::SpotHeight,
        Control::SpotSway,
    ];
}

impl SceneControl for Control {
    const SCENE: Scene = Scene::Glass;
    fn index(self) -> usize {
        self as usize
    }
}

const fn tier(key: &'static str, label: &'static str, routes: &'static [(T, f32)]) -> ControlSpec {
    ctl(key, label, LIGHT, 0.0, (0.0, 1.0), 1.5, (0.0, 1.5), 0.0, routes, 2)
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    // Motion, in pane spacings; together they never move a pane more than
    // 0.35 (glass_motion::MAX_OFFSET).
    // Every pane dances the salsa basic step on the beat (see glass_motion.rs).
    ctl("dance_step", "dance step (x spacing)", PANES, 0.02, (0.0, 0.3), 0.4, (0.0, 0.3), 0.0, &[(T::Pulse, 0.10), (T::Energy, 0.06), (T::Drive, 0.04)], 3),
    ctl("dance_turn", "dancers turn (turns/min)", PANES, 0.5, (0.0, 4.0), 4.0, (0.0, 8.0), 0.0, &[(T::Busyness, 1.0)], 2),
    ctl("breath", "breath: swell at the edge (x spacing)", PANES, 0.0, (0.0, 0.3), 0.4, (0.0, 0.4), 0.0, &[(T::Groove, 0.3)], 3),
    ctl("ripple", "ripple strength (x spacing)", PANES, 0.0, (0.0, 0.6), 1.0, (0.0, 1.0), 0.0, &[(T::Attacks, 0.8)], 3),
    ctl("sway", "sway of the lower panes (x spacing)", PANES, 0.0, (0.0, 0.3), 0.4, (0.0, 0.4), 0.0, &[(T::LowEnd, 0.2)], 3),
    ctl("backlight", "backlight", LIGHT, 0.7, (0.0, 1.5), 1.0, (0.0, 2.0), 0.8, &[(T::Energy, 0.3), (T::Arrival, 0.3)], 2),
    ControlSpec {
        bipolar: true,
        // The dominant pitch mostly sits low: start a little warm, so low music
        // is about neutral and high melodies turn warm.
        ..ctl("light_hue", "light warmth (cool..warm)", LIGHT, 0.3, (-1.0, 1.0), 1.0, (-1.0, 1.0), 0.0, &[(T::Pitch, 1.0)], 2)
    },
    ctl("glints", "glints", LIGHT, 0.0, (0.0, 1.0), 1.5, (0.0, 1.5), 0.0, &[(T::Sparkle, 0.6)], 2),
    tier("tier_low", "bottom tier glow", &[(T::LowEnd, 0.5)]),
    tier("tier_low_mid", "lower tier glow", &[(T::LowMids, 0.5)]),
    tier("tier_high_mid", "upper tier glow", &[(T::HighMids, 0.5)]),
    tier("tier_high", "top tier glow", &[(T::Sparkle, 0.5)]),
    // Rap bursting in shakes the window to pieces (within the panes' reach).
    ctl("havoc", "havoc: panes shake (x spacing)", PANES, 0.0, (0.0, 0.3), 0.4, (0.0, 0.4), 0.0, &[(T::Havoc, 0.35)], 3),
    // A singer: a spotlight behind the glass, dancing.
    ctl("spotlight", "spotlight", LIGHT, 0.0, (0.0, 1.0), 1.5, (0.0, 1.5), 0.0, &[(T::Singing, 1.0), (T::Solo, 0.8)], 2),
    ControlSpec {
        bipolar: true,
        ..ctl("spot_height", "spotlight height (bottom..top)", LIGHT, 0.0, (-1.0, 1.0), 1.5, (-1.0, 1.0), 0.0, &[(T::Melody, 1.2)], 2)
    },
    ctl("spot_sway", "spotlight dance (x half-width)", LIGHT, 0.25, (0.0, 0.8), 0.8, (0.0, 0.8), 0.0, &[(T::Pulse, 0.25)], 2),
];

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    Panes,
    LeadWidth,
    Stiffness,
    Damping,
    Texture,
    Saturation,
}

impl P {
    pub const COUNT: usize = 6;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
    pub const ALL: [P; Self::COUNT] = [P::Panes, P::LeadWidth, P::Stiffness, P::Damping, P::Texture, P::Saturation];
}

impl SceneParam for P {
    const SCENE: Scene = Scene::Glass;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    param("panes", "panes", PANES, 90.0, (24.0, 200.0), 0),
    param("lead_width", "lead width (px)", GLASS, 7.0, (1.0, 16.0), 1),
    param("stiffness", "spring stiffness (Hz)", PANES, 1.5, (0.2, 6.0), 2),
    param("damping", "spring damping (ratio)", PANES, 0.5, (0.05, 1.5), 2),
    param("texture", "glass texture", GLASS, 0.5, (0.0, 1.0), 2),
    param("saturation", "glass saturation", GLASS, 1.0, (0.0, 1.5), 2),
];

// ===========================================================================
// The window.

/// Most panes the shader takes (the `panes` parameter goes to 200; the
/// layout adds a few just outside the screen so edge panes are whole).
/// Pane numbers are stored in a byte, and 255 marks the end of a list.
pub const MAX_PANES: usize = 255;
/// Grid cells, and how many panes each one lists (see [`Grid`]).
pub const MAX_CELLS: usize = 640;
pub const PER_CELL: usize = 32;
/// The grid is in 16-byte words: `PER_CELL` bytes per cell.
const CELL_WORDS: usize = PER_CELL / 16;

/// The window's panes: where each centre is, and what glass it is.
#[derive(Resource, Default)]
pub struct Panes {
    /// Rest position of each pane centre (world px, origin at screen centre).
    pub homes: Vec<Vec2>,
    /// Current position of each pane centre.
    pub seeds: Vec<Vec2>,
    /// Glass colour of each pane (linear RGB).
    pub tints: Vec<Vec3>,
    /// A random number per pane, for its texture.
    pub grain: Vec<f32>,
    /// Spacing of the rest lattice (px).
    pub spacing: f32,
    /// What the layout was made for: pane count and screen size.
    made_for: (usize, Vec2),
}

/// Cathedral glass: (colour, how often). Deep blues and reds lead, as in old
/// windows; pale glass is rare and catches the eye.
const PALETTE: [([f32; 3], f32); 8] = [
    ([0.010, 0.050, 0.50], 0.22), // cobalt
    ([0.55, 0.010, 0.020], 0.20), // ruby
    ([0.85, 0.32, 0.010], 0.12), // amber
    ([0.010, 0.28, 0.07], 0.12), // emerald
    ([0.20, 0.015, 0.38], 0.10), // violet
    ([0.80, 0.55, 0.030], 0.08), // gold
    ([0.010, 0.22, 0.28], 0.10), // teal
    ([0.62, 0.62, 0.52], 0.06), // pale
];

impl Panes {
    /// Lay out `n` panes on a jittered hexagonal lattice over the screen, and
    /// colour them so that neighbours differ. Always the same for the same
    /// count and size.
    pub fn layout(n: usize, size: Vec2) -> Self {
        use rand::{Rng, SeedableRng, rngs::StdRng};
        let mut rng = StdRng::seed_from_u64(1902);
        let half = size / 2.0;
        // Hexagon spacing so that `n` cells cover the screen.
        let a = (2.0 * size.x * size.y / (3f32.sqrt() * n.max(1) as f32)).sqrt();
        let row_h = a * 3f32.sqrt() / 2.0;
        let mut homes = Vec::new();
        let rows = (half.y / row_h).ceil() as i32 + 1;
        let cols = (half.x / a).ceil() as i32 + 1;
        for r in -rows..=rows {
            for c in -cols..=cols {
                let x = (c as f32 + if r.rem_euclid(2) == 1 { 0.5 } else { 0.0 }) * a;
                let jitter = Vec2::new(rng.random_range(-0.3..0.3), rng.random_range(-0.3..0.3)) * a;
                let p = Vec2::new(x, r as f32 * row_h) + jitter;
                // Keep centres up to half a cell outside, so edge panes are whole.
                if p.x.abs() < half.x + 0.5 * a && p.y.abs() < half.y + 0.5 * a {
                    homes.push(p);
                }
            }
        }
        // Too many (very small screens or counts near the limit): keep the middle.
        if homes.len() > MAX_PANES {
            homes.sort_by(|p, q| (p.x.abs() / half.x).max(p.y.abs() / half.y).total_cmp(&(q.x.abs() / half.x).max(q.y.abs() / half.y)));
            homes.truncate(MAX_PANES);
        }

        // Colour: greedy over the 6 nearest neighbours, in a random order.
        let m = homes.len();
        let neighbours: Vec<Vec<usize>> = (0..m)
            .map(|i| {
                let mut d: Vec<(f32, usize)> = (0..m).filter(|&j| j != i).map(|j| (homes[i].distance_squared(homes[j]), j)).collect();
                d.sort_by(|x, y| x.0.total_cmp(&y.0));
                d.iter().take(6).map(|&(_, j)| j).collect()
            })
            .collect();
        let mut colour: Vec<Option<usize>> = vec![None; m];
        let mut order: Vec<usize> = (0..m).collect();
        for i in (1..m).rev() {
            order.swap(i, rng.random_range(0..=i));
        }
        let total: f32 = PALETTE.iter().map(|p| p.1).sum();
        for &i in &order {
            let taken: Vec<usize> = neighbours[i].iter().filter_map(|&j| colour[j]).collect();
            let mut pick = 0;
            for _ in 0..20 {
                let mut r = rng.random_range(0.0..total);
                pick = PALETTE.iter().position(|p| {
                    r -= p.1;
                    r < 0.0
                }).unwrap_or(0);
                if !taken.contains(&pick) {
                    break;
                }
            }
            colour[i] = Some(pick);
        }
        let tints = colour
            .iter()
            .map(|c| {
                // Each sheet of glass is a little different.
                let base = Vec3::from(PALETTE[c.unwrap_or(0)].0);
                base * rng.random_range(0.8..1.15)
            })
            .collect();
        let grain = (0..m).map(|_| rng.random_range(0.0..1.0)).collect();
        Self { seeds: homes.clone(), homes, tints, grain, spacing: a, made_for: (n, size) }
    }
}

/// Which panes can matter where, so the shader looks at ~20 panes per pixel
/// instead of all of them.
///
/// The screen is cut into square cells about one pane wide. Each cell lists
/// the panes whose centre is within `reach` of it. That holds the pixel's own
/// pane (a pane is never wider than its spacing allows: the motion keeps
/// centres apart) and every neighbour whose edge is close enough to matter for
/// the lead and its shadow. Checked against looking at every pane in the tests.
pub struct Grid {
    pub origin: Vec2,
    pub cell: f32,
    pub cols: usize,
    #[cfg_attr(not(test), allow(dead_code))] // the shader works it out from the screen height
    pub rows: usize,
    /// Per cell, up to `PER_CELL` pane numbers, ending with 255.
    pub lists: Vec<[u8; PER_CELL]>,
}

impl Grid {
    pub fn build(seeds: &[Vec2], half: Vec2, spacing: f32) -> Self {
        let mut cell = spacing.max(8.0);
        let (mut cols, mut rows);
        loop {
            cols = (2.0 * half.x / cell).ceil().max(1.0) as usize;
            rows = (2.0 * half.y / cell).ceil().max(1.0) as usize;
            if cols * rows <= MAX_CELLS {
                break;
            }
            cell *= 1.15;
        }
        let origin = -half;
        let reach = 1.6 * spacing;
        let mut lists = Vec::with_capacity(cols * rows);
        let mut near: Vec<(f32, usize)> = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let lo = origin + Vec2::new(c as f32, r as f32) * cell;
                let hi = lo + Vec2::splat(cell);
                let centre = (lo + hi) / 2.0;
                near.clear();
                for (i, &s) in seeds.iter().enumerate() {
                    let out = (lo - s).max(s - hi).max(Vec2::ZERO);
                    if out.length() <= reach {
                        near.push((s.distance_squared(centre), i));
                    }
                }
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut list = [255u8; PER_CELL];
                for (slot, &(_, i)) in list.iter_mut().zip(near.iter()) {
                    *slot = i as u8;
                }
                lists.push(list);
            }
        }
        Self { origin, cell, cols, rows, lists }
    }

    /// The pane list of the cell containing `p` (what the shader looks up).
    #[cfg(test)]
    pub fn at(&self, p: Vec2) -> &[u8; PER_CELL] {
        let q = ((p - self.origin) / self.cell).floor();
        let c = (q.x.max(0.0) as usize).min(self.cols - 1);
        let r = (q.y.max(0.0) as usize).min(self.rows - 1);
        &self.lists[r * self.cols + c]
    }
}

#[derive(Clone, Debug, ShaderType)]
pub struct GlassUniform {
    /// xy: pane centre (world px); z: the pane's grain; w: glint (0..1).
    pub panes: [Vec4; MAX_PANES],
    /// rgb: glass colour (linear); a: the pane's tier glow (0..1).
    pub tints: [Vec4; MAX_PANES],
    /// rgb: the light behind the window (colour × strength); a: unused.
    pub light: Vec4,
    /// xy: half the screen size; z: lead width (px); w: glass texture.
    pub frame: Vec4,
    /// x: saturation; y: pane count.
    pub glass: Vec4,
    /// xy: spotlight centre (px); z: radius (px); w: strength.
    pub spot: Vec4,
    /// xy: grid origin; z: cell size; w: columns.
    pub grid_frame: Vec4,
    /// Pane lists, `PER_CELL` bytes per cell (see [`Grid`]).
    pub grid: [UVec4; MAX_CELLS * CELL_WORDS],
}

impl Default for GlassUniform {
    fn default() -> Self {
        Self {
            panes: [Vec4::ZERO; MAX_PANES],
            tints: [Vec4::ZERO; MAX_PANES],
            light: Vec4::ONE,
            frame: Vec4::ZERO,
            glass: Vec4::ZERO,
            spot: Vec4::ZERO,
            grid_frame: Vec4::ZERO,
            grid: [UVec4::splat(u32::MAX); MAX_CELLS * CELL_WORDS],
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct GlassMaterial {
    #[uniform(0)]
    pub window: GlassUniform,
}

impl Material2d for GlassMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/glass.wgsl".into()
    }
}

/// The quad the window is drawn on.
#[derive(Component)]
struct WindowQuad;

pub struct GlassPlugin;

impl Plugin for GlassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<GlassMaterial>::default())
            .init_resource::<Panes>()
            .add_systems(OnEnter(Scene::Glass), spawn_window)
            // Not `OnEnter`: the first one runs before the camera exists.
            .add_systems(Update, glass_bloom.run_if(in_state(Scene::Glass)).run_if(state_changed::<Scene>))
            .add_systems(OnExit(Scene::Glass), restore_bloom)
            .add_systems(
                Update,
                update_window
                    .after(crate::glass_motion::move_panes)
                    .after(crate::glass_light::light_panes)
                    .run_if(in_state(Scene::Glass)),
            );
    }
}

fn spawn_window(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<GlassMaterial>>) {
    commands.spawn((
        WindowQuad,
        DespawnOnExit(Scene::Glass),
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(materials.add(GlassMaterial::default())),
        Transform::default(),
    ));
}

/// The camera's bloom from before the window was shown.
#[derive(Resource)]
struct SavedBloom(Bloom);

/// A softer bloom than the bubbles': a little light spills over the lead, as
/// through a real window, but the lead stays near black. (A bloom threshold
/// would be better, but the prefilter turned the whole frame black here.)
fn glass_bloom(mut commands: Commands, mut camera: Single<&mut Bloom, With<Camera2d>>) {
    commands.insert_resource(SavedBloom(camera.clone()));
    **camera = Bloom {
        intensity: 0.08,
        low_frequency_boost: 0.2,
        ..Bloom::NATURAL
    };
}

fn restore_bloom(mut commands: Commands, saved: Option<Res<SavedBloom>>, mut camera: Single<&mut Bloom, With<Camera2d>>) {
    if let Some(saved) = saved {
        **camera = saved.0.clone();
        commands.remove_resource::<SavedBloom>();
    }
}

/// Light colour for a warmth of −1 (cool) … 0 (neutral) … 1 (warm).
fn light_colour(warmth: f32) -> Vec3 {
    let neutral = Vec3::new(1.0, 0.94, 0.86);
    let towards = if warmth >= 0.0 { Vec3::new(1.0, 0.78, 0.52) } else { Vec3::new(0.72, 0.86, 1.0) };
    neutral.lerp(towards, warmth.abs().min(1.0))
}

fn update_window(
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    screen: Single<&Window>,
    mut panes: ResMut<Panes>,
    light: Res<crate::glass_light::Light>,
    clock: Res<crate::tuning::MusicClock>,
    mut quad: Query<(&mut Transform, &MeshMaterial2d<GlassMaterial>), With<WindowQuad>>,
    mut materials: ResMut<Assets<GlassMaterial>>,
) {
    let size = screen.size();
    let n = tuning.get(P::Panes).round() as usize;
    if panes.made_for != (n, size) {
        *panes = Panes::layout(n, size);
    }
    let Ok((mut tf, handle)) = quad.single_mut() else { return };
    tf.scale = size.extend(1.0);
    let Some(mut material) = materials.get_mut(&handle.0) else { return };
    let u = &mut material.window;
    for (i, (&p, (&tint, &grain))) in panes.seeds.iter().zip(panes.tints.iter().zip(&panes.grain)).enumerate() {
        u.panes[i] = Vec4::new(p.x, p.y, grain, light.glint.get(i).copied().unwrap_or(0.0));
        u.tints[i] = tint.extend(light.tier.get(i).copied().unwrap_or(0.0));
    }
    let backlight = tuning.control(Control::Backlight, &transforms);
    let warmth = tuning.control(Control::LightHue, &transforms);
    u.light = (light_colour(warmth) * backlight).extend(0.0);
    u.frame = Vec4::new(size.x / 2.0, size.y / 2.0, tuning.get(P::LeadWidth), tuning.get(P::Texture));
    u.glass = Vec4::new(tuning.get(P::Saturation), panes.seeds.len() as f32, 0.0, 0.0);
    // The spotlight dances the salsa too: across the window on the 8 counts
    // (out on 1, back on 5), up and down with the melody.
    let c = |control| tuning.control(control, &transforms);
    let half = size / 2.0;
    let x = crate::glass_motion::salsa(clock.count(8)) * c(Control::SpotSway) * half.x;
    let y = c(Control::SpotHeight) * 0.75 * half.y;
    u.spot = Vec4::new(x, y, 0.38 * size.y, c(Control::Spotlight));

    let grid = Grid::build(&panes.seeds, size / 2.0, panes.spacing);
    u.grid_frame = Vec4::new(grid.origin.x, grid.origin.y, grid.cell, grid.cols as f32);
    for (c, list) in grid.lists.iter().enumerate() {
        for w in 0..CELL_WORDS {
            let word = |k: usize| u32::from_le_bytes(list[w * 16 + k * 4..w * 16 + k * 4 + 4].try_into().unwrap());
            u.grid[c * CELL_WORDS + w] = UVec4::new(word(0), word(1), word(2), word(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's two questions, answered by looking at `candidates`: which
    /// pane is nearest, and how far is the nearest edge of that pane.
    fn pane_and_edge(p: Vec2, seeds: &[Vec2], candidates: impl Iterator<Item = usize> + Clone) -> (usize, f32) {
        let k = candidates.clone().min_by(|&i, &j| p.distance_squared(seeds[i]).total_cmp(&p.distance_squared(seeds[j]))).unwrap();
        let a = seeds[k];
        let edge = candidates
            .filter(|&j| j != k)
            .map(|j| {
                let ab = seeds[j] - a;
                (0.5 * (a + seeds[j]) - p).dot(ab / ab.length())
            })
            .fold(f32::INFINITY, f32::min);
        (k, edge)
    }

    #[test]
    fn grid_gives_the_same_panes_and_lead_as_looking_at_every_pane() {
        use rand::{Rng, SeedableRng, rngs::StdRng};
        let mut rng = StdRng::seed_from_u64(5);
        for (n, size) in [(24, Vec2::new(960.0, 540.0)), (90, Vec2::new(1920.0, 1080.0)), (200, Vec2::new(1920.0, 1080.0)), (200, Vec2::new(3840.0, 2160.0)), (90, Vec2::new(800.0, 1200.0))] {
            let mut panes = Panes::layout(n, size);
            // Moved off their homes, as the music will (up to 0.35 of the spacing).
            for s in &mut panes.seeds {
                *s += Vec2::new(rng.random_range(-0.35..0.35), rng.random_range(-0.35..0.35)) * panes.spacing;
            }
            let grid = Grid::build(&panes.seeds, size / 2.0, panes.spacing);
            assert!(grid.cols * grid.rows <= MAX_CELLS);
            let all = 0..panes.seeds.len();
            for _ in 0..4000 {
                let p = Vec2::new(rng.random_range(-0.5..0.5) * size.x, rng.random_range(-0.5..0.5) * size.y);
                let list = grid.at(p);
                assert!(list[PER_CELL - 1] == 255, "{n} panes: a cell list is full");
                let near = list.iter().take_while(|&&i| i != 255).map(|&i| i as usize);
                let (k1, e1) = pane_and_edge(p, &panes.seeds, all.clone());
                let (k2, e2) = pane_and_edge(p, &panes.seeds, near);
                assert_eq!(k1, k2, "{n} panes at {p}: wrong pane");
                // The shader only uses the edge distance near the lead (lead + shadow).
                if e1 < 40.0 || e2 < 40.0 {
                    assert!((e1 - e2).abs() < 1e-3, "{n} panes at {p}: edge {e1} vs {e2}");
                }
            }
        }
    }

    #[test]
    fn tables_follow_enum_order() {
        for (i, p) in P::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
        }
        for (i, c) in Control::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i);
        }
    }
}
