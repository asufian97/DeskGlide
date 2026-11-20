use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use deskglide_net::{
    load_or_create_identity, KvmClient, KvmServer, PinPolicy,
};
use deskglide_proto::{keycode, InputEvent, KeyState, Message, MouseButton, ScreenEdge};

fn temp_test_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("deskglide-test-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn valid_pin_and_secret_exchanges_datagrams_and_frames() {
    let dir = temp_test_dir("ok");
    let id = load_or_create_identity(Some(&dir)).unwrap();
    let server = KvmServer::bind("127.0.0.1:0".parse().unwrap(), &id).unwrap();
    let addr = server.local_addr().unwrap();

    let srv_task = tokio::spawn(async move {
        let mut s = server.accept_authenticated("K7M2-9X4P").await.unwrap();
        s.send_reliable(&Message::Control {
            active: true,
            entry_edge: ScreenEdge::Left,
        })
        .await
        .unwrap();
        s.send_mouse_datagram(14, -7).await.unwrap();
        s.send_reliable(&Message::Input(InputEvent::Key {
            code: keycode::KEY_A,
            state: KeyState::Press,
        }))
        .await
        .unwrap();
        s.send_reliable(&Message::Input(InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        }))
        .await
        .unwrap();
        s.send_reliable(&Message::ClipboardText("sync-text".into()))
            .await
            .unwrap();
        s.send_reliable(&Message::ReleaseAllKeys).await.unwrap();

        // Keep session open briefly until client finishes reading.
        let _ = s.recv().await;
    });

    let mut client = KvmClient::connect(
        addr,
        "K7M2-9X4P",
        PinPolicy::Exact(id.fingerprint),
        "test-client",
    )
    .await
    .unwrap();

    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push(client.recv().await.unwrap().unwrap());
    }
    client.send_reliable(&Message::EdgeReturn).await.unwrap();
    client.close(b"done").await;
    srv_task.await.unwrap();

    assert!(seen.contains(&Message::Control {
        active: true,
        entry_edge: ScreenEdge::Left,
    }));
    assert!(seen.contains(&Message::Input(InputEvent::MouseMove { dx: 14, dy: -7 })));
    assert!(seen.contains(&Message::Input(InputEvent::Key {
        code: keycode::KEY_A,
        state: KeyState::Press,
    })));
    assert!(seen.contains(&Message::Input(InputEvent::MouseButton {
        button: MouseButton::Left,
        pressed: true,
    })));
    assert!(seen.contains(&Message::ClipboardText("sync-text".into())));
    assert!(seen.contains(&Message::ReleaseAllKeys));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_fingerprint_rejected_at_tls_layer() {
    let dir = temp_test_dir("badfp");
    let id = load_or_create_identity(Some(&dir)).unwrap();
    let server = KvmServer::bind("127.0.0.1:0".parse().unwrap(), &id).unwrap();
    let addr = server.local_addr().unwrap();

    let srv_task = tokio::spawn(async move {
        let _ = server.accept_authenticated("ABCD-1234").await;
    });

    let bad_fp = [0xFFu8; 32];
    let res = KvmClient::connect(addr, "ABCD-1234", PinPolicy::Exact(bad_fp), "cli").await;
    assert!(res.is_err());

    srv_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_secret_rejected_at_auth_layer() {
    let dir = temp_test_dir("badpin");
    let id = load_or_create_identity(Some(&dir)).unwrap();
    let server = KvmServer::bind("127.0.0.1:0".parse().unwrap(), &id).unwrap();
    let addr = server.local_addr().unwrap();

    let srv_task = tokio::spawn(async move { server.accept_authenticated("RIGHT-PIN").await });

    let cli_res = KvmClient::connect(
        addr,
        "WRONG-PIN",
        PinPolicy::Exact(id.fingerprint),
        "cli",
    )
    .await;
    assert!(cli_res.is_err());
    assert!(srv_task.await.unwrap().is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn tofu_pins_once_then_rejects_changed_cert() {
    let dir1 = temp_test_dir("tofu1");
    let dir2 = temp_test_dir("tofu2");
    let known_hosts = dir1.join("known_hosts");

    let id1 = load_or_create_identity(Some(&dir1)).unwrap();
    let server1 = KvmServer::bind("127.0.0.1:0".parse().unwrap(), &id1).unwrap();
    let addr1 = server1.local_addr().unwrap();

    let s1 = tokio::spawn(async move {
        let mut sess = server1.accept_authenticated("SECRET").await.unwrap();
        let _ = sess.recv().await;
    });

    let mut cli1 = KvmClient::connect(
        addr1,
        "SECRET",
        PinPolicy::KnownOrTofu {
            known_hosts_path: known_hosts.clone(),
            peer_key: "lan-host".into(),
            allow_tofu: true,
        },
        "cli",
    )
    .await
    .unwrap();
    cli1.close(b"ok").await;
    s1.await.unwrap();

    // Second server with a DIFFERENT cert under the same peer_key must be rejected!
    let id2 = load_or_create_identity(Some(&dir2)).unwrap();
    let server2 = KvmServer::bind("127.0.0.1:0".parse().unwrap(), &id2).unwrap();
    let addr2 = server2.local_addr().unwrap();

    let s2 = tokio::spawn(async move {
        let _ = server2.accept_authenticated("SECRET").await;
    });

    let cli2 = KvmClient::connect(
        addr2,
        "SECRET",
        PinPolicy::KnownOrTofu {
            known_hosts_path: known_hosts.clone(),
            peer_key: "lan-host".into(),
            allow_tofu: true,
        },
        "cli",
    )
    .await;
    assert!(cli2.is_err());

    s2.abort();
    let _ = std::fs::remove_dir_all(&dir1);
    let _ = std::fs::remove_dir_all(&dir2);
}
