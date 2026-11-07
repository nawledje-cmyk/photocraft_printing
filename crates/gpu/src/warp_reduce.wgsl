// The reduction before Free Transform's warp when it shrinks (photocraft-algo `resize_surface`'s
// Bicubic pass on one axis): each output sample filters the straight-alpha RGBA8 input,
// premultiplied, with the Bicubic kernel (A = -0.75) widened by 1 / f; weights are normalized over
// every tap, and taps past the input read transparency. The result is stored straight, in 8 bits,
// as the CPU stores it between passes.

struct Params {
    // Input width, height; output width, height.
    size: vec4<u32>,
    // Input origin and output origin along the axis (document pixels), f (output per input), axis
    // (0 = x, 1 = y).
    axis: vec4<f32>,
    // Output rows in this band, and its first row.
    band: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> p: Params;

const A: f32 = -0.75;

fn cubic(x: f32) -> f32 {
    let t = abs(x);
    if t < 1.0 {
        return ((A + 2.0) * t - (A + 3.0)) * t * t + 1.0;
    }
    if t < 2.0 {
        return A * (((t - 5.0) * t + 8.0) * t - 4.0);
    }
    return 0.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let ow = p.size.z;
    if id.x >= ow || id.y >= p.band.x {
        return;
    }
    let ox = id.x;
    let oy = id.y + p.band.y;
    let f = p.axis.z;
    let horizontal = p.axis.w < 0.5;
    let o = select(f32(oy), f32(ox), horizontal);
    // The input position (from the input's origin) this sample centres on.
    let c = (p.axis.y + o + 0.5) / f - 0.5 - p.axis.x;
    let r = 2.0 / f;
    let i0 = i32(ceil(c - r));
    let i1 = i32(floor(c + r));
    let n = i32(select(p.size.y, p.size.x, horizontal));
    var acc = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = i0; i <= i1; i++) {
        let w = cubic((f32(i) - c) * f);
        total += w;
        if w == 0.0 || i < 0 || i >= n {
            continue;
        }
        let at = select(u32(i) * p.size.x + ox, oy * p.size.x + u32(i), horizontal);
        let t = unpack4x8unorm(src[at]);
        acc += vec4<f32>(t.rgb * t.a, t.a) * w;
    }
    acc /= select(total, 1.0, total == 0.0);
    let al = clamp(acc.a, 0.0, 1.0);
    var out = 0u;
    if al > 0.0 {
        let rgb = clamp(acc.rgb / acc.a, vec3<f32>(0.0), vec3<f32>(1.0));
        let q = vec4<u32>(vec4<f32>(rgb, al) * 255.0 + 0.5);
        out = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
    }
    dst[oy * ow + ox] = out;
}
