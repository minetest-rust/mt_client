use crate::{GfxEvent, NetEvent};
use cgmath::{Deg, Point3, Vector3};
use futures::future::OptionFuture;
use mt_net::{CltSender, ReceiverExt, SenderExt, ToCltPkt, ToSrvPkt};
use std::{future::Future, time::Duration};
use tokio::{
    sync::mpsc,
    time::{interval, Instant, Interval},
};
use winit::event_loop::EventLoopProxy;

struct Conn {
    tx: CltSender,
    auth: mt_auth::Auth,
    send_pos_iv: Option<Interval>,
    pos: Point3<f32>,
    pitch: Deg<f32>,
    yaw: Deg<f32>,
    events: EventLoopProxy<GfxEvent>,
}

fn maybe_tick(iv: Option<&mut Interval>) -> OptionFuture<impl Future<Output = Instant> + '_> {
    OptionFuture::from(iv.map(Interval::tick))
}

pub(crate) async fn run(
    evt_out: EventLoopProxy<GfxEvent>,
    mut evt_in: mpsc::UnboundedReceiver<NetEvent>,
) {
    let (tx, mut rx, worker) = mt_net::connect("localhost:30000").await.unwrap();

    let mut conn = Conn {
        auth: mt_auth::Auth::new(tx.clone(), "shrek", "boobies", "en_US"),
        tx,
        send_pos_iv: None,
        pos: Point3::new(0.0, 0.0, 0.0),
        pitch: Deg(0.0),
        yaw: Deg(0.0),
        events: evt_out,
    };

    let worker_thread = tokio::spawn(worker.run());

    loop {
        tokio::select! {
            pkt = rx.recv() => match pkt {
                None => break,
                Some(Err(e)) => eprintln!("{e}"),
                Some(Ok(v)) => conn.handle_pkt(v).await,
            },
            _ = conn.auth.poll() => {}
            Some(_) = maybe_tick(conn.send_pos_iv.as_mut()) => {
                conn.tx
                    .send(&ToSrvPkt::PlayerPos(mt_net::PlayerPos {
                        pos: conn.pos,
                        vel: Vector3::new(0.0, 0.0, 0.0),
                        pitch: conn.pitch,
                        yaw: conn.yaw,
                        keys: mt_net::enumset::EnumSet::empty(),
                        fov: Deg(90.0).into(),
                        wanted_range: 12,
                    }))
                    .await
                    .unwrap();
            }
            evt = evt_in.recv() => {
                match evt {
                    Some(NetEvent::PlayerPos(pos, yaw, pitch)) => {
                        conn.pos = pos;
                        conn.yaw = yaw;
                        conn.pitch = pitch;
                    },
                    Some(NetEvent::Ready) => {
                        conn.tx
                            .send(&ToSrvPkt::CltReady {
                                major: 0,
                                minor: 1,
                                patch: 0,
                                reserved: 0,
                                version: format!("Minetest Rust {}", env!("CARGO_PKG_VERSION")),
                                formspec: 4,
                            })
                            .await
                            .unwrap();
                    }
                    None => conn.tx.close(),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                conn.tx.close();
            }
        }
    }

    worker_thread.await.unwrap();
}

impl Conn {
    async fn handle_pkt(&mut self, pkt: ToCltPkt) {
        use ToCltPkt::*;

        self.auth.handle_pkt(&pkt).await;

        match pkt {
            NodeDefs(defs) => {
                self.events.send_event(GfxEvent::NodeDefs(defs.0)).ok();
            }
            Kick(reason) => {
                println!("kicked: {reason}");
            }
            AcceptAuth { player_pos, .. } => {
                self.pos = player_pos;
                self.send_pos_iv = Some(interval(Duration::from_millis(100)));
            }
            MovePlayer { pos, pitch, yaw } => {
                self.pos = pos;
                self.pitch = pitch;
                self.yaw = yaw;

                self.events
                    .send_event(GfxEvent::PlayerPos(self.pos, self.pitch, self.yaw))
                    .ok();
            }
            BlockData { pos, block } => {
                self.events.send_event(GfxEvent::MapBlock(pos, block)).ok();
                self.tx
                    .send(&ToSrvPkt::GotBlocks {
                        blocks: Vec::from([pos]),
                    })
                    .await
                    .unwrap();
            }
            AnnounceMedia { files, .. } => {
                self.tx
                    .send(&ToSrvPkt::RequestMedia {
                        filenames: files.into_keys().collect(), // TODO: cache
                    })
                    .await
                    .ok();
            }
            Media { files, n, i } => {
                self.events
                    .send_event(GfxEvent::Media(files, i + 1 == n))
                    .ok();
            }
            ChatMsg { text, .. } => {
                println!("{text}");
            }
            Movement { walk_speed, .. } => {
                self.events
                    .send_event(GfxEvent::MovementSpeed(walk_speed))
                    .ok();
            }
            _ => {}
        }
    }
}
