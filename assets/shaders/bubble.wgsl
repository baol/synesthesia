// Shiny grey bubble, drawn as an impostor on a flat quad.
//
// Each pixel of the disc is treated as a point on a sphere. The bubble is a thin
// shell, so it is almost transparent: what we see is the environment reflected
// at grazing angles (Fresnel rim), a sharp glint from a key light at the top-left,
// and a dimmer inner reflection on the opposite side.
//
// Plain WGSL, no imports. The 2D mesh pipeline outputs world position at
// location 0 and UV at location 2. Each bubble stores a random seed in its
// translation.z (0..0.01), which also gives a stable draw order.

struct BubbleMaterial {
    // rgb: overall gain (grey), a: glint intensity (HDR, feeds bloom).
    tint: vec4<f32>,
}

@group(2) @binding(0) var<uniform> material: BubbleMaterial;

struct FragmentIn {
    @location(0) world_position: vec4<f32>,
    @location(2) uv: vec2<f32>,
}

fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

// Fake studio environment, looked up by reflection direction (y up, z towards viewer).
fn environment(r: vec3<f32>, seed: f32) -> f32 {
    // Soft overhead light, dark below.
    let sky = 0.04 + 0.45 * smoothstep(-0.3, 0.9, r.y);
    // A faint horizontal band, like a distant horizon or room edge.
    let band_y = 0.05 + 0.1 * seed;
    let band = 0.18 * exp(-pow((r.y - band_y) / 0.07, 2.0));
    // Slightly brighter towards the left, where the key light is.
    let side = 0.12 * smoothstep(0.4, -0.9, r.x);
    return sky + band + side;
}

@fragment
fn fragment(in: FragmentIn) -> @location(0) vec4<f32> {
    // p in [-1, 1] across the quad, y up (UV y points down).
    let p = vec2(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let d2 = dot(p, p);
    let d = sqrt(d2);
    let aa = max(fwidth(d) * 1.25, 1e-4);
    let coverage = 1.0 - smoothstep(1.0 - aa, 1.0, d);
    if coverage <= 0.0 {
        discard;
    }

    let seed = fract(in.world_position.z * 100.0);
    let s1 = hash11(seed);
    let s2 = hash11(seed + 1.7);

    // Sphere normal and reflection of the view ray (orthographic, view along -z).
    let n = vec3(p, sqrt(max(1.0 - d2, 0.0)));
    let ndv = n.z;
    let r = vec3(2.0 * ndv * n.x, 2.0 * ndv * n.y, 2.0 * ndv * ndv - 1.0);

    // Fresnel for a soap film (F0 ≈ 0.04). Exponent < 5 keeps the rim visible on
    // small bubbles; a thin shell also reflects from its inner surface, so ×2.
    let fresnel = 0.04 + 0.96 * pow(1.0 - ndv, 3.0);
    let rim = 2.0 * fresnel * environment(r, s1);

    // Key light, top-left, slightly in front. A broad soft lobe plus a hot core.
    let key = normalize(vec3(-0.55, 0.62, 0.56));
    let rk = max(dot(r, key), 0.0);
    let glint = 0.35 * pow(rk, 24.0) + pow(rk, 220.0 + 120.0 * s2);

    // Light that enters, reflects off the far inner wall, and exits on the
    // opposite side: a dim soft spot bottom-right.
    let inner_c = vec2(0.40, -0.46) + 0.06 * vec2(s1 - 0.5, s2 - 0.5);
    let inner = 0.22 * (1.0 - smoothstep(0.0, 0.22, length(p - inner_c)));

    // Thin dark line just inside the outline, where the film turns edge-on.
    let edge_dark = 1.0 - 0.35 * smoothstep(0.90, 0.97, d) * (1.0 - smoothstep(0.97, 1.0, d));

    let gain = material.tint.rgb * (0.9 + 0.2 * s1);
    let light = (rim * edge_dark + inner) * gain + vec3(material.tint.a * glint);

    // How much the bubble dims what is behind it: very little, more at the rim.
    let occlusion = clamp(0.05 + 0.5 * fresnel, 0.0, 1.0) * coverage;

    // Straight-alpha blending gives  src.rgb·a + dst·(1−a);  dividing by a makes the
    // added light exactly `light · coverage`, independent of the occlusion.
    return vec4(light * coverage / max(occlusion, 1e-3), occlusion);
}
