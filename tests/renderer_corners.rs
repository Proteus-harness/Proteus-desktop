//! Validate the exact shader variants consumed by the patched renderer.

#[test]
fn shared_corner_geometry_validates_for_every_shader_transport() {
    let shared = include_str!("../vendor/gpui-fast-wgpu/src/shaders.wgsl");
    let storage = include_str!("../vendor/gpui-fast-wgpu/src/shaders_storage.wgsl");
    let webgl = include_str!("../vendor/gpui-fast-wgpu/src/shaders_webgl.wgsl");
    let subpixel = include_str!("../vendor/gpui-fast-wgpu/src/shaders_subpixel.wgsl");
    for (name, source) in [
        ("storage", format!("{shared}\n{storage}")),
        ("webgl", format!("{shared}\n{webgl}")),
        (
            "subpixel",
            format!("enable dual_source_blending;\n{shared}\n{storage}\n{subpixel}"),
        ),
    ] {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    }
}
