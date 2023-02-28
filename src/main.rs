mod gfx;
mod net;

use cgmath::{Deg, Point3};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum GfxEvent {
    Close,
    Media(HashMap<String, Vec<u8>>, bool),
    NodeDefs(HashMap<u16, mt_net::NodeDef>),
    MapBlock(Point3<i16>, Box<mt_net::MapBlock>),
    PlayerPos(Point3<f32>, Deg<f32>, Deg<f32>),
}

#[derive(Debug, Clone)]
pub enum NetEvent {
    PlayerPos(Point3<f32>, Deg<f32>, Deg<f32>),
    Ready,
}

fn main() {
    println!(include_str!("../assets/ascii-art.txt"));
    println!("Early WIP. Expext breakage. Trans rights <3");

    let (net_tx, net_rx) = mpsc::unbounded_channel();
    let event_loop = winit::event_loop::EventLoopBuilder::<GfxEvent>::with_user_event().build();
    let event_loop_proxy = event_loop.create_proxy();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .thread_name("network")
        .build()
        .unwrap();

    let net_thread = runtime.spawn(net::run(event_loop_proxy, net_rx));

    // graphics code is pseudo async: the winit event loop is blocking
    // so we can't really use async capabilities
    futures::executor::block_on(gfx::run(event_loop, net_tx));

    // wait for net to finish
    runtime.block_on(net_thread).unwrap();
}
