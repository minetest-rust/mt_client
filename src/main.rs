use mt_net::{MtReceiver, MtSender, RemoteSrv, ToCltPkt, ToSrvPkt};
use rand::RngCore;
use sha2::Sha256;
use srp::{client::SrpClient, groups::G_2048};
use std::time::Duration;
use tokio::sync::oneshot;

async fn handle(tx: MtSender<RemoteSrv>, rx: &mut MtReceiver<RemoteSrv>) {
    let mut username = "hydra".to_string();
    let password = "password";

    let (init_tx, mut init_rx) = oneshot::channel();

    {
        let tx = tx.clone();
        let pkt = ToSrvPkt::Init {
            serialize_version: 29,
            min_proto_version: 40,
            max_proto_version: 40,
            player_name: username.clone(),
            send_full_item_meta: false,
        };

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            while tokio::select! {
                _ = &mut init_rx => false,
                _ = interval.tick() => true,
            } {
                tx.send(&pkt).await.unwrap()
            }
        });
    }

    let mut init_tx = Some(init_tx);
    let mut auth = None;

    while let Some(res) = rx.recv().await {
        match res {
            Ok(pkt) => {
                use ToCltPkt::*;

                match pkt {
                    Hello {
                        auth_methods,
                        username: name,
                        ..
                    } => {
                        use mt_net::AuthMethod;

                        if let Some(chan) = init_tx.take() {
                            chan.send(()).unwrap();

                            let client = SrpClient::<Sha256>::new(&G_2048);

                            let mut rand_bytes = vec![0; 32];
                            rand::thread_rng().fill_bytes(&mut rand_bytes);

                            username = name;

                            if auth_methods.contains(AuthMethod::FirstSrp) {
                                let verifier = client.compute_verifier(
                                    username.to_lowercase().as_bytes(),
                                    password.as_bytes(),
                                    &rand_bytes,
                                );

                                tx.send(&ToSrvPkt::FirstSrp {
                                    salt: rand_bytes,
                                    verifier,
                                    empty_passwd: password.is_empty(),
                                })
                                .await
                                .unwrap();
                            } else if auth_methods.contains(AuthMethod::Srp) {
                                let a = client.compute_public_ephemeral(&rand_bytes);
                                auth = Some((rand_bytes, client));

                                tx.send(&ToSrvPkt::SrpBytesA { a, no_sha1: true })
                                    .await
                                    .unwrap();
                            } else {
                                panic!("unsupported auth methods: {auth_methods:?}");
                            }
                        }
                    }
                    SrpBytesSaltB { salt, b } => {
                        if let Some((a, client)) = auth.take() {
                            let m = client
                                .process_reply(
                                    &a,
                                    username.to_lowercase().as_bytes(),
                                    password.as_bytes(),
                                    &salt,
                                    &b,
                                )
                                .unwrap()
                                .proof()
                                .into();

                            tx.send(&ToSrvPkt::SrpBytesM { m }).await.unwrap();
                        }
                    }
                    x => println!("{x:?}"),
                }
            }
            Err(err) => eprintln!("{err}"),
        }
    }
}

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mt_net::connect("localhost:30000").await.unwrap();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => println!("canceled"),
        _ = handle(tx, &mut rx) => {
            println!("disconnected");
        }
    }

    rx.close().await;
}
