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
        // Bicubic at 1:1 (a reduction is done before this pass): the 4 × 4 taps around the
        // sample, with the CPU's weights, which sum to 1.
        let ix = i32(floor(u));
        let iy = i32(floor(v));
        let fx = u - floor(u);
        let fy = v - floor(v);
        let wx = vec4<f32>(cubic(1.0 + fx), cubic(fx), cubic(1.0 - fx), cubic(2.0 - fx));
        let wy = vec4<f32>(cubic(1.0 + fy), cubic(fy), cubic(1.0 - fy), cubic(2.0 - fy));
        for (var j = 0; j < 4; j++) {
            let yy = iy - 1 + j;
            let row = texel(ix - 1, yy) * wx.x + texel(ix, yy) * wx.y + texel(ix + 1, yy) * wx.z + texel(ix + 2, yy) * wx.w;
            acc += row * wy[j];
        }
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
