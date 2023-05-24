use crate::{GfxEvent::*, NetEvent};
use std::time::Instant;
use tokio::sync::mpsc;
use winit::{
    event::{DeviceEvent::*, Event::*, WindowEvent::*},
    event_loop::ControlFlow::ExitWithCode,
    platform::run_return::EventLoopExtRunReturn,
    window::CursorGrabMode,
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

    let update_cursor_mode = |game_paused| {
        let modes: &[CursorGrabMode] = if game_paused {
            &[CursorGrabMode::None]
        } else {
            &[CursorGrabMode::Confined, CursorGrabMode::Locked]
        };

        for mode in modes {
            if window.set_cursor_grab(*mode).is_ok() {
                return;
            }
        }
    };

    update_cursor_mode(game_paused);

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
                .send(NetEvent::PlayerPos(camera.pos, camera.rot.y, camera.rot.z))
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
            Focused(false) => camera.input = Default::default(),
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
                use winit::event::{ElementState, VirtualKeyCode as Key};

                if key == Key::Escape && key_state == ElementState::Pressed {
                    game_paused = !game_paused;
                    window.set_cursor_visible(game_paused);
                    update_cursor_mode(game_paused);

                    if game_paused {
                        camera.input = Default::default();
                    }
                }

                if game_paused {
                    return;
                }

                if key == Key::F3 && key_state == ElementState::Pressed {
                    debug_menu.enabled = !debug_menu.enabled;
                }

                if !game_paused {
                    *(match key {
                        Key::W => &mut camera.input.forward,
                        Key::A => &mut camera.input.left,
                        Key::S => &mut camera.input.backward,
                        Key::D => &mut camera.input.right,
                        Key::Space => &mut camera.input.jump,
                        Key::LShift => &mut camera.input.sneak,
                        _ => return,
                    }) = key_state == ElementState::Pressed;
                }
            }
            _ => {}
        },
        DeviceEvent {
            event: MouseMotion { delta },
            ..
        } => {
            if !game_paused {
                camera.input.mouse_x += delta.0 as f32;
                camera.input.mouse_y += delta.1 as f32;

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
                camera.pos = pos;
                camera.rot.y = yaw;
                camera.rot.z = pitch;
            }
            MovementSpeed(speed) => {
                camera.speed = speed;
            }
        },
        _ => {}
    });
}
