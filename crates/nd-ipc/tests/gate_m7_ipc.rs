mod gate_m7_ipc {
    use std::process::Command;
    use std::time::Duration;

    use nd_ipc::Endpoint;
    use nd_ipc::instance::{Instance, Launch, acquire};
    use nd_ipc::ladder::LadderConfig;
    use nd_ipc::rpc::{CallError, Client, ClientOptions, Method, Notification, RpcError, Server, Status, codes};
    use serde::{Deserialize, Serialize};

    struct Add;
    #[derive(Serialize, Deserialize)]
    struct Pair {
        a: i64,
        b: i64,
    }
    impl Method for Add {
        const NAME: &'static str = "add";
        type Params = Pair;
        type Output = i64;
    }

    struct Fail;
    impl Method for Fail {
        const NAME: &'static str = "fail";
        type Params = ();
        type Output = ();
    }

    struct Missing;
    impl Method for Missing {
        const NAME: &'static str = "missing";
        type Params = ();
        type Output = ();
    }

    struct Tick;
    impl Notification for Tick {
        const NAME: &'static str = "tick";
        type Params = u32;
    }

    fn server() -> Server {
        Server::new()
            .method::<Add, _, _>(|p| async move { Ok(p.a + p.b) })
            .method::<Fail, _, _>(|()| async { Err(RpcError::new(7, "nope")) })
    }

    fn fast() -> ClientOptions {
        let ladder = LadderConfig {
            base: Duration::from_millis(20),
            max: Duration::from_millis(80),
            stability_window: Duration::from_secs(30),
        };
        ClientOptions { ladder, call_timeout: Duration::from_secs(2) }
    }

    #[tokio::test]
    async fn typed_calls_errors_and_pushes() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at_path(dir.path().join("d.sock"));
        let handle = server().serve(&endpoint).unwrap();
        let client = Client::connect(&endpoint, fast());
        assert!(client.wait_connected(Duration::from_secs(2)).await);

        assert_eq!(client.call::<Add>(Pair { a: 2, b: 40 }).await.unwrap(), 42);
        match client.call::<Fail>(()).await {
            Err(CallError::Rpc(e)) => assert_eq!((e.code, e.message.as_str()), (7, "nope")),
            other => panic!("{other:?}"),
        }
        match client.call::<Missing>(()).await {
            Err(CallError::Rpc(e)) => assert_eq!(e.code, codes::METHOD_NOT_FOUND),
            other => panic!("{other:?}"),
        }

        let mut events = client.events();
        handle.notify::<Tick>(&9).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
        assert_eq!(event.decode::<Tick>(), Some(9));
    }

    #[tokio::test]
    async fn client_waits_for_a_late_server_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at_path(dir.path().join("d.sock"));
        let client = Client::connect(&endpoint, fast());
        let mut status = client.status();
        status.wait_for(|s| matches!(s, Status::Waiting { .. })).await.unwrap();
        assert!(matches!(client.call::<Add>(Pair { a: 1, b: 1 }).await, Err(CallError::NotConnected)));

        let first = server().serve(&endpoint).unwrap();
        assert!(client.wait_connected(Duration::from_secs(3)).await);
        assert_eq!(client.call::<Add>(Pair { a: 1, b: 2 }).await.unwrap(), 3);

        drop(first);
        status.wait_for(|s| matches!(s, Status::Waiting { .. })).await.unwrap();
        let _second = server().serve(&endpoint).unwrap();
        assert!(client.wait_connected(Duration::from_secs(3)).await);
        assert_eq!(client.call::<Add>(Pair { a: 5, b: 5 }).await.unwrap(), 10);
    }

    #[tokio::test]
    async fn second_launch_is_forwarded_in_process() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::at_path(dir.path().join("i.sock"));
        let first = Launch::from_args(["--a".into()]);
        let Instance::Primary(mut primary) = acquire(&endpoint, &first).await.unwrap() else { panic!("not primary") };

        let second = Launch::from_args(["file.txt".into(), "myapp://open/3".into()]);
        assert!(matches!(acquire(&endpoint, &second).await.unwrap(), Instance::Forwarded));
        let got = primary.next_launch().await.unwrap();
        assert_eq!(got.args, ["file.txt", "myapp://open/3"]);
        assert_eq!(got.urls, ["myapp://open/3"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_socket_from_a_crashed_primary_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("i.sock");
        // A bound-then-dropped listener leaves its file behind, as a killed process does.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let endpoint = Endpoint::at_path(&path);
        assert!(matches!(acquire(&endpoint, &Launch::default()).await.unwrap(), Instance::Primary(_)));
    }

    const CHILD_ENV: &str = "ND_IPC_TEST_CHILD_SOCKET";

    /// Run as a second process by `two_processes_forward_one_instance`; a no-op otherwise.
    #[tokio::test]
    async fn child_second_instance() {
        let Some(socket) = std::env::var_os(CHILD_ENV) else { return };
        let endpoint = Endpoint::at_path(socket);
        let launch = Launch::from_args(["from-child".into(), "myapp://child".into()]);
        match acquire(&endpoint, &launch).await.unwrap() {
            Instance::Forwarded => {}
            Instance::Primary(_) => panic!("child became primary"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_processes_forward_one_instance() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("i.sock");
        let endpoint = Endpoint::at_path(&socket);
        let Instance::Primary(mut primary) = acquire(&endpoint, &Launch::default()).await.unwrap() else {
            panic!("not primary")
        };

        let exe = std::env::current_exe().unwrap();
        let status = tokio::task::spawn_blocking(move || {
            Command::new(exe)
                .args(["--exact", "gate_m7_ipc::child_second_instance", "--nocapture"])
                .env(CHILD_ENV, &socket)
                .status()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(status.success());

        let got = tokio::time::timeout(Duration::from_secs(2), primary.next_launch()).await.unwrap().unwrap();
        assert_eq!(got.args, ["from-child", "myapp://child"]);
        assert_eq!(got.urls, ["myapp://child"]);
    }
}
