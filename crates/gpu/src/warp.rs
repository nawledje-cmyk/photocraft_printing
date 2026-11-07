//! Free Transform's affine warp on the GPU: when it shrinks, the source is first reduced in two
//! passes as the CPU reduces it (`warp_reduce.wgsl`, x then y, stored in 8 bits between passes),
//! then warped 1:1 (`warp.wgsl`), one compute pass per band of destination rows, over storage
//! buffers, and read back as RGBA8. The app installs it as `photocraft_algo::transform`'s warp
//! accelerator after [`GpuWarp::self_check`] agrees with the CPU; every error (limits, a lost
//! device, a failed check) declines, and the CPU warps instead.

use std::num::NonZeroU64;

use crate::Unsupported;
use photocraft_algo::transform::WarpRequest;

pub const WARP_SHADER: &str = include_str!("warp.wgsl");
pub const WARP_REDUCE_SHADER: &str = include_str!("warp_reduce.wgsl");

/// Rows per dispatch, so no single submission runs long enough to trip a driver's watchdog.
const BAND_ROWS: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    inv0: [f32; 4],
    inv1: [f32; 4],
    size: [u32; 4],
    misc: [f32; 4],
    band: [u32; 4],
    edge: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ReduceParams {
    size: [u32; 4],
    axis: [f32; 4],
    band: [u32; 4],
}

/// The warp pipelines on one device.
pub struct GpuWarp {
    device: wgpu::Device,
    queue: wgpu::Queue,
    warp: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    reduce: (wgpu::ComputePipeline, wgpu::BindGroupLayout),
}

fn pipeline(device: &wgpu::Device, label: &str, source: &str, uniform: u64) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
    let storage = |read_only: bool| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            entry(0, storage(true)),
            entry(1, storage(false)),
            entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(uniform) }),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (pipeline, layout)
}

/// What one pass reads and writes, and how it is dispatched.
#[cfg(not(target_arch = "wasm32"))]
struct Pass<'a> {
    stage: &'a (wgpu::ComputePipeline, wgpu::BindGroupLayout),
    input: &'a wgpu::Buffer,
    output: &'a wgpu::Buffer,
    width: u32,
    height: u32,
}

impl GpuWarp {
    /// Builds the pipelines on `device` (compiles the shaders).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, Unsupported> {
        let (built, error) = crate::paging::first_error(device, || {
            let warp = pipeline(device, "pc_warp", WARP_SHADER, std::mem::size_of::<Params>() as u64);
            let reduce = pipeline(device, "pc_warp_reduce", WARP_REDUCE_SHADER, std::mem::size_of::<ReduceParams>() as u64);
            (warp, reduce)
        });
        if let Some(e) = error {
            return Err(Unsupported(format!("the GPU warp pipelines failed to build: {e}")));
        }
        let (warp, reduce) = built;
        Ok(Self { device: device.clone(), queue: queue.clone(), warp, reduce })
    }

    /// Runs `req` and reads the result back (blocking; native only). `Err` when the device can't
    /// (limits, errors, a lost device): the caller warps on the CPU.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(&self, req: &WarpRequest<'_>) -> Result<Vec<u8>, Unsupported> {
        let (out, error) = crate::paging::first_error(&self.device, || self.run_inner(req));
        match error {
            Some(e) => Err(Unsupported(format!("the GPU warp failed: {e}"))),
            None => out,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn buffer(&self, label: &str, bytes: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage, mapped_at_creation: false })
    }

    /// Runs `pass` in bands of [`BAND_ROWS`], each with its own uniform (`params(first, rows)`).
    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch(&self, pass: &Pass<'_>, params: &dyn Fn(u32, u32) -> Vec<u8>, last: &mut Option<wgpu::SubmissionIndex>) -> Result<(), Unsupported> {
        let groups_x = pass.width.div_ceil(8);
        if groups_x > self.device.limits().max_compute_workgroups_per_dimension {
            return Err(Unsupported("the warp is too wide for the GPU".into()));
        }
        let mut row = 0;
        while row < pass.height {
            let rows = BAND_ROWS.min(pass.height - row);
            let bytes = params(row, rows);
            let uniform = self.buffer("pc_warp_params", bytes.len() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            self.queue.write_buffer(&uniform, 0, &bytes);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pc_warp"),
                layout: &pass.stage.1,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: pass.input.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: pass.output.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                ],
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_warp") });
            {
                let mut cp = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pc_warp"), timestamp_writes: None });
                cp.set_pipeline(&pass.stage.0);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups_x, rows.div_ceil(8), 1);
            }
            *last = Some(self.queue.submit([encoder.finish()]));
            row += rows;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run_inner(&self, req: &WarpRequest<'_>) -> Result<Vec<u8>, Unsupported> {
        let (sw, sh) = (req.src_rect.width(), req.src_rect.height());
        let (dw, dh) = (req.dst.width(), req.dst.height());
        let bytes = |w: u32, h: u32| u64::from(w) * u64::from(h) * 4;
        let limits = self.device.limits();
        let max = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let fits = |w: u32, h: u32| w > 0 && h > 0 && bytes(w, h) <= max;
        if !fits(sw, sh) || !fits(dw, dh) || req.src.len() as u64 != bytes(sw, sh) {
            return Err(Unsupported(format!("a {sw}×{sh} → {dw}×{dh} warp doesn't fit the GPU's {max}-byte buffers")));
        }
        let f = |v: f64| v as f32;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let source = self.buffer("pc_warp_src", bytes(sw, sh), storage);
        self.queue.write_buffer(&source, 0, req.src);
        let mut last = None;
        // Shrinking (Bicubic): reduce by `k` first, x then y, as the CPU does; then warp from the
        // reduced image at 1:1, its coordinates `k` times the source's.
        let k = if req.bicubic && req.scale < 1.0 { req.scale.max(1.0 / 64.0) } else { 1.0 };
        let [a, b, c, d, e, g] = req.inv;
        let (input, iw, ih, inv, foot, edge) = if k < 1.0 {
            let (rw, rh) = (req.edge[2].round() as u32, req.edge[3].round() as u32);
            if !fits(rw, sh) || !fits(rw, rh) {
                return Err(Unsupported("the reduced warp doesn't fit the GPU's buffers".into()));
            }
            // The reduced image's corner, in document pixels.
            let (rx0, ry0) = (f64::from(req.src_rect.x0) * k - req.edge[0], f64::from(req.src_rect.y0) * k - req.edge[1]);
            let across = self.buffer("pc_warp_reduce_x", bytes(rw, sh), storage);
            let reduced = self.buffer("pc_warp_reduce_y", bytes(rw, rh), storage);
            let axis = |input_origin: f64, output_origin: f64, vertical: bool| [f(input_origin), f(output_origin), f(k), if vertical { 1.0 } else { 0.0 }];
            let x_axis = axis(f64::from(req.src_rect.x0), rx0, false);
            let horizontal = |row: u32, rows: u32| bytemuck::bytes_of(&ReduceParams { size: [sw, sh, rw, sh], axis: x_axis, band: [rows, row, 0, 0] }).to_vec();
            self.dispatch(&Pass { stage: &self.reduce, input: &source, output: &across, width: rw, height: sh }, &horizontal, &mut last)?;
            let y_axis = axis(f64::from(req.src_rect.y0), ry0, true);
            let vertical = |row: u32, rows: u32| bytemuck::bytes_of(&ReduceParams { size: [rw, sh, rw, rh], axis: y_axis, band: [rows, row, 0, 0] }).to_vec();
            self.dispatch(&Pass { stage: &self.reduce, input: &across, output: &reduced, width: rw, height: rh }, &vertical, &mut last)?;
            // u_reduced = k·(u + 0.5 + x0) − 0.5 − rx0, from the reduced image's corner.
            let (cx, cy) = (k * 0.5 - 0.5 + req.edge[0], k * 0.5 - 0.5 + req.edge[1]);
            let inv = [a * k, b * k, c * k + cx, d * k, e * k, g * k + cy];
            (reduced, rw, rh, inv, [req.foot[0] * k, req.foot[1] * k], [0.0, 0.0, f64::from(rw), f64::from(rh)])
        } else {
            (source, sw, sh, req.inv, req.foot, req.edge)
        };
        let target = self.buffer("pc_warp_dst", bytes(dw, dh), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let warp = |row: u32, rows: u32| {
            bytemuck::bytes_of(&Params {
                inv0: [f(inv[0]), f(inv[1]), f(inv[2]), 0.0],
                inv1: [f(inv[3]), f(inv[4]), f(inv[5]), 0.0],
                size: [iw, ih, dw, row],
                misc: [1.0, if req.bicubic { 1.0 } else { 0.0 }, f(foot[0]), f(foot[1])],
                band: [rows, 0, 0, 0],
                edge: edge.map(f),
            })
            .to_vec()
        };
        self.dispatch(&Pass { stage: &self.warp, input: &input, output: &target, width: dw, height: dh }, &warp, &mut last)?;
        let readback = self.buffer("pc_warp_readback", bytes(dw, dh), wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_warp_readback") });
        encoder.copy_buffer_to_buffer(&target, 0, &readback, 0, bytes(dw, dh));
        last = Some(self.queue.submit([encoder.finish()]));
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::Wait { submission_index: last, timeout: Some(std::time::Duration::from_secs(20)) }).map_err(|e| Unsupported(format!("GPU wait failed: {e}")))?;
        let out = {
            let data = readback.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU readback failed: {e:?}")))?;
            data.to_vec()
        };
        readback.unmap();
        Ok(out)
    }

    /// Warps a noisy test image on the GPU and on the CPU, enlarged, rotated and reduced, and says
    /// whether they agree: inside the picture within a few levels (more when reduced, where the CPU
    /// reduces in two passes first), and the same outline within a pixel or two. `Err` names the
    /// disagreement.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn self_check(&self) -> Result<(), Unsupported> {
        use photocraft_algo::transform::{Homography, Interp, warp_surface_cpu};
        use photocraft_color::PixelFormat;
        use photocraft_geom::Rect;
        let (w, h) = (97, 61);
        let px: Vec<f32> = (0..w * h * 4)
            .map(|i| {
                let (pixel, c) = (i / 4, i % 4);
                if c == 3 { if pixel % 11 == 0 { 0.4 } else { 1.0 } } else { ((pixel * 37 + c * 91) % 256) as f32 / 255.0 }
            })
            .collect();
        let mut src = photocraft_raster::Surface::new(PixelFormat::RGBA8);
        let src_rect = Rect::new(3, 5, 3 + w, 5 + h);
        src.write_region(src_rect, &px);
        let affine = |k: f64, a: f64| Homography([k * a.cos(), -k * a.sin(), 40.0, k * a.sin(), k * a.cos(), 20.0, 0.0, 0.0, 1.0]);
        // Reduced, both reduce in two 8-bit passes, then warp; the CPU's 1/128-pixel tap phases
        // move single pixels of this noise (the worst case) a little more.
        for (name, hm, interp, max_tol, mean_tol) in [
            ("bicubic 1.7× rotated", affine(1.7, 0.4), Interp::Bicubic, 3.0, 1.0),
            ("bilinear 1.3×", affine(1.3, 0.0), Interp::Bilinear, 2.0, 1.0),
            ("bicubic 0.6× rotated", affine(0.6, -0.3), Interp::Bicubic, 24.0, 3.0),
        ] {
            let cpu = warp_surface_cpu(&src, src_rect, &hm, interp).map_err(|e| Unsupported(e.to_string()))?;
            let req = request(&src, src_rect, &hm, interp).ok_or_else(|| Unsupported(format!("{name}: no request")))?;
            let gpu = self.run(&req.as_request())?;
            let dst = req.dst;
            let (dw, dh) = (dst.width() as usize, dst.height() as usize);
            let mut want = vec![[0u8; 4]; dw * dh];
            cpu.read_rgba8_into(dst, &mut want);
            let got: Vec<[u8; 4]> = gpu.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            // Inside the picture (covered by both 2 px around): every channel within `max_tol`.
            // The 1–2 px antialiased outline may differ (the CPU reduces first, then warps).
            let opaque = |v: &[[u8; 4]], x: usize, y: usize| v.get(y * dw + x).is_some_and(|p| p[3] > 0);
            let (mut worst, mut inside, mut sum) = (0.0f64, 0usize, 0.0f64);
            for y in 2..dh.saturating_sub(2) {
                for x in 2..dw.saturating_sub(2) {
                    let around = (y - 2..=y + 2).all(|yy| (x - 2..=x + 2).all(|xx| opaque(&want, xx, yy) && opaque(&got, xx, yy)));
                    if !around {
                        continue;
                    }
                    inside += 1;
                    let (g, c) = (got[y * dw + x], want[y * dw + x]);
                    for k in 0..4 {
                        let d = (f64::from(g[k]) - f64::from(c[k])).abs();
                        worst = worst.max(d);
                        sum += d;
                    }
                }
            }
            // The outline: the opaque area is the same within a pixel or two.
            let count = |v: &[[u8; 4]]| v.iter().filter(|p| p[3] >= 128).count() as f64;
            let (a, b) = (count(&got), count(&want));
            let perimeter = 2.0 * (dw + dh) as f64;
            let mean = sum / (inside.max(1) as f64 * 4.0);
            if inside < 100 || worst > max_tol || mean > mean_tol || (a - b).abs() > 2.0 * perimeter {
                return Err(Unsupported(format!(
                    "the GPU warp disagrees with the CPU ({name}: up to {worst} levels inside, {mean:.2} on average, {inside} pixels compared, opaque area {a} vs {b})"
                )));
            }
        }
        Ok(())
    }
}

/// An owned [`WarpRequest`] (for the self-check).
#[cfg(not(target_arch = "wasm32"))]
struct OwnedRequest {
    src: Vec<u8>,
    src_rect: photocraft_geom::Rect,
    dst: photocraft_geom::Rect,
    inv: [f64; 6],
    scale: f64,
    bicubic: bool,
    foot: [f64; 2],
    edge: [f64; 4],
}

#[cfg(not(target_arch = "wasm32"))]
impl OwnedRequest {
    fn as_request(&self) -> WarpRequest<'_> {
        WarpRequest { src: &self.src, src_rect: self.src_rect, dst: self.dst, inv: self.inv, scale: self.scale, bicubic: self.bicubic, foot: self.foot, edge: self.edge }
    }
}

/// The request `warp_surface` would make for this warp (the same arithmetic).
#[cfg(not(target_arch = "wasm32"))]
fn request(
    src: &photocraft_raster::Surface,
    src_rect: photocraft_geom::Rect,
    h: &photocraft_algo::transform::Homography,
    interp: photocraft_algo::transform::Interp,
) -> Option<OwnedRequest> {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = captured.clone();
    let probe: photocraft_algo::transform::WarpAccelerator = std::sync::Arc::new(move |r: &WarpRequest<'_>| {
        *sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(OwnedRequest { src: r.src.to_vec(), src_rect: r.src_rect, dst: r.dst, inv: r.inv, scale: r.scale, bicubic: r.bicubic, foot: r.foot, edge: r.edge });
        None
    });
    let _ = photocraft_algo::transform::warp_surface_with_accelerator(src, src_rect, h, interp, &probe);
    captured.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}
