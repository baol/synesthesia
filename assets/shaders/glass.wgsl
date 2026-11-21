// Stained-glass window, lit from behind.
//
// The panes are the Voronoi cells of the pane centres. Only the ~20 panes
// listed for the pixel's grid cell are looked at (see `Grid` in glass.rs).
// For each pixel:
//   1. its pane is the nearest centre;
//   2. its distance to the lead is the distance to the nearest cell edge,
//      measured to each bisector between its centre and another one (exact,
//      so the lead keeps an even width however the cells are shaped);
//   3. glass: the pane's colour times the light behind it (the window's
//      backlight, the pane's tier glow and any glint), with uneven thickness
//      and streaks, darker where it meets the lead;
//   4. lead: near black, with a faint sheen along the ridge of each came.
//
// Plain WGSL, no imports. The 2D mesh pipeline outputs world position at
// location 0 (pixels, origin at the screen centre).

const MAX_PANES: u32 = 255u;
const PER_CELL: u32 = 32u;
const CELL_WORDS: u32 = 2u;

struct Glass {
    // xy: pane centre; z: grain (0..1); w: glint (0..1).
    panes: array<vec4<f32>, 255>,
    // rgb: glass colour (linear); a: tier glow (0..1).
    tints: array<vec4<f32>, 255>,
    // rgb: light behind the window.
    light: vec4<f32>,
    // xy: half the screen size; z: lead width (px); w: glass texture (0..1).
    frame: vec4<f32>,
    // x: saturation; y: pane count.
    glass: vec4<f32>,
    // xy: spotlight centre; z: radius; w: strength.
    spot: vec4<f32>,
    // xy: grid origin; z: cell size; w: columns.
    grid_frame: vec4<f32>,
    // Pane numbers, one byte each, PER_CELL per cell; 255 ends a list.
    grid: array<vec4<u32>, 1280>,
}

@group(2) @binding(0) var<uniform> win: Glass;

struct FragmentIn {
    @location(0) world_position: vec4<f32>,
}

fn hash2(p: vec2<f32>) -> f32 {
    // No sin(): its precision falls off for large arguments (blocky noise on some GPUs).
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Smooth value noise.
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

fn rotate(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

@fragment
fn fragment(in: FragmentIn) -> @location(0) vec4<f32> {
    let p = in.world_position.xy;
    let n = min(u32(win.glass.y), MAX_PANES);
    let half = win.frame.xy;
    let lead_w = win.frame.z;
    let texture = win.frame.w;

    // The panes listed for this pixel's grid cell.
    let gf = win.grid_frame;
    let cols = u32(gf.w);
    let rows = u32(ceil(2.0 * half.y / gf.z));
    let cell = vec2<u32>(clamp(floor((p - gf.xy) / gf.z), vec2<f32>(0.0), vec2<f32>(f32(cols - 1u), f32(rows - 1u))));
    let base = (cell.y * cols + cell.x) * CELL_WORDS;
    var list: array<u32, 32>;
    var m = 0u;
    for (var w = 0u; w < CELL_WORDS * 4u; w++) {
        let word = win.grid[base + w / 4u][w % 4u];
        for (var b = 0u; b < 4u; b++) {
            let idx = (word >> (8u * b)) & 255u;
            if idx != 255u && idx < n {
                list[m] = idx;
                m++;
            }
        }
    }

    // 1. Which pane.
    var best = 1e20;
    var k = 0u;
    for (var t = 0u; t < m; t++) {
        let i = list[t];
        let d = p - win.panes[i].xy;
        let dd = dot(d, d);
        if dd < best {
            best = dd;
            k = i;
        }
    }
    let a = win.panes[k].xy;

    // 2. Distance to the nearest edge of this pane.
    var edge = 1e20;
    for (var t = 0u; t < m; t++) {
        let j = list[t];
        if j == k {
            continue;
        }
        let b = win.panes[j].xy;
        let ab = b - a;
        let len = length(ab);
        if len < 1e-3 {
            continue;
        }
        // Inside the cell the point is on a's side of every bisector.
        edge = min(edge, dot(0.5 * (a + b) - p, ab / len));
    }
    // The frame around the window is lead too, a little heavier.
    let border = min(half.x - abs(p.x), half.y - abs(p.y));
    let half_lead = 0.5 * lead_w;
    let to_lead = min(edge - half_lead, border - 1.5 * lead_w);
    let aa = max(fwidth(to_lead), 0.5);
    let glass_mask = smoothstep(-aa, aa, to_lead);

    // 3. Glass.
    let grain = win.panes[k].z;
    var tint = win.tints[k].rgb;
    let luma = dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722));
    tint = max(mix(vec3<f32>(luma), tint, win.glass.x), vec3<f32>(0.0));
    // Thickness: slow swells, different in every pane.
    let q = p + grain * 1000.0;
    let thick = noise(q * 0.008) * 0.5 + noise(q * 0.025) * 0.3 + noise(q * 0.07) * 0.2;
    // Streaks: drawn sheet glass, each pane cut at its own angle.
    let s = rotate(p, grain * 6.2832);
    let streak = noise(vec2<f32>(s.x * 0.004, s.y * 0.09) + grain * 37.0);
    // Thicker glass is deeper and darker: colour goes as tint^thickness.
    let depth = 1.0 + texture * (1.6 * (thick - 0.5) + 0.5 * (streak - 0.5));
    var glass = pow(max(tint, vec3<f32>(1e-4)), vec3<f32>(depth)) * (1.0 + 0.3 * texture * (0.5 - thick));
    // Light gathers behind the upper middle of the window.
    let r = (p - vec2<f32>(-0.15, 0.35) * half) / max(half.x, half.y);
    let still_glow = 0.25 + 1.7 * exp(-dot(r, r) * 2.2);
    // While someone sings, the light gathers into the spotlight, dancing.
    let ds = (p - win.spot.xy) / max(win.spot.z, 1.0);
    let spot_glow = 0.2 + 2.6 * exp(-dot(ds, ds) * 2.0);
    let spot_on = clamp(win.spot.w, 0.0, 1.0);
    let glow = mix(still_glow, spot_glow, spot_on) * (1.0 + 0.5 * max(win.spot.w - 1.0, 0.0));
    // Darker where the glass meets the lead (thicker edges, shadow of the came).
    let rim = mix(0.45, 1.0, smoothstep(0.0, 16.0, to_lead));
    // Light for this pane: its tier's glow, and a glint flaring it (bright
    // enough to bloom over its lead).
    let tier = win.tints[k].a;
    let glint = win.panes[k].w;
    let g2 = glint * glint;
    // Tiers average 0.5, so on average they leave the light as it is: a tier
    // above its share brightens, one below dims.
    glass = glass * win.light.rgb * glow * rim * (0.3 + 1.4 * tier) * (1.0 + 3.0 * g2);
    // The spotlight's centre is warm and a little white, as a lamp seen through glass.
    let spot = spot_on * exp(-dot(ds, ds) * 4.0);
    glass += win.light.rgb * vec3<f32>(1.0, 0.85, 0.6) * 0.2 * spot;
    // A glint is a flash of the light itself through the glass (near white,
    // even through deep blue), strongest in the middle of the pane.
    glass += win.light.rgb * g2 * 0.9 * smoothstep(0.0, 30.0, to_lead);

    // 4. Lead: a rounded came seen from the front, lit only by the room.
    let across = clamp(1.0 - max(-to_lead, 0.0) / max(half_lead, 0.5), 0.0, 1.0);
    let ridge = sqrt(max(1.0 - across * across, 0.0));
    let lead = vec3<f32>(0.0015 + 0.005 * pow(ridge, 8.0));

    return vec4<f32>(mix(lead, glass, glass_mask), 1.0);
}
