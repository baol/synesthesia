// A row of glowing pendulums over a dark, glossy floor.
//
// Everything is light added to black: each pendulum's string (a faint line),
// its trail (the path of the last moments, thinning out), and its bob (a
// bright core with a halo that swells on the beat). Below the floor line the
// same picture is drawn upside down and dimmer: the reflection.
//
// Plain WGSL, no imports. The 2D mesh pipeline outputs world position at
// location 0 (pixels, origin at the screen centre, y up).

const N: u32 = 12u;
const TRAIL: u32 = 12u;

struct Pendulums {
    // xy: the bob; z: its radius; w: its light.
    bobs: array<vec4<f32>, 12>,
    // rgb: colour (linear); a: flare on the beat.
    colours: array<vec4<f32>, 12>,
    // xy: where the string hangs from.
    pivots: array<vec4<f32>, 12>,
    // xy: the bob's recent path, TRAIL points per bob, newest first.
    trail: array<vec4<f32>, 144>,
    // xy: half the screen size; z: the floor's height; w: trail length (0..1).
    frame: vec4<f32>,
    // x: floor reflection; y: the rail's height; z: time.
    look: vec4<f32>,
}

@group(2) @binding(0) var<uniform> row: Pendulums;

struct FragmentIn {
    @location(0) world_position: vec4<f32>,
}

// Distance from p to the segment a–b.
fn to_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

// The light of the whole row at p.
fn lights(p: vec2<f32>) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    let shown = row.frame.w * f32(TRAIL);
    for (var i = 0u; i < N; i++) {
        let bob = row.bobs[i];
        let tint = row.colours[i].rgb;
        let flare = row.colours[i].a;
        let r = bob.z;
        let light = bob.w;

        // The string: a thin line, catching a little of the bob's light.
        let string = to_segment(p, row.pivots[i].xy, bob.xy);
        col += (vec3<f32>(0.012) + 0.03 * light * tint) * smoothstep(1.6, 0.2, string);

        let d = length(p - bob.xy);
        // The trail: the bob's path, thinner and fainter the older it is.
        // (Only looked at near the bob: it is never longer than this.)
        if (d < 500.0 + 4.0 * r && shown > 0.5) {
            var trail = 0.0;
            var head = bob.xy;
            for (var k = 0u; k < TRAIL; k++) {
                let age = (f32(k) + 0.5) / shown;
                if (age >= 1.0) {
                    break;
                }
                let tail = row.trail[i * TRAIL + k].xy;
                let width = r * (1.0 - 0.8 * age);
                let fade = (1.0 - age) * (1.0 - age);
                trail = max(trail, fade * smoothstep(width, 0.35 * width, to_segment(p, head, tail)));
                head = tail;
            }
            col += tint * trail * (0.06 + 0.5 * light);
        }

        // The bob: a core that turns white-hot with its light, and a halo.
        let core = smoothstep(r, 0.82 * r, d);
        let hot = clamp(0.25 * light + 0.3 * flare, 0.0, 0.7);
        col += mix(tint, vec3<f32>(1.0), hot) * core * (0.10 + 2.2 * light + 1.2 * flare * (0.3 + light));
        let halo = exp(-d / (r * (1.3 + 1.8 * flare)));
        col += tint * halo * (0.03 + 0.7 * light + 0.5 * flare * (0.2 + light));
    }
    return col;
}

@fragment
fn fragment(in: FragmentIn) -> @location(0) vec4<f32> {
    let p = in.world_position.xy;
    let half = row.frame.xy;
    let floor_y = row.frame.z;

    if (p.y >= floor_y) {
        var col = lights(p);
        // The rail the strings hang from.
        let rail = smoothstep(2.5, 0.5, abs(p.y - row.look.y)) * smoothstep(0.86 * half.x, 0.82 * half.x, abs(p.x));
        col += vec3<f32>(0.02, 0.02, 0.025) * rail;
        // A breath of blue in the dark behind, deepest at the top.
        col += vec3<f32>(0.0015, 0.002, 0.005) * (1.0 - (p.y - floor_y) / (half.y - floor_y));
        return vec4<f32>(col, 1.0);
    }

    // The floor: the row upside down, dimmer with depth, a little unsteady.
    let depth = (floor_y - p.y) / (floor_y + half.y);
    let wobble = sin(p.y * 0.11 + row.look.z * 1.3) * 5.0 * depth;
    let mirrored = vec2<f32>(p.x + wobble, 2.0 * floor_y - p.y);
    var col = lights(mirrored) * row.look.x * (1.0 - 0.75 * depth);
    // Where floor meets wall.
    col += vec3<f32>(0.006, 0.006, 0.008) * smoothstep(3.0, 0.0, floor_y - p.y);
    return vec4<f32>(col, 1.0);
}
