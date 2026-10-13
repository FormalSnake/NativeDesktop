#![cfg(unix)]

mod gate_m7_discord {
    use std::path::Path;
    use std::time::Duration;

    use discord_rpc::{Activity, DiscordPlugin, DiscordStatus, DiscordStatusFeed, PresenceFeed};
    use nd_ipc::ladder::LadderConfig;
    use nd_plugin::{PluginHost, Services};
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::watch;

    async fn read_frame(s: &mut UnixStream) -> (u32, Value) {
        let mut h = [0u8; 8];
        s.read_exact(&mut h).await.unwrap();
        let op = u32::from_le_bytes(h[..4].try_into().unwrap());
        let len = u32::from_le_bytes(h[4..].try_into().unwrap()) as usize;
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.unwrap();
        (op, serde_json::from_slice(&body).unwrap())
    }

    async fn write_frame(s: &mut UnixStream, op: u32, v: Value) {
        let body = serde_json::to_vec(&v).unwrap();
        s.write_all(&op.to_le_bytes()).await.unwrap();
        s.write_all(&(body.len() as u32).to_le_bytes()).await.unwrap();
        s.write_all(&body).await.unwrap();
    }

    fn fast() -> LadderConfig {
        LadderConfig {
            base: Duration::from_millis(20),
            max: Duration::from_millis(60),
            stability_window: Duration::from_secs(30),
        }
    }

    struct Rig {
        host: PluginHost,
        activity: watch::Sender<Option<Activity>>,
        status: watch::Receiver<DiscordStatus>,
    }

    fn rig(dir: &Path, section: &str) -> Rig {
        let (activity, rx) = watch::channel(None);
        let services = Services::new();
        services.insert(PresenceFeed(rx));
        let mut host = PluginHost::new("gate-discord", tokio::runtime::Handle::current(), services.clone());
        host.register(DiscordPlugin::with_ladder(fast()));
        let mut root: toml::Table =
            format!("[plugins.discord]\nsocket_dir = {:?}\n{section}", dir.to_str().unwrap()).parse().unwrap();
        let report = host.load(&mut root);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        let status = services.get::<DiscordStatusFeed>().unwrap().0.clone();
        Rig { host, activity, status }
    }

    fn song() -> Activity {
        Activity {
            details: Some("Song".into()),
            state: Some("Artist".into()),
            started_at: Some(1_700_000_000),
            ..Activity::default()
        }
    }

    #[tokio::test]
    async fn without_discord_the_plugin_searches_quietly_then_connects_when_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let mut rig = rig(dir.path(), "client_id = \"123\"\n");
        rig.activity.send(Some(song())).unwrap();

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(*rig.status.borrow(), DiscordStatus::Searching);

        let listener = UnixListener::bind(dir.path().join("discord-ipc-0")).unwrap();
        let (mut conn, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
        let (op, hello) = read_frame(&mut conn).await;
        assert_eq!((op, hello["v"].as_i64(), hello["client_id"].as_str()), (0, Some(1), Some("123")));
        write_frame(&mut conn, 1, json!({ "cmd": "DISPATCH", "evt": "READY", "data": {} })).await;

        let (op, set) = read_frame(&mut conn).await;
        assert_eq!(op, 1);
        assert_eq!(set["cmd"], "SET_ACTIVITY");
        assert_eq!(set["args"]["activity"]["details"], "Song");
        assert_eq!(set["args"]["activity"]["timestamps"]["start"], 1_700_000_000);
        assert!(set["args"]["pid"].as_u64().is_some());
        rig.status.wait_for(|s| *s == DiscordStatus::Connected).await.unwrap();

        // Updates stream through, a ping is answered, and clearing sends a null activity.
        rig.activity.send(Some(Activity { details: Some("Next".into()), ..song() })).unwrap();
        let (_, set) = read_frame(&mut conn).await;
        assert_eq!(set["args"]["activity"]["details"], "Next");
        write_frame(&mut conn, 3, json!({ "nonce": "p" })).await;
        let (op, pong) = read_frame(&mut conn).await;
        assert_eq!((op, pong["nonce"].as_str()), (4, Some("p")));
        rig.activity.send(None).unwrap();
        let (_, clear) = read_frame(&mut conn).await;
        assert!(clear["args"]["activity"].is_null());
        rig.host.shutdown();
    }

    #[tokio::test]
    async fn reconnects_after_discord_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("discord-ipc-0");
        let mut rig = rig(dir.path(), "client_id = \"9\"\n");
        rig.activity.send(Some(song())).unwrap();

        for _ in 0..2 {
            let listener = UnixListener::bind(&path).unwrap();
            let (mut conn, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
            read_frame(&mut conn).await;
            write_frame(&mut conn, 1, json!({ "evt": "READY" })).await;
            let (_, set) = read_frame(&mut conn).await;
            assert_eq!(set["args"]["activity"]["state"], "Artist");
            drop(conn);
            drop(listener);
            std::fs::remove_file(&path).unwrap();
        }
        rig.host.shutdown();
    }

    #[tokio::test]
    async fn no_client_id_stays_idle_and_a_typo_in_the_section_does_not_start_it() {
        let dir = tempfile::tempdir().unwrap();
        let rig = rig(dir.path(), "");
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(*rig.status.borrow(), DiscordStatus::Idle);

        let (_tx, rx) = watch::channel(None);
        let services = Services::new();
        services.insert(PresenceFeed(rx));
        let mut host = PluginHost::new("gate-discord", tokio::runtime::Handle::current(), services);
        host.register(DiscordPlugin::new());
        let mut root: toml::Table = "[plugins.discord]\nclient-id = \"1\"\n".parse().unwrap();
        let report = host.load(&mut root);
        assert_eq!(report.failed.len(), 1);
    }
}
