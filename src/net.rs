use crate::{GfxEvent, NetEvent};
use cgmath::{Deg, Point3, Vector3};
use futures::future::OptionFuture;
use mt_net::{CltSender, ReceiverExt, SenderExt, ToCltPkt, ToSrvPkt};
use rand::RngCore;
use sha2::Sha256;
use srp::{client::SrpClient, groups::G_2048};
use std::{future::Future, time::Duration};
use tokio::{
    sync::mpsc,
    time::{interval, Instant, Interval},
};
use winit::event_loop::EventLoopProxy;

enum AuthState {
    Init(Interval),
    Verify(Vec<u8>, SrpClient<'static, Sha256>),
    Done,
}

struct Conn {
    tx: CltSender,
    auth: AuthState,
    send_pos_iv: Option<Interval>,
    username: String,
    password: String,
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
        tx,
        auth: AuthState::Init(interval(Duration::from_millis(100))),
        send_pos_iv: None,
        username: "shrek".into(), // shrek is love, shrek is life <3
        password: "boobies".into(),
        pos: Point3::new(0.0, 0.0, 0.0),
        pitch: Deg(0.0),
        yaw: Deg(0.0),
        events: evt_out,
    };

    let init_pkt = ToSrvPkt::Init {
        serialize_version: 29,
        proto_version: 40..=40,
        player_name: conn.username.clone(),
        send_full_item_meta: false,
    };

    let worker_thread = tokio::spawn(worker.run());

    loop {
        tokio::select! {
            pkt = rx.recv() => match pkt {
                None => break,
                Some(Err(e)) => eprintln!("{e}"),
                Some(Ok(v)) => conn.handle_pkt(v).await,
            },
            Some(_) = maybe_tick(match &mut conn.auth {
                AuthState::Init(iv) => Some(iv),
                _ => None,
            }) => {
                conn.tx.send(&init_pkt).await.unwrap();
            }
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

    conn.events.send_event(GfxEvent::Close).ok(); // TODO: make sure to send this on panic
    worker_thread.await.unwrap();
}

impl Conn {
    async fn handle_pkt(&mut self, pkt: ToCltPkt) {
        use ToCltPkt::*;

        match pkt {
            Hello {
                auth_methods,
                username: name,
                ..
            } => {
                use mt_net::AuthMethod;

                if !matches!(self.auth, AuthState::Init(_)) {
                    return;
                }

                let srp = SrpClient::<Sha256>::new(&G_2048);

                let mut rand_bytes = vec![0; 32];
                rand::thread_rng().fill_bytes(&mut rand_bytes);

                if self.username != name {
                    panic!("username changed");
                }

                if auth_methods.contains(AuthMethod::FirstSrp) {
                    let verifier = srp.compute_verifier(
                        self.username.to_lowercase().as_bytes(),
                        self.password.as_bytes(),
                        &rand_bytes,
                    );

                    self.tx
                        .send(&ToSrvPkt::FirstSrp {
                            salt: rand_bytes,
                            verifier,
                            empty_passwd: self.password.is_empty(),
                        })
                        .await
                        .unwrap();

                    self.auth = AuthState::Done;
                } else if auth_methods.contains(AuthMethod::Srp) {
                    let a = srp.compute_public_ephemeral(&rand_bytes);

                    self.tx
                        .send(&ToSrvPkt::SrpBytesA { a, no_sha1: true })
                        .await
                        .unwrap();

                    self.auth = AuthState::Verify(rand_bytes, srp);
                } else {
                    panic!("unsupported auth methods: {auth_methods:?}");
                }
            }
            SrpBytesSaltB { salt, b } => {
                if let AuthState::Verify(a, srp) = &self.auth {
                    let m = srp
                        .process_reply(
                            a,
                            self.username.to_lowercase().as_bytes(),
                            self.password.as_bytes(),
                            &salt,
                            &b,
                        )
                        .unwrap()
                        .proof()
                        .into();

                    self.tx.send(&ToSrvPkt::SrpBytesM { m }).await.unwrap();

                    self.auth = AuthState::Done;
                }
            }
            NodeDefs(defs) => {
                self.events.send_event(GfxEvent::NodeDefs(defs.0)).ok();
            }
            Kick(reason) => {
                println!("kicked: {reason}");
            }
            AcceptAuth { player_pos, .. } => {
                self.tx
                    .send(&ToSrvPkt::Init2 {
                        lang: "en_US".into(), // localization is unironically overrated
                    })
                    .await
                    .unwrap();

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
            _ => {}
        }
    }
}
