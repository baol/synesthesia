use bevy::{
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};

const BUBBLE_SHADER: &str = "shaders/bubble.wgsl";

/// One quad mesh and one material shared by every bubble, so Bevy can batch them.
#[derive(Resource)]
pub struct BubbleAssets {
    pub mesh: Handle<Mesh>,
    pub material: Handle<BubbleMaterial>,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BubbleMaterial {
    #[uniform(0)]
    pub tint: LinearRgba,
}

impl Material2d for BubbleMaterial {
    fn fragment_shader() -> ShaderRef {
        BUBBLE_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

pub struct BubbleRenderPlugin;

impl Plugin for BubbleRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<BubbleMaterial>::default())
            .add_systems(PreStartup, setup_assets);
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BubbleMaterial>>,
) {
    commands.insert_resource(BubbleAssets {
        // Unit quad; each bubble scales it to its diameter.
        mesh: meshes.add(Rectangle::new(1.0, 1.0)),
        material: materials.add(BubbleMaterial {
            // rgb: grey gain of rim and reflections; alpha: glint intensity (HDR).
            tint: LinearRgba::new(0.85, 0.85, 0.88, 6.0),
        }),
    });
}
