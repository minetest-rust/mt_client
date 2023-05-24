use super::{gpu::Gpu, util::MatrixUniform};
use cgmath::{prelude::*, Deg, Euler, Matrix3, Matrix4, Point3, Rad, Vector3};
use collision::Frustum;
use std::time::Duration;

#[derive(Default)]
pub struct CameraInput {
    pub forward: bool,
    pub left: bool,
    pub backward: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub mouse_x: f32,
    pub mouse_y: f32,
}

pub struct Camera {
    pub pos: Point3<f32>,
    pub rot: Euler<Deg<f32>>,
    pub speed: f32,
    pub fov: Rad<f32>,
    pub view: Matrix4<f32>,
    pub proj: Matrix4<f32>,
    pub frustum: Frustum<f32>,
    pub uniform: MatrixUniform,
    pub layout: wgpu::BindGroupLayout,
    pub input: CameraInput,
}

pub trait ToNative {
    fn to_native(self) -> Self;
}

impl<T> ToNative for Euler<T>
where
    T: From<Deg<f32>> + std::ops::Add<T, Output = T> + std::ops::Neg<Output = T>,
{
    fn to_native(mut self) -> Self {
        self.y = -self.y + Deg(270.0).into();
        self.z = -self.z;
        self
    }
}

impl Camera {
    pub fn new(gpu: &Gpu) -> Self {
        let layout = MatrixUniform::layout(&gpu.device, "camera");
        let uniform = MatrixUniform::new(&gpu.device, &layout, Matrix4::identity(), "camera", true);

        Self {
            pos: Point3::new(0.0, 0.0, 0.0),
            rot: Euler {
                x: Deg(0.0),
                y: Deg(0.0),
                z: Deg(0.0),
            },
            speed: 0.0,
            fov: Deg(90.0).into(),
            proj: Matrix4::identity(),
            view: Matrix4::identity(),
            frustum: Frustum::from_matrix4(Matrix4::identity()).unwrap(),
            uniform,
            layout,
            input: Default::default(),
        }
    }

    pub fn update(&mut self, gpu: &Gpu, dt: Duration) {
        let dt = dt.as_secs_f32();

        let sensitivity = dt * 2.0;

        self.rot.y += Deg(sensitivity * self.input.mouse_x);
        self.rot.z += Deg(sensitivity * self.input.mouse_y);
        self.rot.z.0 = self.rot.z.0.min(89.9).max(-89.9);

        self.input.mouse_x = 0.0;
        self.input.mouse_y = 0.0;

        let rot = Matrix3::from(self.rot.to_native());

        let forward = rot * Vector3::unit_x();
        let up = rot * Vector3::unit_y();

        {
            let mut forward = forward;
            let mut up = up;
            let mut right = forward.cross(up);

            let pitch_move = false;

            if !pitch_move {
                forward.y = 0.0;
                right.y = 0.0;
                up = Vector3::unit_y();
            }

            let mut hdir = Vector3::zero();
            let mut vdir = Vector3::zero();

            if self.input.forward {
                hdir += forward;
            }
            if self.input.backward {
                hdir -= forward;
            }
            if self.input.right {
                hdir += right;
            }
            if self.input.left {
                hdir -= right;
            }
            if self.input.jump {
                vdir += up;
            }
            if self.input.sneak {
                vdir -= up;
            }

            self.pos += self.speed
                * dt
                * (vdir
                    + if hdir.is_zero() {
                        hdir
                    } else {
                        hdir.normalize()
                    });
        }

        self.view = Matrix4::look_at_dir(self.pos, forward, up);
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
