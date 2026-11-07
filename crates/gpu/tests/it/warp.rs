//! Free Transform's warp on the GPU agrees with the CPU (the self-check the app runs before using
//! it), and a large warp is faster there. Skips when no adapter exists; the Windows build runs it
//! on DX12's software adapter (WARP).

use photocraft_algo::transform::{Homography, Interp, WarpRequest, warp_surface_cpu, warp_surface_with_accelerator};
use photocraft_gpu::GpuWarp;

fn device() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    let required = std::env::var_os("WGPU_BACKEND").is_some();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let fallback = std::env::var_os("PHOTOCRAFT_TEST_FALLBACK_ADAPTER").is_some();
    let options = wgpu::RequestAdapterOptions { force_fallback_adapter: fallback, ..Default::default() };
    let adapter = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(a) => a,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but has no adapter: {e}");
            eprintln!("skipping the GPU warp test: no adapter ({e})");
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a device");
    Some((device, queue, adapter.get_info()))
}

#[test]
fn the_gpu_warp_agrees_with_the_cpu() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, info)) = device() else { return };
    let warp = GpuWarp::new(&device, &queue).unwrap_or_else(|e| panic!("the warp pipeline didn't build on {info:?}: {}", e.0));
    if let Err(e) = warp.self_check() {
        panic!("{} on {info:?}", e.0);
    }
}

#[test]
fn a_photo_sized_warp_through_the_gpu() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, info)) = device() else { return };
    let warp = std::sync::Arc::new(GpuWarp::new(&device, &queue).expect("pipeline"));
    let (w, h) = (4032usize, 3024usize);
    let mut src = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let px: Vec<f32> = (0..w * h * 4).map(|i| if i % 4 == 3 { 1.0 } else { ((i / 4 % w / 8 + i / 4 / w / 8 + i % 4 * 50) % 256) as f32 / 255.0 }).collect();
    let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
    src.write_region(r, &px);
    let gpu = warp.clone();
    let accel: photocraft_algo::transform::WarpAccelerator = std::sync::Arc::new(move |req: &WarpRequest<'_>| gpu.run(req).ok());
    for (name, k, a) in [("12 MP rotated 10°", 1.0f64, 0.17f64), ("12 MP to 63 % (fit a Letter page)", 0.63, 0.0), ("12 MP to 63 %, rotated", 0.63, 0.3)] {
        let hm = Homography([k * a.cos(), -k * a.sin(), 10.0, k * a.sin(), k * a.cos(), 5.0, 0.0, 0.0, 1.0]);
        let t = std::time::Instant::now();
        let out = warp_surface_with_accelerator(&src, r, &hm, Interp::Bicubic, &accel).expect("the GPU takes it");
        let gpu_ms = t.elapsed();
        let t = std::time::Instant::now();
        let cpu = warp_surface_cpu(&src, r, &hm, Interp::Bicubic).unwrap();
        let cpu_ms = t.elapsed();
        let (g, c) = (out.content_bounds(), cpu.content_bounds());
        assert!((g.x0 - c.x0).abs() <= 2 && (g.y0 - c.y0).abs() <= 2 && (g.x1 - c.x1).abs() <= 2 && (g.y1 - c.y1).abs() <= 2, "{name}: {g:?} vs {c:?}");
        eprintln!("GPU warp {name} on {}: GPU {gpu_ms:?}, CPU {cpu_ms:?}", info.name);
    }
}

