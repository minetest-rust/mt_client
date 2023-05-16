use super::{gpu::Gpu, util::MatrixUniform};
use cgmath::{prelude::*, Deg, Matrix4, Rad};
use collision::Frustum;
use fps_camera::{FirstPerson, FirstPersonSettings};
use std::time::Duration;

pub struct Camera {
    pub fov: Rad<f32>,
    pub view: Matrix4<f32>,
    pub proj: Matrix4<f32>,
    pub frustum: Frustum<f32>,
    pub first_person: FirstPerson,
    pub uniform: MatrixUniform,
    pub layout: wgpu::BindGroupLayout,
}

impl Camera {
    pub fn new(gpu: &Gpu) -> Self {
        let first_person = FirstPerson::new(
            [0.0, 0.0, 0.0],
            FirstPersonSettings {
                speed_horizontal: 10.0,
                speed_vertical: 10.0,
                mouse_sensitivity_horizontal: 1.0,
                mouse_sensitivity_vertical: 1.0,
            },
        );

        let layout = MatrixUniform::layout(&gpu.device, "camera");
        let uniform = MatrixUniform::new(&gpu.device, &layout, Matrix4::identity(), "camera", true);

        Self {
            fov: Deg(90.0).into(),
            proj: Matrix4::identity(),
            view: Matrix4::identity(),
            frustum: Frustum::from_matrix4(Matrix4::identity()).unwrap(),
            first_person,
            uniform,
            layout,
        }
    }

    pub fn update(&mut self, gpu: &Gpu, dt: Duration) {
        self.first_person.yaw += Rad::from(Deg(180.0)).0;
        self.first_person.yaw *= -1.0;

        let cam = self.first_person.camera(dt.as_secs_f32());

        self.first_person.yaw *= -1.0;
        self.first_person.yaw -= Rad::from(Deg(180.0)).0;

        self.first_person.position = cam.position;

        self.view = Matrix4::from(cam.orthogonal());
        self.uniform.set(&gpu.queue, self.proj * self.view);
    }

    pub fn resize(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.proj = cgmath::perspective(
            self.fov,
            size.width as f32 / size.height as f32,
            0.1,
            100000.0,
        );
        self.frustum = Frustum::from_matrix4(self.proj).unwrap();
    }
}
