//! Monitor composite on wgpu. The CPU path in `draw` stays the reference and the fallback.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;

use bytemuck::Zeroable;
use oc_timeline::{
    AlphaShape, CubeLut, CurvePoint, FrameCard, Generator, Lut, MaskShape, TransitionKind,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::Surface;
use crate::draw::{pan_px, parse_hex};
use crate::{FramePlan, FrameSource, Layer};

const MAX_LAYERS: usize = 8;
const MAX_PICTURES: usize = 4;
const MAX_CUBES: usize = 2;
const MAX_CURVE_POINTS: usize = 8;

pub enum MonitorPath {
    /// A WebGPU device is ready. Paint with [`present_monitor`].
    Gpu,
    /// The adapter probe is still running. Leave the canvas alone.
    Waiting,
    /// No WebGPU device. Paint with the CPU compositor.
    Cpu,
}

/// `shift` is the stabilize offset in frame pixels: output `(x, y)` shows input `(x - dx, y - dy)`.
pub fn present_monitor(
    canvas: &web_sys::HtmlCanvasElement,
    plan: &FramePlan,
    sources: &[FrameSource],
    cubes: &[CubeLut],
    shift: (i32, i32),
) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_present(canvas, plan, sources, cubes, shift)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (canvas, plan, sources, cubes, shift);
        false
    }
}

pub fn monitor_path() -> MonitorPath {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_monitor_path()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        MonitorPath::Cpu
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuFrame {
    size: [f32; 4],
    bg: [f32; 4],
    shift: [f32; 4],
    cubes: [f32; 4],
    tex: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLayer {
    rect: [f32; 4],
    xform: [f32; 4],
    grade: [f32; 4],
    grade2: [f32; 4],
    fx: [f32; 4],
    mask: [f32; 4],
    mask2: [f32; 4],
    crop: [f32; 4],
    flags: [f32; 4],
    extra: [f32; 4],
    slide: [f32; 4],
    genb: [f32; 4],
    counts: [f32; 4],
    curves: [[f32; 4]; 16],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuScene {
    frame: GpuFrame,
    layers: [GpuLayer; MAX_LAYERS],
}

struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct CubeUpload {
    size: u32,
    rgba: Vec<u8>,
}

struct Packed {
    scene: GpuScene,
    pictures: Vec<Picture>,
    cubes: Vec<CubeUpload>,
}

struct Engine {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    instance: wgpu::Instance,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    bind_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

enum BootError {
    NoAdapter(String),
    Fault(String),
}

impl BootError {
    #[cfg(target_arch = "wasm32")]
    fn text(&self) -> &str {
        match self {
            Self::NoAdapter(text) | Self::Fault(text) => text,
        }
    }
}

impl Engine {
    async fn boot() -> Result<Self, BootError> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(|err| BootError::NoAdapter(err.to_string()))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|err| BootError::NoAdapter(err.to_string()))?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[
                buffer_entry(0),
                texture_entry(1, wgpu::TextureViewDimension::D2),
                texture_entry(2, wgpu::TextureViewDimension::D2),
                texture_entry(3, wgpu::TextureViewDimension::D2),
                texture_entry(4, wgpu::TextureViewDimension::D2),
                texture_entry(5, wgpu::TextureViewDimension::D3),
                texture_entry(6, wgpu::TextureViewDimension::D3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let mut engine = Self {
            instance,
            adapter,
            device,
            queue,
            shader,
            bind_layout,
            pipeline_layout,
            pipelines: Vec::new(),
        };
        engine
            .ensure_pipeline(wgpu::TextureFormat::Rgba8Unorm)
            .await?;
        Ok(engine)
    }

    fn build_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("composite"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    async fn ensure_pipeline(&mut self, format: wgpu::TextureFormat) -> Result<(), BootError> {
        if self.pipelines.iter().any(|(have, _)| *have == format) {
            return Ok(());
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = self.build_pipeline(format);
        if let Some(err) = scope.pop().await {
            return Err(BootError::Fault(err.to_string()));
        }
        self.pipelines.push((format, pipeline));
        Ok(())
    }

    fn ensure_format(&mut self, format: wgpu::TextureFormat) {
        if self.pipelines.iter().any(|(have, _)| *have == format) {
            return;
        }
        let pipeline = self.build_pipeline(format);
        self.pipelines.push((format, pipeline));
    }

    fn pipeline(&self, format: wgpu::TextureFormat) -> Result<&wgpu::RenderPipeline, String> {
        self.pipelines
            .iter()
            .find(|(have, _)| *have == format)
            .map(|(_, pipeline)| pipeline)
            .ok_or_else(|| format!("no pipeline for {format:?}"))
    }

    fn paint(
        &mut self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        plan: &FramePlan,
        sources: &[FrameSource],
        cubes: &[CubeLut],
        shift: (i32, i32),
    ) -> Result<(), String> {
        let packed = prepare(plan, sources, cubes, shift);
        self.ensure_format(format);
        let pipeline = self.pipeline(format)?.clone();
        let mut pictures = [None, None, None, None];
        let mut picture_views = Vec::with_capacity(MAX_PICTURES);
        for slot in 0..MAX_PICTURES {
            let picture = packed.pictures.get(slot);
            let (width, height, rgba) = match picture {
                Some(picture) => (
                    picture.width.max(1),
                    picture.height.max(1),
                    picture.rgba.as_slice(),
                ),
                None => (1, 1, &[0, 0, 0, 255][..]),
            };
            let texture = self.texture_2d(width, height, rgba)?;
            picture_views.push(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            pictures[slot] = Some(texture);
        }
        let mut cube_keep = [None, None];
        let mut cube_views = Vec::with_capacity(MAX_CUBES);
        for slot in 0..MAX_CUBES {
            let upload = packed.cubes.get(slot);
            let (size, rgba) = match upload {
                Some(cube) => (cube.size.max(1), cube.rgba.as_slice()),
                None => (1, &[0, 0, 0, 255][..]),
            };
            let texture = self.texture_3d(size, rgba)?;
            cube_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D3),
                ..Default::default()
            }));
            cube_keep[slot] = Some(texture);
        }
        let _ = (pictures, cube_keep);
        let scene_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: std::mem::size_of::<GpuScene>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&scene_buffer, 0, bytemuck::bytes_of(&packed.scene));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&picture_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&picture_views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&picture_views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&picture_views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&cube_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&cube_views[1]),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn texture_2d(&self, width: u32, height: u32, rgba: &[u8]) -> Result<wgpu::Texture, String> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &padded_rgba(rgba, width, height),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(texture)
    }

    fn texture_3d(&self, size: u32, rgba: &[u8]) -> Result<wgpu::Texture, String> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cube"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.saturating_mul(4)),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
        );
        Ok(texture)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn render_rgba(
        &mut self,
        plan: &FramePlan,
        sources: &[FrameSource],
        cubes: &[CubeLut],
        shift: (i32, i32),
    ) -> Result<Surface, String> {
        let width = plan.width.max(2);
        let height = plan.height.max(2);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("readback"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.paint(
            &view,
            wgpu::TextureFormat::Rgba8Unorm,
            plan,
            sources,
            cubes,
            shift,
        )?;
        let padded = align_row(width * 4);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|err| err.to_string())?;
        recv.recv()
            .map_err(|_| "map dropped".to_string())?
            .map_err(|err| err.to_string())?;
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|err| err.to_string())?;
        let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
        for y in 0..height as usize {
            let src = y * padded as usize;
            let dst = y * width as usize * 4;
            let row = (width as usize) * 4;
            if src + row <= mapped.len() && dst + row <= rgba.len() {
                rgba[dst..dst + row].copy_from_slice(&mapped[src..src + row]);
            }
        }
        drop(mapped);
        readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(err.to_string());
        }
        Ok(Surface {
            width,
            height,
            rgba,
        })
    }
}

fn buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<GpuScene>() as u64),
        },
        count: None,
    }
}

fn texture_entry(binding: u32, view: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: view,
            multisampled: false,
        },
        count: None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn align_row(bytes: u32) -> u32 {
    bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn padded_rgba(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = (width as usize) * 4;
    let need = row * height as usize;
    if rgba.len() >= need {
        return rgba[..need].to_vec();
    }
    let mut out = vec![0u8; need];
    out[..rgba.len()].copy_from_slice(rgba);
    out
}

fn prepare(
    plan: &FramePlan,
    sources: &[FrameSource],
    cubes: &[CubeLut],
    shift: (i32, i32),
) -> Packed {
    pack(plan, sources, cubes, shift).unwrap_or_else(|| cpu_blit(plan, sources, cubes, shift))
}

fn cpu_blit(
    plan: &FramePlan,
    sources: &[FrameSource],
    cubes: &[CubeLut],
    shift: (i32, i32),
) -> Packed {
    let cpu = crate::composite(plan, sources, cubes);
    let width = cpu.width.max(2);
    let height = cpu.height.max(2);
    let mut scene = GpuScene::zeroed();
    scene.frame.size = [width as f32, height as f32, 1.0, 0.0];
    scene.frame.shift = [shift.0 as f32, shift.1 as f32, 0.0, 0.0];
    scene.frame.tex[0] = [width as f32, height as f32, 0.0, 0.0];
    let layer = &mut scene.layers[0];
    layer.rect = [0.0, 0.0, width as f32, height as f32];
    layer.xform = [0.0, 0.0, 0.0, 1.0];
    layer.crop = [0.0, 0.0, 1.0, 1.0];
    layer.fx = [0.0, 0.0, 1.0, 0.0];
    layer.extra[1] = -1.0;
    layer.genb[3] = 1.0;
    Packed {
        scene,
        pictures: vec![Picture {
            width,
            height,
            rgba: cpu.rgba,
        }],
        cubes: Vec::new(),
    }
}

fn pack(
    plan: &FramePlan,
    sources: &[FrameSource],
    cubes: &[CubeLut],
    shift: (i32, i32),
) -> Option<Packed> {
    let width = plan.width.max(2);
    let height = plan.height.max(2);
    let mut chosen: Vec<&Layer> = Vec::new();
    let mut index = 0;
    while index < plan.layers.len() {
        let layer = &plan.layers[index];
        let Layer::Video {
            incoming, overlay, ..
        } = layer
        else {
            index += 1;
            continue;
        };
        if *incoming {
            index += 1;
            continue;
        }
        let partner = plan.layers.get(index + 1);
        if !*overlay && matches!(partner, Some(Layer::Video { incoming: true, .. })) {
            chosen.push(layer);
            chosen.push(partner.unwrap());
            index += 2;
            continue;
        }
        chosen.push(layer);
        index += 1;
    }
    if chosen.len() > MAX_LAYERS {
        return None;
    }
    for layer in &chosen {
        if curve_overflow(layer) {
            return None;
        }
    }

    let mut picture_ids: Vec<usize> = Vec::new();
    let mut cube_ids: Vec<u32> = Vec::new();
    for layer in &chosen {
        if let Some(slot) = picture_slot(layer, sources) {
            if !picture_ids.contains(&slot) {
                if picture_ids.len() >= MAX_PICTURES {
                    return None;
                }
                picture_ids.push(slot);
            }
        }
        if let Some(id) = cube_id(layer, cubes) {
            if !cube_ids.contains(&id) {
                if cube_ids.len() >= MAX_CUBES {
                    return None;
                }
                cube_ids.push(id);
            }
        }
    }

    let mut scene = GpuScene::zeroed();
    let bg = parse_hex(&plan.background);
    scene.frame.size = [
        width as f32,
        height as f32,
        chosen.len() as f32,
        if plan.letterbox { 1.0 } else { 0.0 },
    ];
    scene.frame.bg = [bg[0], bg[1], bg[2], 1.0];
    scene.frame.shift = [shift.0 as f32, shift.1 as f32, 0.0, 0.0];
    if let Some(id) = cube_ids.first() {
        scene.frame.cubes[0] = cube_size(cubes, *id);
    }
    if let Some(id) = cube_ids.get(1) {
        scene.frame.cubes[1] = cube_size(cubes, *id);
    }
    for (slot, source_index) in picture_ids.iter().enumerate() {
        let source = &sources[*source_index];
        scene.frame.tex[slot] = [
            source.width.max(1) as f32,
            source.height.max(1) as f32,
            0.0,
            0.0,
        ];
    }

    for (index, layer) in chosen.iter().enumerate() {
        scene.layers[index] = pack_layer(layer, width, height, sources, &picture_ids, &cube_ids);
    }

    let pictures = picture_ids
        .iter()
        .map(|index| {
            let source = &sources[*index];
            Picture {
                width: source.width.max(1),
                height: source.height.max(1),
                rgba: padded_rgba(&source.rgba, source.width.max(1), source.height.max(1)),
            }
        })
        .collect();
    let cubes = cube_ids
        .iter()
        .filter_map(|id| cubes.iter().find(|cube| cube.id == *id).map(quantize_cube))
        .collect();
    Some(Packed {
        scene,
        pictures,
        cubes,
    })
}

fn picture_slot(layer: &Layer, sources: &[FrameSource]) -> Option<usize> {
    let Layer::Video {
        media_id,
        source_time,
        generator,
        ..
    } = layer
    else {
        return None;
    };
    if generator.is_some() {
        return None;
    }
    nearest_source(sources, media_id, source_time.as_seconds())
}

fn nearest_source(
    sources: &[FrameSource],
    media_id: &oc_timeline::MediaId,
    source_time: f64,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_distance = f64::MAX;
    for (index, source) in sources.iter().enumerate() {
        if source.media_id != *media_id {
            continue;
        }
        let distance = (source.source_time - source_time).abs();
        if best.is_none() || distance < best_distance {
            best = Some(index);
            best_distance = distance;
        }
    }
    best
}

fn cube_id(layer: &Layer, cubes: &[CubeLut]) -> Option<u32> {
    let Layer::Video { grade, .. } = layer else {
        return None;
    };
    let id = grade.cube?;
    cubes.iter().any(|cube| cube.id == id).then_some(id)
}

fn cube_size(cubes: &[CubeLut], id: u32) -> f32 {
    cubes
        .iter()
        .find(|cube| cube.id == id)
        .map(|cube| cube.size as f32)
        .unwrap_or(0.0)
}

fn curve_overflow(layer: &Layer) -> bool {
    let Layer::Video { curves, .. } = layer else {
        return false;
    };
    curves.all.len() > MAX_CURVE_POINTS
        || curves.red.len() > MAX_CURVE_POINTS
        || curves.green.len() > MAX_CURVE_POINTS
        || curves.blue.len() > MAX_CURVE_POINTS
}

fn pack_layer(
    layer: &Layer,
    width: u32,
    height: u32,
    sources: &[FrameSource],
    picture_ids: &[usize],
    cube_ids: &[u32],
) -> GpuLayer {
    let Layer::Video {
        source_time,
        transform,
        grade,
        fx,
        opacity,
        transition,
        mix,
        curves,
        mask,
        crop,
        card,
        generator,
        incoming,
        ..
    } = layer
    else {
        return GpuLayer::zeroed();
    };
    let mut gpu = GpuLayer::zeroed();
    let rect = placement(*card, width, height);
    gpu.rect = rect;
    gpu.xform = [
        pan_px(transform.x, rect[2]),
        pan_px(transform.y, rect[3]),
        -transform.rotation.to_radians(),
        transform.scale.abs().max(0.05),
    ];
    gpu.grade = [
        grade.exposure,
        grade.contrast,
        grade.saturation,
        grade.temperature,
    ];
    gpu.grade2 = [grade.lift, grade.gamma, grade.gain, lut_id(grade.lut)];
    gpu.fx = [
        fx.blur,
        fx.vignette,
        opacity.clamp(0.0, 1.0),
        mix.clamp(0.0, 1.0),
    ];
    let (mask_rect, mask_flags) = mask_of(*mask);
    gpu.mask = mask_rect;
    gpu.mask2 = mask_flags;
    gpu.crop = crop_of(*crop);
    let tex = picture_slot(layer, sources)
        .and_then(|source| picture_ids.iter().position(|slot| *slot == source))
        .map(|slot| slot as f32)
        .unwrap_or(-1.0);
    let (generated, color) = match generator {
        Some(kind) => generator_of(kind),
        None => (0.0, [0.0; 3]),
    };
    gpu.flags = [
        tex,
        generated,
        mix_mode(*transition),
        if *incoming { 1.0 } else { 0.0 },
    ];
    let cube_slot = grade
        .cube
        .and_then(|id| cube_ids.iter().position(|slot| *slot == id))
        .map(|slot| slot as f32)
        .unwrap_or(-1.0);
    gpu.extra = [
        source_time.as_seconds() as f32,
        cube_slot,
        color[0],
        color[1],
    ];
    gpu.slide = slide_of(*transition, *mix, width, height);
    let (cell_w, cell_h) = mosaic_cell(*mix, width, height);
    gpu.genb = [color[2], cell_w as f32, cell_h as f32, 0.0];
    gpu.counts = [
        curves.all.len() as f32,
        curves.red.len() as f32,
        curves.green.len() as f32,
        curves.blue.len() as f32,
    ];
    write_curve(&mut gpu.curves, 0, &curves.all);
    write_curve(&mut gpu.curves, 4, &curves.red);
    write_curve(&mut gpu.curves, 8, &curves.green);
    write_curve(&mut gpu.curves, 12, &curves.blue);
    gpu
}

fn placement(card: Option<FrameCard>, width: u32, height: u32) -> [f32; 4] {
    if let Some(card) = card {
        [
            card.x.clamp(0.0, 1.0) * width as f32,
            card.y.clamp(0.0, 1.0) * height as f32,
            (card.w.clamp(0.02, 1.0) * width as f32).max(1.0),
            (card.h.clamp(0.02, 1.0) * height as f32).max(1.0),
        ]
    } else {
        [0.0, 0.0, width as f32, height as f32]
    }
}

fn crop_of(crop: Option<oc_timeline::Crop>) -> [f32; 4] {
    match crop {
        Some(crop) => [
            crop.x.clamp(0.0, 1.0),
            crop.y.clamp(0.0, 1.0),
            crop.w.clamp(0.02, 1.0),
            crop.h.clamp(0.02, 1.0),
        ],
        None => [0.0, 0.0, 1.0, 1.0],
    }
}

fn mask_of(mask: Option<AlphaShape>) -> ([f32; 4], [f32; 4]) {
    let Some(mask) = mask else {
        return ([0.0; 4], [0.0; 4]);
    };
    let shape = match mask.shape {
        MaskShape::Rectangle => 0.0,
        MaskShape::Ellipse => 1.0,
        MaskShape::Diamond => 2.0,
        MaskShape::Triangle => 3.0,
    };
    (
        [mask.x, mask.y, mask.w, mask.h],
        [
            shape,
            mask.feather,
            if mask.invert { 1.0 } else { 0.0 },
            1.0,
        ],
    )
}

fn lut_id(lut: Lut) -> f32 {
    match lut {
        Lut::None => 0.0,
        Lut::Film => 1.0,
        Lut::Cool => 2.0,
        Lut::Warm => 3.0,
        Lut::TealOrange => 4.0,
        Lut::Mono => 5.0,
    }
}

fn generator_of(kind: &Generator) -> (f32, [f32; 3]) {
    match kind {
        Generator::Color { color } => (1.0, parse_hex(color)),
        Generator::ColorBars => (2.0, [0.0; 3]),
        Generator::WhiteNoise => (3.0, [0.0; 3]),
        Generator::Counter => (4.0, [0.0; 3]),
    }
}

fn mix_mode(kind: TransitionKind) -> f32 {
    match kind {
        TransitionKind::Cut => 0.0,
        TransitionKind::FadeBlack => 2.0,
        TransitionKind::FadeWhite => 3.0,
        TransitionKind::Wipe | TransitionKind::WipeLeft | TransitionKind::SmoothLeft => 4.0,
        TransitionKind::WipeRight | TransitionKind::SmoothRight => 5.0,
        TransitionKind::WipeUp | TransitionKind::SmoothUp => 6.0,
        TransitionKind::WipeDown | TransitionKind::SmoothDown => 7.0,
        TransitionKind::WipeTl => 8.0,
        TransitionKind::WipeTr => 9.0,
        TransitionKind::WipeBl => 10.0,
        TransitionKind::WipeBr => 11.0,
        TransitionKind::HorzOpen => 12.0,
        TransitionKind::VertOpen => 13.0,
        TransitionKind::CircleOpen => 14.0,
        TransitionKind::CircleClose => 15.0,
        TransitionKind::Radial => 16.0,
        TransitionKind::Pixelize => 17.0,
        _ => 1.0,
    }
}

fn slide_of(kind: TransitionKind, mix: f32, width: u32, height: u32) -> [f32; 4] {
    let Some((dx, dy)) = kind.slide_delta() else {
        return [0.0; 4];
    };
    let p = mix.clamp(0.0, 1.0);
    [
        dx as f32 * p * width as f32,
        dy as f32 * p * height as f32,
        -dx as f32 * (1.0 - p) * width as f32,
        -dy as f32 * (1.0 - p) * height as f32,
    ]
}

fn mosaic_cell(mix: f32, width: u32, height: u32) -> (u32, u32) {
    let cells = 6.0 + mix.clamp(0.0, 1.0) * 40.0;
    let cell_w = (width as f32 / cells).round().max(1.0) as u32;
    let cell_h = (height as f32 / cells).round().max(1.0) as u32;
    (cell_w.max(1), cell_h.max(1))
}

fn write_curve(dst: &mut [[f32; 4]; 16], base: usize, points: &[CurvePoint]) {
    for (index, point) in points.iter().take(MAX_CURVE_POINTS).enumerate() {
        let slot = base + index / 2;
        let x = point.x.clamp(0.0, 1.0);
        let y = point.y.clamp(0.0, 1.0);
        if index % 2 == 0 {
            dst[slot][0] = x;
            dst[slot][1] = y;
        } else {
            dst[slot][2] = x;
            dst[slot][3] = y;
        }
    }
}

fn quantize_cube(cube: &CubeLut) -> CubeUpload {
    let size = cube.size.max(1);
    let texels = (size as usize).saturating_pow(3);
    let mut rgba = vec![0u8; texels * 4];
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                let src = ((b * size * size + g * size + r) * 3) as usize;
                let dst = ((b * size * size + g * size + r) * 4) as usize;
                if src + 2 >= cube.rgb.len() || dst + 3 >= rgba.len() {
                    continue;
                }
                for channel in 0..3 {
                    let sample = cube.rgb[src + channel].clamp(0.0, 1.0);
                    rgba[dst + channel] = (sample * 255.0).round() as u8;
                }
                rgba[dst + 3] = 255;
            }
        }
    }
    CubeUpload { size, rgba }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_engine() -> Result<&'static std::sync::Mutex<Engine>, BootError> {
    static ENGINE: OnceLock<Result<std::sync::Mutex<Engine>, BootError>> = OnceLock::new();
    match ENGINE.get_or_init(|| pollster::block_on(Engine::boot()).map(std::sync::Mutex::new)) {
        Ok(engine) => Ok(engine),
        Err(err) => Err(match err {
            BootError::NoAdapter(text) => BootError::NoAdapter(text.clone()),
            BootError::Fault(text) => BootError::Fault(text.clone()),
        }),
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm_monitor {
    use std::cell::{Cell, RefCell};

    use super::*;

    enum Slot {
        Empty,
        Pending { epoch: u32, started: f64 },
        Ready(Box<Session>),
        Failed,
    }

    struct Session {
        engine: Engine,
        surface: Option<wgpu::Surface<'static>>,
        canvas: Option<web_sys::HtmlCanvasElement>,
        configured: Option<(u32, u32, wgpu::TextureFormat)>,
    }

    thread_local! {
        static SLOT: RefCell<Slot> = const { RefCell::new(Slot::Empty) };
        static WARNED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn monitor_path() -> MonitorPath {
        SLOT.with(|slot| {
            let mut slot = slot.borrow_mut();
            match &mut *slot {
                Slot::Failed => MonitorPath::Cpu,
                Slot::Ready(_) => MonitorPath::Gpu,
                Slot::Pending { epoch, started } => {
                    if now_secs() - *started > 3.0 {
                        let epoch = *epoch;
                        warn_once("opencut: WebGPU monitor timed out, using CPU paint");
                        *slot = Slot::Failed;
                        let _ = epoch;
                        MonitorPath::Cpu
                    } else {
                        MonitorPath::Waiting
                    }
                }
                Slot::Empty => {
                    let epoch = 1;
                    *slot = Slot::Pending {
                        epoch,
                        started: now_secs(),
                    };
                    drop(slot);
                    spawn_boot(epoch);
                    MonitorPath::Waiting
                }
            }
        })
    }

    pub fn present(
        canvas: &web_sys::HtmlCanvasElement,
        plan: &FramePlan,
        sources: &[FrameSource],
        cubes: &[CubeLut],
        shift: (i32, i32),
    ) -> bool {
        SLOT.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Slot::Ready(session) = &mut *slot else {
                return false;
            };
            match session.present(canvas, plan, sources, cubes, shift) {
                Present::Frame => true,
                Present::GiveUp => {
                    warn_once("opencut: WebGPU surface failed, using CPU paint");
                    *slot = Slot::Failed;
                    false
                }
            }
        })
    }

    enum Present {
        Frame,
        GiveUp,
    }

    impl Session {
        fn present(
            &mut self,
            canvas: &web_sys::HtmlCanvasElement,
            plan: &FramePlan,
            sources: &[FrameSource],
            cubes: &[CubeLut],
            shift: (i32, i32),
        ) -> Present {
            let width = plan.width.max(2);
            let height = plan.height.max(2);
            if self
                .canvas
                .as_ref()
                .is_none_or(|have| !have.is_same_node(Some(canvas)))
            {
                self.surface = None;
                self.configured = None;
                match surface_from_canvas(&self.engine.instance, canvas) {
                    Ok(surface) => {
                        self.surface = Some(surface);
                        self.canvas = Some(canvas.clone());
                    }
                    Err(err) => {
                        warn_once(&err);
                        return Present::GiveUp;
                    }
                }
            }
            if canvas.width() != width {
                canvas.set_width(width);
                self.configured = None;
            }
            if canvas.height() != height {
                canvas.set_height(height);
                self.configured = None;
            }
            let Some(surface) = self.surface.as_ref() else {
                return Present::GiveUp;
            };
            // The pointer keeps configure() from holding `self.surface` while `self.engine` is used.
            // This function does not move `self.surface` while the pointer is live.
            let surface = surface as *const wgpu::Surface<'static>;
            let surface = unsafe { &*surface };
            if self.configured.map(|(w, h, _)| (w, h)) != Some((width, height)) {
                match configure_surface(&self.engine, surface, width, height) {
                    Ok(format) => self.configured = Some((width, height, format)),
                    Err(err) => {
                        // The canvas already has a WebGPU context. Stay here and try again.
                        warn_once(&err);
                        return Present::Frame;
                    }
                }
            }
            let Some((_, _, format)) = self.configured else {
                return Present::Frame;
            };
            let Some(frame) = acquire(surface).or_else(|| {
                if let Ok(format) = configure_surface(&self.engine, surface, width, height) {
                    self.configured = Some((width, height, format));
                }
                acquire(surface)
            }) else {
                return Present::Frame;
            };
            let view = frame.texture.create_view(&Default::default());
            if let Err(err) = self
                .engine
                .paint(&view, format, plan, sources, cubes, shift)
            {
                warn_once(&err);
                return Present::Frame;
            }
            self.engine.queue.present(frame);
            Present::Frame
        }
    }

    fn configure_surface(
        engine: &Engine,
        surface: &wgpu::Surface<'_>,
        width: u32,
        height: u32,
    ) -> Result<wgpu::TextureFormat, String> {
        let caps = surface.get_capabilities(&engine.adapter);
        let format = pick_format(&caps);
        let mut config = surface
            .get_default_config(&engine.adapter, width, height)
            .ok_or_else(|| "surface has no configuration".to_string())?;
        config.format = format;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        config.width = width;
        config.height = height;
        surface.configure(&engine.device, &config);
        Ok(format)
    }

    fn pick_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
        for format in [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            if caps.formats.contains(&format) {
                return format;
            }
        }
        caps.formats
            .first()
            .copied()
            .unwrap_or(wgpu::TextureFormat::Rgba8Unorm)
    }

    fn acquire(surface: &wgpu::Surface<'_>) -> Option<wgpu::SurfaceTexture> {
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            _ => None,
        }
    }

    fn surface_from_canvas(
        instance: &wgpu::Instance,
        canvas: &web_sys::HtmlCanvasElement,
    ) -> Result<wgpu::Surface<'static>, String> {
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|err| err.to_string())?;
        // wgpu copies the canvas handle. The Rust lifetime does not own it.
        Ok(unsafe { std::mem::transmute::<wgpu::Surface<'_>, wgpu::Surface<'static>>(surface) })
    }

    fn spawn_boot(epoch: u32) {
        wasm_bindgen_futures::spawn_local(async move {
            let booted = Engine::boot().await;
            SLOT.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Slot::Pending { epoch: current, .. } = *slot else {
                    return;
                };
                if current != epoch {
                    return;
                }
                match booted {
                    Ok(engine) => {
                        *slot = Slot::Ready(Box::new(Session {
                            engine,
                            surface: None,
                            canvas: None,
                            configured: None,
                        }));
                    }
                    Err(err) => {
                        warn_once(err.text());
                        *slot = Slot::Failed;
                    }
                }
            });
        });
    }

    fn now_secs() -> f64 {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now() / 1000.0)
            .unwrap_or(0.0)
    }

    fn warn_once(message: &str) {
        WARNED.with(|warned| {
            if warned.get() {
                return;
            }
            warned.set(true);
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(message));
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn wasm_monitor_path() -> MonitorPath {
    wasm_monitor::monitor_path()
}

#[cfg(target_arch = "wasm32")]
fn wasm_present(
    canvas: &web_sys::HtmlCanvasElement,
    plan: &FramePlan,
    sources: &[FrameSource],
    cubes: &[CubeLut],
    shift: (i32, i32),
) -> bool {
    wasm_monitor::present(canvas, plan, sources, cubes, shift)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_time::{Duration, Time};
    use oc_timeline::{
        Clip, ClipId, ClipKind, ClipLook, CurvePoint, FrameCard, MediaId, Timeline, TrackKind,
        Transform,
    };

    fn assert_layout() {
        assert_eq!(std::mem::size_of::<GpuFrame>(), 128);
        assert_eq!(std::mem::size_of::<GpuLayer>(), 464);
        assert_eq!(std::mem::size_of::<GpuScene>(), 128 + 464 * MAX_LAYERS);
    }

    #[test]
    fn scene_layout_matches_the_shader() {
        assert_layout();
    }

    fn video(media: MediaId, look: ClipLook) -> Layer {
        Layer::Video {
            media_id: media,
            source_time: Time::ZERO,
            transform: Transform::default(),
            grade: look.grade,
            fx: look.fx,
            opacity: 1.0,
            transition: look.transition,
            mix: 0.0,
            curves: look.curves,
            mask: look.mask,
            crop: look.crop,
            card: look.card,
            generator: look.generator,
            overlay: look.overlay,
            incoming: false,
            stabilize: false,
        }
    }

    fn plan_of(width: u32, height: u32, layers: Vec<Layer>) -> FramePlan {
        FramePlan {
            time: Time::ZERO,
            width,
            height,
            layers,
            needs_paint: true,
            background: "#000000".into(),
            letterbox: false,
        }
    }

    fn solid(media: MediaId, rgb: [u8; 3], n: u32) -> FrameSource {
        let mut rgba = Vec::new();
        for _ in 0..n * n {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        FrameSource {
            media_id: media,
            source_time: 0.0,
            width: n,
            height: n,
            rgba,
        }
    }

    fn plate(media: MediaId, look: ClipLook, start: Time) -> Clip {
        Clip {
            id: ClipId::new(),
            media_id: Some(media),
            kind: ClipKind::Video {
                transform: Transform::default(),
            },
            start,
            duration: Duration::from_seconds(4.0),
            source_in: Time::ZERO,
            speed: 1.0,
            group_id: None,
            link_id: None,
            disabled: false,
            look,
        }
    }

    #[test]
    fn nine_layers_or_a_long_curve_do_not_pack() {
        assert_layout();
        let media = MediaId::new();
        let layers = (0..9).map(|_| video(media, ClipLook::default())).collect();
        let plan = plan_of(8, 8, layers);
        assert!(pack(&plan, &[solid(media, [255, 0, 0], 4)], &[], (0, 0)).is_none());
        let mut look = ClipLook::default();
        look.curves.all = (0..9)
            .map(|i| CurvePoint {
                x: i as f32 / 8.0,
                y: 0.5,
            })
            .collect();
        let plan = plan_of(8, 8, vec![video(media, look)]);
        assert!(pack(&plan, &[solid(media, [255, 0, 0], 4)], &[], (0, 0)).is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn draw_gpu(
        plan: &FramePlan,
        sources: &[FrameSource],
        cubes: &[CubeLut],
        shift: (i32, i32),
    ) -> Option<Surface> {
        let engine = match native_engine() {
            Ok(engine) => engine,
            Err(BootError::NoAdapter(text)) => {
                eprintln!("skip gpu pixels: {text}");
                return None;
            }
            Err(BootError::Fault(text)) => panic!("{text}"),
        };
        let mut engine = engine.lock().expect("gpu engine");
        Some(
            engine
                .render_rgba(plan, sources, cubes, shift)
                .unwrap_or_else(|err| panic!("{err}")),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assert_match(
        plan: &FramePlan,
        sources: &[FrameSource],
        cubes: &[CubeLut],
        shift: (i32, i32),
        tol: i32,
    ) {
        assert!(
            pack(plan, sources, cubes, shift).is_some(),
            "cpu blit must not satisfy this frame"
        );
        let Some(gpu) = draw_gpu(plan, sources, cubes, shift) else {
            return;
        };
        let cpu = crate::composite(plan, sources, cubes);
        assert_eq!(gpu.width, cpu.width);
        assert_eq!(gpu.height, cpu.height);
        let mut worst = 0;
        let mut where_at = (0u32, 0u32);
        let mut cpu_px = [0u8; 3];
        let mut gpu_px = [0u8; 3];
        for y in 0..cpu.height {
            for x in 0..cpu.width {
                let sx = x as i32 - shift.0;
                let sy = y as i32 - shift.1;
                let expect =
                    if sx >= 0 && sy >= 0 && (sx as u32) < cpu.width && (sy as u32) < cpu.height {
                        let i = ((sy as u32 * cpu.width + sx as u32) * 4) as usize;
                        [cpu.rgba[i], cpu.rgba[i + 1], cpu.rgba[i + 2]]
                    } else {
                        [0, 0, 0]
                    };
                let i = ((y * gpu.width + x) * 4) as usize;
                let got = [gpu.rgba[i], gpu.rgba[i + 1], gpu.rgba[i + 2]];
                for channel in 0..3 {
                    let delta = (expect[channel] as i32 - got[channel] as i32).abs();
                    if delta > worst {
                        worst = delta;
                        where_at = (x, y);
                        cpu_px = expect;
                        gpu_px = got;
                    }
                }
            }
        }
        assert!(
            worst <= tol,
            "delta {worst} at {where_at:?} cpu {cpu_px:?} gpu {gpu_px:?}"
        );
    }

    #[test]
    fn a_red_frame_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let media = MediaId::new();
            let plan = plan_of(16, 12, vec![video(media, ClipLook::default())]);
            let sources = [solid(media, [255, 0, 0], 8)];
            assert_match(&plan, &sources, &[], (0, 0), 0);
        }
    }

    #[test]
    fn film_grade_stays_within_two() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let media = MediaId::new();
            let mut look = ClipLook::default();
            look.grade.lut = Lut::Film;
            look.grade.exposure = 0.08;
            look.grade.contrast = 0.14;
            let plan = plan_of(12, 10, vec![video(media, look)]);
            assert_match(&plan, &[solid(media, [180, 40, 40], 8)], &[], (0, 0), 2);
        }
    }

    #[test]
    fn a_dissolve_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let outgoing = MediaId::new();
            let incoming = MediaId::new();
            let mut out = video(
                outgoing,
                ClipLook {
                    transition: TransitionKind::Dissolve,
                    ..ClipLook::default()
                },
            );
            let mut inc = video(incoming, ClipLook::default());
            if let Layer::Video { mix, .. } = &mut out {
                *mix = 0.5;
            }
            if let Layer::Video { incoming, .. } = &mut inc {
                *incoming = true;
            }
            let plan = plan_of(16, 10, vec![out, inc]);
            assert_match(
                &plan,
                &[
                    solid(outgoing, [255, 0, 0], 8),
                    solid(incoming, [0, 0, 255], 8),
                ],
                &[],
                (0, 0),
                2,
            );
        }
    }

    #[test]
    fn an_ellipse_mask_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let media = MediaId::new();
            let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 16, 16);
            tl.background = "#000000".into();
            let track = tl.first_track(TrackKind::Video).unwrap().id;
            let look = ClipLook {
                mask: Some(oc_timeline::AlphaShape {
                    shape: MaskShape::Ellipse,
                    x: 0.5,
                    y: 0.5,
                    w: 0.5,
                    h: 0.5,
                    feather: 0.0,
                    invert: false,
                }),
                ..ClipLook::default()
            };
            tl.add_clip(track, plate(media, look, Time::ZERO)).unwrap();
            let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
            assert_match(&plan, &[solid(media, [255, 0, 0], 16)], &[], (0, 0), 1);
        }
    }

    #[test]
    fn a_wipe_matches_at_both_ends() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let outgoing = MediaId::new();
            let incoming = MediaId::new();
            for mix in [0.0, 1.0] {
                let mut out = video(
                    outgoing,
                    ClipLook {
                        transition: TransitionKind::WipeLeft,
                        ..ClipLook::default()
                    },
                );
                let mut inc = video(incoming, ClipLook::default());
                if let Layer::Video { mix: slot, .. } = &mut out {
                    *slot = mix;
                }
                if let Layer::Video { incoming, .. } = &mut inc {
                    *incoming = true;
                }
                let plan = plan_of(20, 8, vec![out, inc]);
                assert_match(
                    &plan,
                    &[
                        solid(outgoing, [200, 0, 0], 8),
                        solid(incoming, [0, 180, 0], 8),
                    ],
                    &[],
                    (0, 0),
                    1,
                );
            }
        }
    }

    #[test]
    fn a_card_keeps_its_corner() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let speaker = MediaId::new();
            let design = MediaId::new();
            let mut tl = Timeline::new(oc_timeline::FrameRate::FPS_30, 20, 10);
            let v1 = tl.first_track(TrackKind::Video).unwrap().id;
            tl.add_clip(v1, plate(speaker, ClipLook::default(), Time::ZERO))
                .unwrap();
            let design_track = tl.add_track(TrackKind::Video, "Design");
            let look = ClipLook {
                card: Some(FrameCard {
                    x: 0.0,
                    y: 0.0,
                    w: 0.5,
                    h: 0.5,
                }),
                overlay: true,
                ..ClipLook::default()
            };
            tl.add_clip(design_track, plate(design, look, Time::ZERO))
                .unwrap();
            let plan = crate::plan_frame(&tl, Time::from_seconds(0.2));
            assert_match(
                &plan,
                &[
                    solid(speaker, [200, 0, 0], 8),
                    solid(design, [0, 180, 0], 8),
                ],
                &[],
                (0, 0),
                1,
            );
        }
    }

    #[test]
    fn letterbox_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let media = MediaId::new();
            let mut plan = plan_of(16, 40, vec![video(media, ClipLook::default())]);
            plan.letterbox = true;
            assert_match(&plan, &[solid(media, [0, 90, 200], 8)], &[], (0, 0), 1);
        }
    }

    #[test]
    fn a_counter_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut layer = video(
                MediaId::new(),
                ClipLook {
                    generator: Some(oc_timeline::Generator::Counter),
                    ..ClipLook::default()
                },
            );
            if let Layer::Video { source_time, .. } = &mut layer {
                *source_time = Time::from_seconds(75.0);
            }
            let plan = plan_of(96, 54, vec![layer]);
            assert_match(&plan, &[], &[], (0, 0), 0);
        }
    }

    #[test]
    fn stabilize_shift_matches_the_cpu() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let media = MediaId::new();
            let plan = plan_of(18, 12, vec![video(media, ClipLook::default())]);
            assert_match(&plan, &[solid(media, [20, 200, 40], 8)], &[], (3, -2), 0);
        }
    }
}
