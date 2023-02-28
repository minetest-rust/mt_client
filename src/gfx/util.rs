use cgmath::Matrix4;
use wgpu::util::DeviceExt;

pub struct MatrixUniform {
    buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

impl MatrixUniform {
    pub fn new(
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
        init: Matrix4<f32>,
        name: &str,
        writable: bool,
    ) -> Self {
        let uniform: [[f32; 4]; 4] = init.into();

        let mut usage = wgpu::BufferUsages::UNIFORM;

        if writable {
            usage |= wgpu::BufferUsages::COPY_DST;
        }

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{name}.buffer")),
            contents: bytemuck::cast_slice(&[uniform]),
            usage,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
            label: Some(&format!("{name}.bind_group")),
        });

        Self { buffer, bind_group }
    }

    pub fn layout(device: &wgpu::Device, name: &str) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some(&format!("{name}.bind_group_layout")),
        })
    }

    pub fn set(&self, queue: &wgpu::Queue, to: Matrix4<f32>) {
        let uniform: [[f32; 4]; 4] = to.into();
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[uniform]));
    }
}
