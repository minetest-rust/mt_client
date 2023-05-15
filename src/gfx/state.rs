use super::util::MatrixUniform;
use cgmath::{prelude::*, Deg, Matrix4, Rad};
use collision::Frustum;
use fps_camera::{FirstPerson, FirstPersonSettings};
use std::time::Duration;

pub struct State {
    pub surface: wgpu::Surface,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub fov: Rad<f32>,
    pub view: Matrix4<f32>,
    pub proj: Matrix4<f32>,
    pub frustum: Frustum<f32>,
    pub camera: FirstPerson,
    pub camera_uniform: MatrixUniform,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub depth_sampler: wgpu::Sampler,
}

impl State {
    pub async fn new(window: &winit::window::Window) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            dx12_shader_compiler: Default::default(),
        });

        let surface = unsafe { instance.create_surface(window) }.unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    features: wgpu::Features::empty(),
                    limits: Default::default(),
                    label: None,
                },
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.describe().srgb)
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };

        let (depth_texture, depth_view, depth_sampler) =
            Self::create_depth_texture(&config, &device);

        let camera = FirstPerson::new(
            [0.0, 0.0, 0.0],
            FirstPersonSettings {
                speed_horizontal: 10.0,
                speed_vertical: 10.0,
                mouse_sensitivity_horizontal: 1.0,
                mouse_sensitivity_vertical: 1.0,
            },
        );

        let camera_bind_group_layout = MatrixUniform::layout(&device, "camera");

        let camera_uniform = MatrixUniform::new(
            &device,
            &camera_bind_group_layout,
            Matrix4::identity(),
            "camera",
            true,
        );

        let mut state = Self {
            surface,
            device,
            queue,
            config,
            fov: Deg(90.0).into(),
            proj: Matrix4::identity(),
            view: Matrix4::identity(),
            frustum: Frustum::from_matrix4(Matrix4::identity()).unwrap(),
            camera,
            camera_uniform,
            camera_bind_group_layout,
            depth_texture,
            depth_view,
            depth_sampler,
        };

        state.resize(size);

        state
    }

    pub fn create_depth_texture(
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
        let depth_size = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let depth_descriptor = wgpu::TextureDescriptor {
            label: Some("depth texture"),
            size: depth_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT // 3.
				| wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let depth_texture = device.create_texture(&depth_descriptor);

        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 100.0,
            ..Default::default()
        });

        (depth_texture, depth_view, depth_sampler)
    }

    pub fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            self.config.width = size.width;
            self.config.height = size.height;
            self.configure_surface();
            self.update_projection();
            (self.depth_texture, self.depth_view, self.depth_sampler) =
                Self::create_depth_texture(&self.config, &self.device);
        }
    }

    pub fn configure_surface(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }

    pub fn update_projection(&mut self) {
        self.proj = cgmath::perspective(
            self.fov,
            self.config.width as f32 / self.config.height as f32,
            0.1,
            100000.0,
        );
        self.frustum = Frustum::from_matrix4(self.proj).unwrap();
    }

    pub fn update(&mut self, dt: Duration) {
        self.camera.yaw += Rad::from(Deg(180.0)).0;
        self.camera.yaw *= -1.0;

        let cam = self.camera.camera(dt.as_secs_f32());

        self.camera.yaw *= -1.0;
        self.camera.yaw -= Rad::from(Deg(180.0)).0;

        self.camera.position = cam.position;

        self.view = Matrix4::from(cam.orthogonal());
        self.camera_uniform.set(&self.queue, self.proj * self.view);
    }

    pub fn render(&self, map: &Option<super::map::MapRender>) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0x87 as f64 / 255.0,
                            g: 0xCE as f64 / 255.0,
                            b: 0xEB as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    stencil_ops: None,
                }),
            });

            if let Some(map) = map.as_ref() {
                map.render(self, &mut render_pass);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
