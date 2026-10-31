//! The compositor's pipelines build on the adapter there is. DX12's FXC rejected a shader that
//! naga and the other backends accepted ("X3507: not all control paths return a value"), so every
//! layer composited on the CPU on such machines. The Windows build runs this on DX12's software
//! adapter (WARP), which compiles with FXC. Skips when no adapter exists.

use photocraft_gpu::Compositor;

#[test]
fn the_compositor_builds_on_the_adapter() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // `WGPU_BACKEND` picks the backend; `PHOTOCRAFT_TEST_FALLBACK_ADAPTER=1` the software one (WARP).
    let required = std::env::var_os("WGPU_BACKEND").is_some();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let fallback = std::env::var_os("PHOTOCRAFT_TEST_FALLBACK_ADAPTER").is_some();
    let options = wgpu::RequestAdapterOptions { force_fallback_adapter: fallback, ..Default::default() };
    let adapter = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(a) => a,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but has no adapter: {e}");
            eprintln!("skipping the compositor build test: no adapter ({e})");
            return;
        }
    };
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a device");
    if let Err(e) = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)) {
        panic!("the compositor didn't build on {:?}: {}", adapter.get_info(), e.0);
    }
}
