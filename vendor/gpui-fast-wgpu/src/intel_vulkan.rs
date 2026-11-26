//! Open the compositor's Mesa Intel driver for this VkInstance only.
use crate::CompositorGpuHint;
use ash::{Entry, ext, khr, lunarg, vk};
use std::{env, path::Path};

pub(crate) fn eligible(hint: Option<CompositorGpuHint>) -> bool {
    if cfg!(debug_assertions) || !hint.is_some_and(|card| card.vendor_id == 0x8086) {
        return false;
    }
    // Respect driver, layer, offload and debugging choices made by the caller.
    if env::vars_os().any(|(name, _)| {
        let name = name.as_encoded_bytes();
        name.starts_with(b"VK_")
            || name.starts_with(b"MESA_")
            || name.starts_with(b"WGPU_")
            || matches!(
                name,
                b"DRI_PRIME"
                    | b"ZED_DEVICE_ID"
                    | b"PROTEUS_GPU_PROBE_ALL"
                    | b"__NV_PRIME_RENDER_OFFLOAD"
            )
    }) {
        return false;
    }
    let arch = env::consts::ARCH;
    let dir = Path::new("/usr/share/vulkan/icd.d");
    dir.join(format!("intel_icd.{arch}.json")).is_file() || dir.join("intel_icd.json").is_file()
}

pub(crate) fn instance() -> anyhow::Result<wgpu::Instance> {
    // SAFETY: load the system Vulkan loader and Mesa ICD using their published ABI.
    // The destruction callback owns both libraries until after vkDestroyInstance.
    let entry = unsafe { Entry::load()? };
    let driver = unsafe { libloading::Library::new("libvulkan_intel.so")? };
    let get_proc_addr =
        unsafe { *driver.get::<vk::PFN_vkGetInstanceProcAddr>(b"vk_icdGetInstanceProcAddr\0")? };
    let drivers = [vk::DirectDriverLoadingInfoLUNARG::default()
        .pfn_get_instance_proc_addr(Some(get_proc_addr))];
    let mut select = vk::DirectDriverLoadingListLUNARG::default()
        .mode(vk::DirectDriverLoadingModeLUNARG::EXCLUSIVE)
        .drivers(&drivers);

    // Superset of wgpu-hal 30's release Linux WSI requirements (no DRM backend).
    // Asking the loader to enumerate extensions would initialize every installed
    // ICD. vkCreateInstance itself checks support; any missing extension falls
    // back to wgpu's regular instance creation.
    let extensions = vec![
        khr::surface::NAME,
        khr::xlib_surface::NAME,
        khr::xcb_surface::NAME,
        khr::wayland_surface::NAME,
        khr::get_physical_device_properties2::NAME,
        ext::swapchain_colorspace::NAME,
        lunarg::direct_driver_loading::NAME,
    ];
    let pointers: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
    let app = vk::ApplicationInfo::default()
        .application_name(c"Proteus")
        .engine_name(c"wgpu-hal")
        .engine_version(2)
        .api_version(vk::API_VERSION_1_3);
    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&pointers)
        .push_next(&mut select);
    // SAFETY: all pointers in create_info remain live until vkCreateInstance returns.
    let raw = unsafe { entry.create_instance(&create_info, None)? };
    let owned = raw.clone();
    let loader = entry.clone();
    let destroy: wgpu::hal::DropCallback = Box::new(move || {
        // SAFETY: this is the sole owner of VkInstance destruction. HAL calls
        // this after releasing dependent Vulkan objects.
        unsafe { owned.destroy_instance(None) };
        drop(driver);
        drop(loader);
    });
    // SAFETY: raw was created with this entry, API version and every extension
    // supplied below. This path only runs in release, without validation layers.
    let hal = unsafe {
        wgpu::hal::vulkan::Instance::from_raw(
            entry,
            raw,
            vk::API_VERSION_1_3,
            0,
            None,
            extensions,
            wgpu::InstanceFlags::default(),
            wgpu::MemoryBudgetThresholds::default(),
            false,
            Some(destroy),
        )
    }?;
    Ok(unsafe { wgpu::Instance::from_hal::<wgpu::hal::api::Vulkan>(hal) })
}
