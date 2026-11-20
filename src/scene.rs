//! Scenes: the different ways the music can be drawn.
//!
//! Every scene listens to the same music: signals and transformations are
//! shared and keep running when the scene changes. Each scene owns its
//! controls and constants, named in its own world's language (the bubbles'
//! physics, the glass window's panes and light, the pendulums' swing, the
//! aurora's curtains, the ballroom's couples), as a [`SceneTable`].
//!
//! Scene code uses its own typed enums (`bubble_controls::Control`,
//! `glass::Control`, ...); generic code (preferences, solo, offline analysis)
//! uses [`ControlId`] and [`ParamId`].
//!
//! `Tab` switches to the next scene; `--scene NAME` picks one at start.

use bevy::prelude::*;

use crate::{
    config::Config,
    tuning::{ControlSpec, ParamSpec, Tuning},
};

#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scene {
    #[default]
    Bubbles,
    Glass,
    Pendulums,
    Aurora,
    Ballroom,
}

/// What a scene can be tuned with.
pub struct SceneTable {
    /// Headings in the preferences panel, in order.
    pub groups: &'static [&'static str],
    pub controls: &'static [ControlSpec],
    pub params: &'static [ParamSpec],
}

impl Scene {
    pub const COUNT: usize = 5;
    pub const ALL: [Scene; Self::COUNT] = [Scene::Bubbles, Scene::Glass, Scene::Pendulums, Scene::Aurora, Scene::Ballroom];

    pub fn key(self) -> &'static str {
        match self {
            Scene::Bubbles => "bubbles",
            Scene::Glass => "glass",
            Scene::Pendulums => "pendulums",
            Scene::Aurora => "aurora",
            Scene::Ballroom => "ballroom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scene::Bubbles => "bubbles",
            Scene::Glass => "glass window",
            Scene::Pendulums => "pendulums",
            Scene::Aurora => "aurora",
            Scene::Ballroom => "ballroom",
        }
    }

    pub fn find(key: &str) -> Option<Scene> {
        Scene::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn next(self) -> Scene {
        Scene::ALL[(self as usize + 1) % Self::COUNT]
    }

    pub fn table(self) -> &'static SceneTable {
        match self {
            Scene::Bubbles => &crate::bubble_controls::TABLE,
            Scene::Glass => &crate::glass::TABLE,
            Scene::Pendulums => &crate::pendulums::TABLE,
            Scene::Aurora => &crate::aurora::TABLE,
            Scene::Ballroom => &crate::ballroom::TABLE,
        }
    }

    pub fn controls(self) -> impl Iterator<Item = ControlId> {
        (0..self.table().controls.len()).map(move |index| ControlId { scene: self, index })
    }

    pub fn params(self) -> impl Iterator<Item = ParamId> {
        (0..self.table().params.len()).map(move |index| ParamId { scene: self, index })
    }
}

/// Any scene's control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControlId {
    pub scene: Scene,
    pub index: usize,
}

impl ControlId {
    pub fn spec(self) -> &'static ControlSpec {
        &self.scene.table().controls[self.index]
    }

    /// `scene.key`, or a bare key searched in every scene (first match).
    pub fn find(name: &str) -> Option<ControlId> {
        match name.split_once('.') {
            Some((scene, key)) => Scene::find(scene)?.controls().find(|c| c.spec().key == key),
            None => Scene::ALL.into_iter().flat_map(Scene::controls).find(|c| c.spec().key == name),
        }
    }
}

/// Any scene's constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamId {
    pub scene: Scene,
    pub index: usize,
}

impl ParamId {
    pub fn spec(self) -> &'static ParamSpec {
        &self.scene.table().params[self.index]
    }
}

/// A scene's typed control enum.
pub trait SceneControl: Copy {
    const SCENE: Scene;
    fn index(self) -> usize;
    fn id(self) -> ControlId {
        ControlId { scene: Self::SCENE, index: self.index() }
    }
}

/// A scene's typed constant enum.
pub trait SceneParam: Copy {
    const SCENE: Scene;
    fn index(self) -> usize;
    fn id(self) -> ParamId {
        ParamId { scene: Self::SCENE, index: self.index() }
    }
}

pub struct ScenePlugin {
    pub initial: Scene,
}

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(self.initial).add_systems(Update, switch_on_tab);
    }
}

/// Switch to `scene` and remember it in the preferences.
pub fn switch_to(scene: Scene, next: &mut NextState<Scene>, tuning: &mut Tuning, cfg: &Config) {
    next.set(scene);
    tuning.scene = scene;
    crate::prefs::save(tuning, cfg);
}

fn switch_on_tab(
    keys: Res<ButtonInput<KeyCode>>,
    current: Res<State<Scene>>,
    mut next: ResMut<NextState<Scene>>,
    mut tuning: ResMut<Tuning>,
    cfg: Res<Config>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        switch_to(current.get().next(), &mut next, &mut tuning, &cfg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_has_unique_keys_and_known_groups() {
        for scene in Scene::ALL {
            let t = scene.table();
            let mut keys: Vec<&str> = t.controls.iter().map(|c| c.key).collect();
            keys.extend(t.params.iter().map(|p| p.key));
            let n = keys.len();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), n, "{}: duplicate keys", scene.key());
            for c in t.controls {
                assert!(t.groups.contains(&c.group), "{}: {} has unknown group {}", scene.key(), c.key, c.group);
                for &(_, a) in c.routes {
                    assert!(a.abs() <= c.amount_max, "{}: default amount out of range", c.key);
                }
                assert!(c.base >= c.base_range.0 && c.base <= c.base_range.1, "{}", c.key);
            }
            for p in t.params {
                assert!(t.groups.contains(&p.group), "{}: {} has unknown group {}", scene.key(), p.key, p.group);
                assert!(p.default >= p.range.0 && p.default <= p.range.1, "{}", p.key);
            }
        }
    }

    #[test]
    fn controls_are_found_by_name() {
        use crate::{bubble_controls, glass};
        assert_eq!(ControlId::find("wind_speed"), Some(bubble_controls::Control::WindSpeed.id()));
        assert_eq!(ControlId::find("glass.breath"), Some(glass::Control::Breath.id()));
        assert_eq!(ControlId::find("breath"), Some(glass::Control::Breath.id()));
        assert_eq!(ControlId::find("glass.wind_speed"), None);
    }
}
