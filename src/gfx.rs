use crate::{GfxEvent::*, NetEvent};
use cgmath::Rad;
use std::time::Instant;
use tokio::sync::mpsc;
use winit::{
    event::{DeviceEvent::*, Event::*, WindowEvent::*},
    event_loop::ControlFlow::ExitWithCode,
    platform::run_return::EventLoopExtRunReturn,
};

mod camera;
mod debug_menu;
mod font;
mod gpu;
mod map;
mod media;
mod util;

pub async fn run(
    mut event_loop: winit::event_loop::EventLoop<crate::GfxEvent>,
    net_events: mpsc::UnboundedSender<NetEvent>,
) {
    let window = winit::window::WindowBuilder::new()
        .build(&event_loop)
        .unwrap();

    window.set_cursor_visible(false);

    let mut gpu = gpu::Gpu::new(&window).await;
    let mut map: Option<map::MapRender> = None;
    let mut font = font::Font::new(&gpu);
    let mut debug_menu = debug_menu::DebugMenu::new();
    let mut media = media::MediaMgr::new();
    let mut camera = camera::Camera::new(&gpu);

    let mut nodedefs = None;
    let mut last_frame = Instant::now();
    let mut fps_counter = fps_counter::FPSCounter::new();
    let mut game_paused = false;

    event_loop.run_return(|event, _, flow| match event {
        MainEventsCleared => window.request_redraw(),
        RedrawRequested(id) if id == window.id() => {
            let now = Instant::now();
            let dt = now - last_frame;
            last_frame = now;

            debug_menu.fps = fps_counter.tick();
            camera.update(&gpu, dt);
            if let Some(map) = &mut map {
                map.update(&gpu);
            }

            net_events
                .send(NetEvent::PlayerPos(
                    camera.first_person.position.into(),
                    Rad(camera.first_person.yaw).into(),
                    Rad(camera.first_person.pitch).into(),
                ))
                .ok();

            let mut render = || {
                let size = (gpu.config.width as f32, gpu.config.height as f32);
                let mut frame = gpu::Frame::new(&mut gpu)?;

                {
                    let mut pass = frame.pass();
                    if let Some(map) = &mut map {
                        map.render(&camera, &mut debug_menu, &mut pass);
                    }
                }

                debug_menu.render(size, &camera, &mut font);
                font.submit(&mut frame);

                frame.finish();
                font.cleanup();

                Ok(())
            };

            use wgpu::SurfaceError::*;
            match render() {
                Ok(_) => {}
                Err(Lost) => gpu.configure_surface(),
                Err(OutOfMemory) => *flow = ExitWithCode(0),
                Err(err) => eprintln!("gfx error: {err:?}"),
            }
        }
        WindowEvent {
            event,
            window_id: id,
        } if id == window.id() => match event {
            CloseRequested => *flow = ExitWithCode(0),
            Resized(size)
            | ScaleFactorChanged {
                new_inner_size: &mut size,
                ..
            } => {
                gpu.resize(size);
                camera.resize(size);
            }
            KeyboardInput {
                input:
                    winit::event::KeyboardInput {
                        virtual_keycode: Some(key),
                        state: key_state,
                        ..
                    },
                ..
            } => {
                use fps_camera::Actions;
                use winit::event::{ElementState, VirtualKeyCode as Key};

                if key == Key::Escape && key_state == ElementState::Pressed {
                    game_paused = !game_paused;
                    window.set_cursor_visible(game_paused);
                }

                if key == Key::F3 && key_state == ElementState::Pressed {
                    debug_menu.enabled = !debug_menu.enabled;
                }

                if !game_paused {
                    let actions = match key {
                        Key::W => Actions::MOVE_FORWARD,
                        Key::A => Actions::STRAFE_LEFT,
                        Key::S => Actions::MOVE_BACKWARD,
                        Key::D => Actions::STRAFE_RIGHT,
                        Key::Space => Actions::FLY_UP,
                        Key::LShift => Actions::FLY_DOWN,
                        _ => Actions::empty(),
                    };

                    match key_state {
                        ElementState::Pressed => camera.first_person.enable_actions(actions),
                        ElementState::Released => camera.first_person.disable_action(actions),
                    }
                }
            }
            _ => {}
        },
        DeviceEvent {
            event: MouseMotion { delta },
            ..
        } => {
            if !game_paused {
                camera
                    .first_person
                    .update_mouse(-delta.0 as f32, delta.1 as f32);
                window
                    .set_cursor_position(winit::dpi::PhysicalPosition::new(
                        gpu.config.width / 2,
                        gpu.config.height / 2,
                    ))
                    .ok();
            }
        }
        UserEvent(event) => match event {
            Close => *flow = ExitWithCode(0),
            NodeDefs(defs) => nodedefs = Some(defs),
            MapBlock(pos, blk) => {
                if let Some(map) = &mut map {
                    map.add_block(pos, blk);
                }
            }
            Media(files, finished) => {
                media.add_server_media(files);

                if finished {
                    map = Some(map::MapRender::new(
                        &mut gpu,
                        &camera,
                        &media,
                        nodedefs.take().unwrap_or_default(),
                    ));

                    net_events.send(NetEvent::Ready).ok();
                }
            }
            PlayerPos(pos, pitch, yaw) => {
                camera.first_person.position = pos.into();
                camera.first_person.pitch = Rad::<f32>::from(pitch).0;
                camera.first_person.yaw = Rad::<f32>::from(yaw).0;
            }
        },
        _ => {}
    });
}
