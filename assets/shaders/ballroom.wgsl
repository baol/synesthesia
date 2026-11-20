// Couples of glowing dancers seen from above (a ballroom is only the
// inspiration: nothing of the room is drawn but the disco ball's flecks).
//
// First the ground they dance on, one of five:
//   0 water:  dark water; each dancer rings it around itself, and is
//             mirrored in it, wobbling with the ripples.
//   1 space:  stars that twinkle and a faint nebula.
//   2 sky:    deep blue, with clouds below that catch the dancers' light.
//   3 black:  only a faint haze of their light.
//   4 mirror: each dancer over its reflection, a little nearer the middle (as
//             a camera above the middle sees it), and the disco ball in it.
// Then the light: the house lights (a glow in the middle) and the dancers'
// own glow, shown by the ground; the disco ball's flecks sweeping round; and
// for each dancer its trail (the path of the last moments, thinning out),
// the hold between partners (a faint line), and the dancer (a bright core
// with a halo that swells on the beat).
//
// Plain WGSL, no imports. The 2D mesh pipeline outputs world position at
// location 0 (pixels, origin at the screen centre, y up).

const D: u32 = 48u;
const TRAIL: u32 = 10u;
const TAU: f32 = 6.2831853;
// Where a reflection appears, seen from above the middle: this much nearer it.
const MIRRORED: f32 = 0.88;

struct Ballroom {
    // xy: the dancer; z: its radius; w: its light.
    dancers: array<vec4<f32>, 48>,
    // rgb: colour (linear); a: flare on the beat.
    colours: array<vec4<f32>, 48>,
    // xy: the dancer's recent path, TRAIL points per dancer, newest first.
    trail: array<vec4<f32>, 480>,
    // xy: half the screen size; z: house lights; w: trail length (0..1).
    frame: vec4<f32>,
    // x: the ground; y: time; z: the disco ball; w: the partners' hold.
    look: vec4<f32>,
}

@group(2) @binding(0) var<uniform> room: Ballroom;

struct FragmentIn {
    @location(0) world_position: vec4<f32>,
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * noise(q);
        q = q * 2.03 + vec2<f32>(17.0, 9.0);
        a *= 0.5;
    }
    return v;
}

// Distance from p to the segment a–b.
fn to_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

fn rotate(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

// The disco ball's flecks at p: rings of spots round the middle, all turning
// together as the ball turns, drawn out along the way they sweep; many facets
// throw no light here.
fn flecks(p: vec2<f32>, t: f32) -> f32 {
    let spacing = 64.0;
    let q = rotate(p, -0.3 * t);
    let r = length(q);
    let ring = floor(r / spacing);
    if (ring < 1.0) {
        return 0.0;
    }
    let n = floor(TAU * (ring + 0.5) * spacing / 70.0);
    let turn = atan2(q.y, q.x) / TAU + 0.5 + hash(vec2<f32>(ring, 1.0));
    let cell = floor(turn * n);
    let h = hash(vec2<f32>(ring, cell));
    if (h < 0.55) {
        return 0.0;
    }
    let at = (cell + 0.5) / n - 0.5 - hash(vec2<f32>(ring, 1.0));
    let centre = (ring + 0.5) * spacing * vec2<f32>(cos(at * TAU), sin(at * TAU));
    // Each glints as its facet catches the light.
    let glint = 0.5 + 0.5 * sin(t * (1.5 + 3.0 * h) + 40.0 * h);
    let off = q - centre;
    let along = normalize(vec2<f32>(-centre.y, centre.x));
    let d = length(vec2<f32>(dot(off, along) / 2.2, dot(off, normalize(centre))));
    return smoothstep(7.0, 3.0, d) * (0.3 + 0.7 * glint) * (h - 0.4);
}

// A dancer's core and halo at distance d, blurred by `blur` (reflections).
fn shine(d: f32, rad: f32, tint: vec3<f32>, light: f32, flare: f32, blur: f32) -> vec3<f32> {
    let r = rad * blur;
    let core = smoothstep(r, 0.8 * rad, d);
    let hot = clamp(0.25 * light + 0.3 * flare, 0.0, 0.7);
    var c = mix(tint, vec3<f32>(1.0), hot) * core * (0.15 + 2.0 * light + 1.2 * flare * (0.3 + light)) / (blur * blur);
    let halo = exp(-d / (r * (1.2 + 1.6 * flare)));
    c += tint * halo * (0.03 + 0.5 * light + 0.4 * flare * (0.2 + light));
    return c;
}

@fragment
fn fragment(in: FragmentIn) -> @location(0) vec4<f32> {
    let p = in.world_position.xy;
    let half = room.frame.xy;
    let house = room.frame.z;
    let shown = room.frame.w * f32(TRAIL);
    let ground = u32(room.look.x + 0.5);
    let t = room.look.y;
    let disco = room.look.z;
    let hold = room.look.w;

    // Water: each dancer rings the water around it. `bend` is the slope,
    // which moves the reflections and catches the light.
    var bend = vec2<f32>(0.0);
    if (ground == 0u) {
        for (var k = 0u; k < D; k++) {
            let dancer = room.dancers[k];
            let off = p - dancer.xy;
            let d = length(off);
            if (d < 400.0) {
                let amp = (0.3 + 0.6 * dancer.w + 0.5 * room.colours[k].a) * exp(-d / 130.0);
                bend += off / max(d, 1.0) * cos(0.08 * d - 4.0 * t + f32(k)) * amp;
            }
        }
    }

    // The house lights, a glow in the middle.
    let r = p / half;
    var lit = house * vec3<f32>(1.0, 0.85, 0.65) * (0.04 + 0.4 * exp(-1.8 * dot(r, r)));

    var glow = vec3<f32>(0.0);
    var mirrored = vec3<f32>(0.0);
    // Where the reflections are looked up.
    let rp = (p + 10.0 * bend) / MIRRORED;
    for (var k = 0u; k < D; k++) {
        let dancer = room.dancers[k];
        let tint = room.colours[k].rgb;
        let flare = room.colours[k].a;
        let rad = dancer.z;
        let light = dancer.w;
        let d = length(p - dancer.xy);

        // Its light, shown by the ground.
        lit += tint * (0.03 + 0.5 * light + 0.3 * flare) * exp(-d / (rad * 6.0)) * 0.6;

        if (ground == 0u || ground == 4u) {
            mirrored += shine(length(rp - dancer.xy) * MIRRORED, rad, tint, light, flare, 1.5);
        }

        // The hold: a faint line to the partner (drawn once, by the leader).
        if (k % 2u == 0u) {
            let partner = room.dancers[k + 1u].xy;
            let apart = length(partner - dancer.xy);
            // Fades out as the hold is broken.
            let held = smoothstep(3.0 * hold, 1.2 * hold, apart);
            glow += tint * (0.04 + 0.12 * light) * held * smoothstep(1.4, 0.2, to_segment(p, dancer.xy, partner));
        }

        // The trail: the dancer's path, thinner and fainter the older it is.
        if (d < 300.0 && shown > 0.5) {
            var trail = 0.0;
            var head = dancer.xy;
            for (var i = 0u; i < TRAIL; i++) {
                let age = (f32(i) + 0.5) / shown;
                if (age >= 1.0) {
                    break;
                }
                let tail = room.trail[k * TRAIL + i].xy;
                let width = 0.7 * rad * (1.0 - 0.8 * age);
                let fade = (1.0 - age) * (1.0 - age);
                trail = max(trail, fade * smoothstep(width, 0.3 * width, to_segment(p, head, tail)));
                head = tail;
            }
            glow += tint * trail * (0.05 + 0.4 * light);
        }

        glow += shine(d, rad, tint, light, flare, 1.0);
    }

    // The ground.
    var col = vec3<f32>(0.0);
    switch ground {
        case 0u: {
            // Dark water, the light on its ripples, the dancers in it.
            col = vec3<f32>(0.002, 0.008, 0.014) + 0.05 * lit;
            let catch_light = max(dot(bend, normalize(vec2<f32>(-0.4, 0.9))), 0.0);
            col += catch_light * catch_light * (0.004 + 0.12 * lit);
            col += 0.3 * mirrored;
        }
        case 1u: {
            // Stars in two layers, the far one drifting slower, and a nebula.
            var stars = 0.0;
            for (var layer = 0; layer < 2; layer++) {
                let size = select(70.0, 34.0, layer == 1);
                let q = p + vec2<f32>(t * select(3.0, 1.2, layer == 1), 0.0);
                let cell = floor(q / size);
                let h = hash(cell + f32(layer) * 31.0);
                let at = (cell + 0.2 + 0.6 * vec2<f32>(h, hash(cell + 7.0))) * size;
                let twinkle = 0.6 + 0.4 * sin(t * (0.8 + 2.0 * h) + 50.0 * h);
                stars += pow(h, 6.0) * twinkle * smoothstep(1.6, 0.3, length(q - at)) * select(1.0, 0.5, layer == 1);
            }
            let cloud = fbm(p / 420.0 + vec2<f32>(t * 0.01, 0.0));
            let nebula = mix(vec3<f32>(0.020, 0.006, 0.032), vec3<f32>(0.004, 0.018, 0.028), fbm(p / 260.0 + 5.0));
            col = vec3<f32>(0.0008, 0.0008, 0.002) + nebula * smoothstep(0.35, 0.8, cloud) + stars * vec3<f32>(0.9, 0.92, 1.0) + 0.03 * lit;
        }
        case 2u: {
            // Night sky, and clouds below drifting by, lit by the dancers.
            let drift = vec2<f32>(t * 0.012, t * 0.004);
            let cloud = smoothstep(0.38, 0.72, fbm(p / 380.0 + drift));
            let deep = mix(vec3<f32>(0.006, 0.014, 0.06), vec3<f32>(0.018, 0.010, 0.05), 0.5 + 0.5 * r.y);
            // Thicker clouds are lighter on top, and take more of the dancers' light.
            let top = vec3<f32>(0.06, 0.065, 0.09) * (0.6 + 0.6 * fbm(p / 90.0 + drift * 3.0));
            col = mix(deep, top + 0.4 * lit, cloud) + 0.02 * lit;
        }
        case 4u: {
            // A dark mirror: the dancers' reflections, and the disco ball overhead.
            col = vec3<f32>(0.004, 0.004, 0.006) + 0.03 * lit + 0.45 * mirrored;
            let ball = length(p) / 26.0;
            if (ball < 1.0 && disco > 0.02) {
                let facet = hash(floor(rotate(p, -0.3 * t) / 6.0));
                col += disco * (0.08 + 0.5 * pow(facet, 6.0)) * (1.0 - ball * ball) * vec3<f32>(0.9, 0.92, 1.0);
            }
        }
        default: {
            // Black, with a breath of blue and the haze of their light.
            col = vec3<f32>(0.0015, 0.002, 0.005) + 0.05 * lit;
        }
    }

    // The disco ball's flecks.
    if (disco > 0.01) {
        col += 1.4 * disco * flecks(p, t) * vec3<f32>(1.0, 0.95, 0.9);
    }
    return vec4<f32>(col + glow, 1.0);
}
