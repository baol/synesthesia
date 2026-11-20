//! The aurora scene: northern lights over a dark treeline. This is the "follow
//! the music" kind of scene: nothing is choreographed or locked to the beat;
//! the sky takes the shape and the colour of what is playing.
//!
//! - **Shape**: the spectrum is laid along the horizon, bass on the left,
//!   treble on the right, and the curtains reach highest where the music is.
//! - **Colour**: with no clear harmony the lights are the classic pale green
//!   under violet; a clear chord tints them with its place on the circle of
//!   fifths, so related chords are neighbouring colours and a modulation is a
//!   change of sky. Major leans warm, minor cold.
//! - **Chord changes** send a wave of light along the curtains.
//! - Loud music makes them brighter and taller; busy music makes them drift
//!   faster; tension (harmony far from the key) folds them; treble makes the
//!   rays shimmer; onsets make the stars twinkle.
//! - A singer draws a bright band across the curtains, at the height of the
//!   melody.
//!
//! Drawing: one full-screen quad; `shaders/aurora.wgsl`.

use bevy::{
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
};

use crate::{
    features::SPECTRUM_BINS,
    scene::{Scene, SceneControl, SceneParam, SceneTable},
    tuning::{ControlSpec, Notes, ParamSpec, Transformation as T, Transforms, Tuning, ctl, param},
};

const CURTAINS: &str = "CURTAINS";
const LIGHT: &str = "LIGHT";
const SKY: &str = "SKY";

pub const TABLE: SceneTable = SceneTable { groups: &[CURTAINS, LIGHT, SKY], controls: &CONTROLS, params: &PARAMS };

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Brightness,
    Height,
    Shape,
    Drift,
    Fold,
    Rays,
    Surge,
    Colour,
    Warmth,
    Band,
    BandHeight,
    Stars,
}

impl Control {
    pub const COUNT: usize = 12;
    /// Every value in order (the tests check the table follows it).
    #[cfg(test)]
    pub const ALL: [Control; Self::COUNT] = [
        Control::Brightness,
        Control::Height,
        Control::Shape,
        Control::Drift,
        Control::Fold,
        Control::Rays,
        Control::Surge,
        Control::Colour,
        Control::Warmth,
        Control::Band,
        Control::BandHeight,
        Control::Stars,
    ];
}

impl SceneControl for Control {
    const SCENE: Scene = Scene::Aurora;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`Control`].
pub const CONTROLS: [ControlSpec; Control::COUNT] = [
    ctl("brightness", "brightness", LIGHT, 0.25, (0.0, 2.0), 2.0, (0.0, 2.5), 0.9, &[(T::Energy, 1.0), (T::Arrival, 0.5)], 2),
    ctl("height", "how high the curtains reach", CURTAINS, 0.3, (0.0, 1.5), 1.5, (0.05, 1.6), 0.6, &[(T::Energy, 0.4), (T::Fullness, 0.3)], 2),
    ctl("shape", "the spectrum shapes the curtains", CURTAINS, 0.9, (0.0, 1.5), 1.5, (0.0, 1.5), 0.0, &[], 2),
    ctl("drift", "drift (speed)", CURTAINS, 0.2, (0.0, 3.0), 3.0, (0.0, 3.0), 0.3, &[(T::Busyness, 1.2)], 2),
    ctl("fold", "folds", CURTAINS, 0.3, (0.0, 1.5), 1.5, (0.0, 1.5), 0.3, &[(T::Tension, 0.9)], 2),
    ctl("rays", "rays shimmer", LIGHT, 0.3, (0.0, 1.0), 1.5, (0.0, 1.0), 0.3, &[(T::Sparkle, 0.8)], 2),
    ctl("surge", "surge: a wave of light along the curtains", LIGHT, 0.0, (0.0, 1.0), 1.5, (0.0, 1.5), 0.0, &[(T::Chords, 0.9), (T::Arrival, 0.9)], 2),
    ctl("colour", "colour from the harmony", LIGHT, 0.2, (0.0, 1.0), 1.0, (0.0, 1.0), 0.6, &[(T::Harmony, 0.8)], 2),
    ControlSpec { bipolar: true, ..ctl("warmth", "warmth (cold..warm)", LIGHT, 0.0, (-1.0, 1.0), 1.0, (-1.0, 1.0), 0.0, &[(T::Mood, 0.8)], 2) },
    ctl("band", "bright band (a singer)", LIGHT, 0.0, (0.0, 1.0), 1.5, (0.0, 1.5), 0.0, &[(T::Singing, 0.9), (T::Solo, 0.7)], 2),
    ControlSpec {
        bipolar: true,
        ..ctl("band_height", "the band's height (low..high)", LIGHT, 0.0, (-1.0, 1.0), 1.5, (-1.0, 1.0), 0.0, &[(T::Melody, 1.0)], 2)
    },
    ctl("stars", "stars twinkle", SKY, 0.3, (0.0, 1.5), 1.5, (0.0, 2.0), 0.3, &[(T::Attacks, 0.7), (T::LetGo, 0.8)], 2),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    Wheel,
    Treeline,
}

impl P {
    pub const COUNT: usize = 2;
    #[cfg(test)]
    pub const ALL: [P; Self::COUNT] = [P::Wheel, P::Treeline];
}

impl SceneParam for P {
    const SCENE: Scene = Scene::Aurora;
    fn index(self) -> usize {
        self as usize
    }
}

/// In the order of [`P`].
pub const PARAMS: [ParamSpec; P::COUNT] = [
    // Which colour C major gets: 0.2 puts it on green.
    param("wheel", "colour wheel turned by", LIGHT, 0.2, (0.0, 1.0), 2),
    param("treeline", "treeline height", SKY, 0.14, (0.0, 0.4), 2),
];

/// A surge starts when its control rises past this, and can start again once
/// it has fallen below half of it.
const SURGE_ON: f32 = 0.15;
/// It crosses the sky in about this long (s).
const SURGE_CROSSING: f32 = 1.4;

/// What moves on its own in the sky.
#[derive(Resource, Default)]
pub struct Sky {
    /// How far the curtains have drifted, and the rays shimmered (the shader's clocks).
    drift: f32,
    shimmer: f32,
    /// The surge: where its front is (in screen heights from the centre), how
    /// strong, and whether another may start.
    surge_x: f32,
    surge: f32,
    ready: bool,
    pub surges: u32,
}

impl Sky {
    /// Advance by `dt`. `aspect` is the screen's width over its height.
    pub fn step(&mut self, drift: f32, rays: f32, surge: f32, aspect: f32, dt: f32) {
        self.drift += drift * dt;
        self.shimmer += (0.4 + 2.5 * rays) * dt;
        if surge < SURGE_ON / 2.0 {
            self.ready = true;
        }
        if surge > SURGE_ON && self.ready {
            self.ready = false;
            self.surge_x = -aspect - 0.6;
            self.surge = surge.min(1.5);
            self.surges += 1;
        }
        // The front runs left to right and is gone once past the edge.
        self.surge_x += (2.0 * aspect + 1.2) / SURGE_CROSSING * dt;
        if self.surge_x > aspect + 0.6 {
            self.surge = 0.0;
        }
    }
}

#[derive(Clone, Debug, Default, ShaderType)]
pub struct AuroraUniform {
    /// The spectrum, 4 bands per entry, bass first.
    pub spectrum: [Vec4; SPECTRUM_BINS / 4],
    /// xy: half the screen size (px); z: drift; w: shimmer.
    pub frame: Vec4,
    /// x: brightness; y: height; z: how much the spectrum shapes; w: rays.
    pub curtain: Vec4,
    /// x: hue (0..1 round the wheel); y: how much of it; z: warmth (−1..1); w: folds.
    pub colour: Vec4,
    /// x: the surge's front; y: its strength; z: the singer's band; w: its height.
    pub surge: Vec4,
    /// x: stars; y: time (s); z: treeline height.
    pub sky: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct AuroraMaterial {
    #[uniform(0)]
    pub sky: AuroraUniform,
}

impl Material2d for AuroraMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/aurora.wgsl".into()
    }
}

/// The quad the sky is drawn on.
#[derive(Component)]
struct SkyQuad;

pub struct AuroraPlugin;

impl Plugin for AuroraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<AuroraMaterial>::default())
            .init_resource::<Sky>()
            .add_systems(OnEnter(Scene::Aurora), spawn_sky)
            .add_systems(Update, update_sky.after(crate::tuning::SignalSet).run_if(in_state(Scene::Aurora)));
    }
}

fn spawn_sky(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<AuroraMaterial>>) {
    commands.spawn((
        SkyQuad,
        DespawnOnExit(Scene::Aurora),
        Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial2d(materials.add(AuroraMaterial::default())),
        Transform::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_sky(
    time: Res<Time>,
    tuning: Res<Tuning>,
    transforms: Res<Transforms>,
    notes: Res<Notes>,
    screen: Single<&Window>,
    mut sky: ResMut<Sky>,
    mut quad: Query<(&mut Transform, &MeshMaterial2d<AuroraMaterial>), With<SkyQuad>>,
    mut materials: ResMut<Assets<AuroraMaterial>>,
) {
    let c = |control| tuning.control(control, &transforms);
    let size = screen.size();
    let dt = time.delta_secs().min(0.05);
    sky.step(c(Control::Drift), c(Control::Rays), c(Control::Surge), size.x / size.y.max(1.0), dt);

    let Ok((mut tf, handle)) = quad.single_mut() else { return };
    tf.scale = size.extend(1.0);
    let Some(mut material) = materials.get_mut(&handle.0) else { return };
    let u = &mut material.sky;
    for (i, band) in notes.spectrum.chunks_exact(4).enumerate() {
        u.spectrum[i] = Vec4::new(band[0], band[1], band[2], band[3]);
    }
    u.frame = Vec4::new(size.x / 2.0, size.y / 2.0, sky.drift, sky.shimmer);
    u.curtain = Vec4::new(c(Control::Brightness), c(Control::Height), c(Control::Shape), c(Control::Rays));
    u.colour = Vec4::new((notes.hue + tuning.get(P::Wheel)).fract(), c(Control::Colour), c(Control::Warmth), c(Control::Fold));
    u.surge = Vec4::new(sky.surge_x, sky.surge, c(Control::Band), 0.1 + 0.45 * c(Control::BandHeight));
    u.sky = Vec4::new(c(Control::Stars), time.elapsed_secs(), tuning.get(P::Treeline), 0.0);
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

    /// A chord change (the control jumps, then fades over ~a second) starts
    /// one surge, which crosses the sky and is gone; the next one starts only
    /// after the control has come back down.
    #[test]
    fn a_surge_crosses_the_sky_once_per_rise() {
        let mut sky = Sky::default();
        let (aspect, dt) = (16.0 / 9.0, 1.0 / 60.0);
        let mut t = 0.0f32;
        let mut seen_inside = false;
        while t < 6.0 {
            // Two changes: at 1 s and at 1.3 s (too soon: the same surge), then one at 4 s.
            let since = [1.0, 1.3, 4.0].iter().filter(|&&at| t >= at).map(|&at| t - at).fold(f32::INFINITY, f32::min);
            let control = if since.is_finite() { 0.9 * (-since / 0.8).exp() } else { 0.0 };
            sky.step(0.2, 0.3, control, aspect, dt);
            if (1.2..2.0).contains(&t) && sky.surge > 0.0 && sky.surge_x.abs() < aspect {
                seen_inside = true;
            }
            if (2.8..3.9).contains(&t) {
                assert_eq!(sky.surge, 0.0, "the surge is still there at {t}");
            }
            t += dt;
        }
        assert!(seen_inside);
        assert_eq!(sky.surges, 2);
    }
}
