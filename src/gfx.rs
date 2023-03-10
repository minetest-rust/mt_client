use crate::{GfxEvent::*, NetEvent};
use cgmath::Rad;
use std::time::Instant;
use tokio::sync::mpsc;
use winit::{
    event::{DeviceEvent::*, Event::*, WindowEvent::*},
    event_loop::ControlFlow::ExitWithCode,
    platform::run_return::EventLoopExtRunReturn,
    window::CursorGrabMode,
};

mod map;
mod media;
mod state;
mod util;

pub async fn run(
    mut event_loop: winit::event_loop::EventLoop<crate::GfxEvent>,
    net_events: mpsc::UnboundedSender<NetEvent>,
) {
    let window = winit::window::WindowBuilder::new()
        .build(&event_loop)
        .unwrap();

    window
        .set_cursor_grab(CursorGrabMode::Locked)
        .or_else(|_e| window.set_cursor_grab(CursorGrabMode::Confined))
        .unwrap();

    window.set_cursor_visible(false);

    let mut state = state::State::new(&window).await;
    let mut map = None;
    let mut media = media::MediaMgr::new();

    let mut nodedefs = None;

    let mut last_frame = Instant::now();

    event_loop.run_return(move |event, _, flow| match event {
        MainEventsCleared => window.request_redraw(),
        RedrawRequested(id) if id == window.id() => {
            let now = Instant::now();
            let dt = now - last_frame;
            last_frame = now;

            state.update(dt);
            net_events
                .send(NetEvent::PlayerPos(
                    state.camera.position.into(),
                    Rad(state.camera.yaw).into(),
                    Rad(state.camera.pitch).into(),
                ))
                .ok();

            use wgpu::SurfaceError::*;
            match state.render(&map) {
                Ok(_) => {}
                Err(Lost) => state.configure_surface(),
                Err(OutOfMemory) => *flow = ExitWithCode(0),
                Err(err) => eprintln!("gfx error: {err:?}"),
            }
        }
        WindowEvent {
            ref event,
            window_id: id,
        } if id == window.id() => match event {
            CloseRequested => *flow = ExitWithCode(0),
            Resized(size) => state.resize(*size),
            ScaleFactorChanged { new_inner_size, .. } => state.resize(**new_inner_size),
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
                use winit::event::{ElementState::*, VirtualKeyCode as Key};

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
                    Pressed => state.camera.enable_actions(actions),
                    Released => state.camera.disable_action(actions),
                }
            }
            _ => {}
        },
        DeviceEvent {
            event: MouseMotion { delta },
            ..
        } => {
            state.camera.update_mouse(delta.0 as f32, delta.1 as f32);
            window
                .set_cursor_position(winit::dpi::PhysicalPosition::new(
                    state.config.width / 2,
                    state.config.height / 2,
                ))
                .ok();
        }
        UserEvent(event) => match event {
            Close => *flow = ExitWithCode(0),
            NodeDefs(defs) => nodedefs = Some(defs),
            MapBlock(pos, blk) => {
                if let Some(map) = map.as_mut() {
                    map.add_block(&mut state, pos, blk);
                }
            }
            Media(files, finished) => {
                media.add_server_media(files);

                if finished {
                    map = Some(map::MapRender::new(
                        &mut state,
                        &media,
                        nodedefs.take().unwrap_or_default(),
                    ));

                    net_events.send(NetEvent::Ready).ok();
                }
            }
            PlayerPos(pos, pitch, yaw) => {
                state.camera.position = pos.into();
                state.camera.pitch = Rad::<f32>::from(pitch).0;
                state.camera.yaw = Rad::<f32>::from(yaw).0;
            }
        },
        _ => {}
    });
}
