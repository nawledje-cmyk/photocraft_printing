// Free Transform's affine warp on the GPU (photocraft-algo `warp_surface`, which this matches
// within about 1/255): each destination pixel samples the straight-alpha RGBA8 source,
// premultiplied, with a Bilinear or Bicubic (A = -0.75) kernel; edges repeat; the outline is
// antialiased by coverage. Below 100 % the bicubic kernel widens by 1 / scale, as the CPU's
// reduction before its warp does.

struct Params {
    // Source position of destination pixel (x, y) of this band: (inv0.x*x + inv0.y*y + inv0.z,
    // inv1.x*x + inv1.y*y + inv1.z), relative to the source's corner.
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    // Source width, height; destination width; first destination row of this band.
    size: vec4<u32>,
    // Scale (dst per src, at most 1), 1 for bicubic, the pixel's extent in source px (u, v).
    misc: vec4<f32>,
    // Rows in this band.
    band: vec4<u32>,
    // The outline in reduced source pixels: offset (x, y) and size (w, h); see `WarpRequest::edge`.
    edge: vec4<f32>,
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

// Past the source, a reduction reads transparency (as the CPU's reduction does before its warp);
// otherwise the edge repeats.
fn texel_or_clear(x: i32, y: i32) -> vec4<f32> {
    if x < 0 || y < 0 || x >= i32(p.size.x) || y >= i32(p.size.y) {
        return vec4<f32>(0.0);
    }
    return texel(x, y);
}

fn texel(x: i32, y: i32) -> vec4<f32> {
    let sw = i32(p.size.x);
    let sh = i32(p.size.y);
    let cx = clamp(x, 0, sw - 1);
    let cy = clamp(y, 0, sh - 1);
    let c = unpack4x8unorm(src[u32(cy) * u32(sw) + u32(cx)]);
    return vec4<f32>(c.rgb * c.a, c.a);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dw = p.size.z;
    if id.x >= dw || id.y >= p.band.x {
        return;
    }
    let x = f32(id.x);
    let y = f32(id.y + p.size.w);
    let out = (id.y + p.size.w) * dw + id.x;
    let u = p.inv0.x * x + p.inv0.y * y + p.inv0.z;
    let v = p.inv1.x * x + p.inv1.y * y + p.inv1.z;
    // The position in reduced source pixels (the source itself when nothing is reduced), where the
    // CPU tests the outline.
    let k = select(1.0, p.misc.x, p.misc.y > 0.5);
    let ur = (u + 0.5) * k - 0.5 + p.edge.x;
    let vr = (v + 0.5) * k - 0.5 + p.edge.y;
    if ur < -1.0 || vr < -1.0 || ur > p.edge.z || vr > p.edge.w {
        dst[out] = 0u;
        return;
    }
    var acc = vec4<f32>(0.0);
    if p.misc.y > 0.5 {
        // Bicubic, widened by 1 / scale below 100 %.
        let s = p.misc.x;
        let r = 2.0 / s;
        let x0 = i32(ceil(u - r));
        let x1 = i32(floor(u + r));
        let y0 = i32(ceil(v - r));
        let y1 = i32(floor(v + r));
        var wsum_x = 0.0;
        for (var i = x0; i <= x1; i++) {
            wsum_x += cubic((f32(i) - u) * s);
        }
        var wsum_y = 0.0;
        for (var j = y0; j <= y1; j++) {
            wsum_y += cubic((f32(j) - v) * s);
        }
        let norm = 1.0 / max(wsum_x * wsum_y, 1e-6);
        for (var j = y0; j <= y1; j++) {
            let wy = cubic((f32(j) - v) * s);
            if wy == 0.0 {
                continue;
            }
            var row = vec4<f32>(0.0);
            for (var i = x0; i <= x1; i++) {
                let wx = cubic((f32(i) - u) * s);
                if wx != 0.0 {
                    row += select(texel(i, j), texel_or_clear(i, j), s < 0.999) * wx;
                }
            }
            acc += row * wy;
        }
        acc *= norm;
    } else {
        let ix = i32(floor(u));
        let iy = i32(floor(v));
        let fx = u - floor(u);
        let fy = v - floor(v);
        acc = (texel(ix, iy) * (1.0 - fx) + texel(ix + 1, iy) * fx) * (1.0 - fy)
            + (texel(ix, iy + 1) * (1.0 - fx) + texel(ix + 1, iy + 1) * fx) * fy;
    }
    // How much of this pixel the warped source covers, per axis.
    let uc = ur + 0.5;
    let vc = vr + 0.5;
    let cu = clamp(min(uc, p.edge.z - uc) / (p.misc.z * k) + 0.5, 0.0, 1.0);
    let cv = clamp(min(vc, p.edge.w - vc) / (p.misc.w * k) + 0.5, 0.0, 1.0);
    acc *= cu * cv;
    let al = clamp(acc.a, 0.0, 1.0);
    if al <= 0.0 {
        dst[out] = 0u;
        return;
    }
    // Un-premultiplied by the unclamped alpha, as the CPU does (bicubic overshoot).
    let rgb = clamp(acc.rgb / acc.a, vec3<f32>(0.0), vec3<f32>(1.0));
    let q = vec4<u32>(vec4<f32>(rgb, al) * 255.0 + 0.5);
    dst[out] = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}
