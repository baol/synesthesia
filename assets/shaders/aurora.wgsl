// Northern lights over a treeline.
//
// Three curtains, one behind the other. Each has a wavy lower edge, bright and
// sharp, and fades upwards over a height that follows the spectrum at that
// place along the horizon (bass on the left, treble on the right). Fine
// vertical rays shimmer in it. Its colour is the classic green under violet,
// tinted by the harmony's hue as far as the harmony is clear. A surge is a
// bright front running along the curtains; the singer's band is a brighter
// stripe at the melody's height.
//
// Coordinates here: screen heights from the centre (y from −1 at the bottom
// to 1 at the top), so the picture keeps its proportions on any screen.
//
// Plain WGSL, no imports. The 2D mesh pipeline outputs world position at
// location 0 (pixels, origin at the screen centre, y up).

struct Aurora {
    // The spectrum, 4 bands per entry, bass first.
    spectrum: array<vec4<f32>, 8>,
    // xy: half the screen size (px); z: drift; w: shimmer.
    frame: vec4<f32>,
    // x: brightness; y: height; z: how much the spectrum shapes; w: rays.
    curtain: vec4<f32>,
    // x: hue; y: how much of it; z: warmth (−1..1); w: folds.
    colour: vec4<f32>,
    // x: the surge's front; y: its strength; z: the singer's band; w: its height.
    surge: vec4<f32>,
    // x: stars; y: time; z: treeline height.
    sky: vec4<f32>,
}

@group(2) @binding(0) var<uniform> au: Aurora;

struct FragmentIn {
    @location(0) world_position: vec4<f32>,
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise, 0..1.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn hsv(h: f32, s: f32, v: f32) -> vec3<f32> {
    let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0));
    return v * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), s);
}

fn band(i: u32) -> f32 {
    return au.spectrum[i / 4u][i % 4u];
}

// The spectrum at x (0 = left edge, bass … 1 = right edge, treble), smooth.
fn spectrum_at(x: f32) -> f32 {
    let at = clamp(x, 0.0, 1.0) * 31.0;
    let i = u32(floor(at));
    let f = fract(at);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(band(i), band(min(i + 1u, 31u)), u);
}

@fragment
fn fragment(in: FragmentIn) -> @location(0) vec4<f32> {
    let half = au.frame.xy;
    let uv = in.world_position.xy / half.y;
    let aspect = half.x / half.y;
    let drift = au.frame.z;
    let shimmer = au.frame.w;
    let brightness = au.curtain.x;
    let height = au.curtain.y;
    let shape = au.curtain.z;
    let rays = clamp(au.curtain.w, 0.0, 1.0);
    let fold = au.colour.w;
    let warmth = au.colour.z;
    let time = au.sky.y;

    // The night sky: a little lighter towards the horizon.
    var col = mix(vec3<f32>(0.0, 0.004, 0.012), vec3<f32>(0.0, 0.0, 0.003), 0.5 * uv.y + 0.5);

    // Stars: one in a few cells of a grid, each twinkling at its own pace.
    let cell = floor(in.world_position.xy / 16.0);
    let pick = hash2(cell);
    if (pick > 0.93) {
        let h2 = hash2(cell + vec2<f32>(5.3, 1.7));
        let centre = vec2<f32>(0.25 + 0.5 * h2, 0.25 + 0.5 * fract(h2 * 7.3));
        let d = length(fract(in.world_position.xy / 16.0) - centre);
        let twinkle = 0.55 + 0.45 * sin(time * (0.8 + 4.0 * h2) + h2 * 40.0);
        let size = 0.05 + 0.07 * fract(pick * 31.0);
        col += vec3<f32>(0.75, 0.85, 1.0) * smoothstep(size, 0.0, d) * (0.12 + au.sky.x * 0.5) * mix(1.0, twinkle, clamp(au.sky.x, 0.0, 1.0));
    }

    // The curtains, back to front.
    let along = uv.x / aspect * 0.5 + 0.5;
    let music = spectrum_at(along);
    let surge = 1.0 + 2.5 * au.surge.y * exp(-pow((uv.x - au.surge.x) / 0.4, 2.0));
    let sung = 1.0 + 2.0 * au.surge.z * exp(-pow((uv.y - au.surge.w) / 0.16, 2.0));
    var lights = vec3<f32>(0.0);
    for (var l = 2; l >= 0; l--) {
        let fl = f32(l);
        // Farther curtains are lower on the sky, smaller and dimmer.
        let scale = 1.0 - 0.22 * fl;
        let xw = uv.x * (0.55 + 0.2 * fl) + drift * (0.3 + 0.13 * fl) + fl * 17.3;
        let edge = -0.42 + 0.16 * fl
            + fold * (0.2 * sin(xw * 1.7) + 0.5 * (noise(vec2<f32>(xw * 1.3, drift * 0.2 + fl * 5.0)) - 0.5));
        let reach = max(0.04, height * scale * (0.3 + shape * 1.4 * music));
        let v = (uv.y - edge) / reach;
        // Sharp below the edge, a long fade above it.
        let profile = exp(min(v, 0.0) * 22.0) * exp(-max(v, 0.0) * 2.3);
        if (profile < 0.002) {
            continue;
        }
        // Rays: fine vertical streaks, leaning with the folds.
        let xr = uv.x + fold * 0.12 * sin(uv.y * 2.5 + xw * 2.0);
        let streak = noise(vec2<f32>(xr * (26.0 + 9.0 * fl) + fl * 3.0, shimmer * (0.5 + 0.2 * fl)));
        let ray = mix(1.0, 0.25 + 1.6 * streak * streak, rays);
        // Slow patches of more and less light along the curtain.
        let patches = 0.55 + 0.45 * noise(vec2<f32>(xw * 3.0, drift * 0.6 + fl));

        let up = clamp(v, 0.0, 1.5);
        let classic = mix(vec3<f32>(0.10, 1.0, 0.35), vec3<f32>(0.55, 0.12, 0.75), smoothstep(0.15, 1.1, up));
        let tinted = hsv(au.colour.x + (fl - 1.0) * 0.07 + 0.10 * up, 0.85, 1.0);
        var c = mix(classic, tinted, clamp(au.colour.y, 0.0, 1.0));
        c *= max(vec3<f32>(1.0 + 0.35 * warmth, 1.0, 1.0 - 0.35 * warmth), vec3<f32>(0.0));
        // The lower rim burns nearly white.
        let rim = 0.5 * exp(-abs(v) * 18.0);
        lights += (c + vec3<f32>(rim)) * profile * ray * patches * surge * sung * brightness * scale;
    }
    col += lights;

    // The treeline: hills, with the ragged tops of firs.
    let trees = au.sky.z;
    let ridge = -1.0 + trees * (0.9 + 0.7 * noise(vec2<f32>(uv.x * 2.3, 1.0)) + 0.25 * noise(vec2<f32>(uv.x * 9.0, 5.0)))
        + 0.035 * pow(noise(vec2<f32>(uv.x * 70.0, 9.0)), 2.0) * min(trees * 10.0, 1.0);
    if (uv.y < ridge) {
        col = vec3<f32>(0.0004, 0.0006, 0.001);
    }
    return vec4<f32>(col, 1.0);
}
