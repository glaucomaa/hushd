use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ipc::{Request, Response};

struct Daemon {
    child: Child,
    socket: PathBuf,
}

impl Daemon {
    fn start(name: &str) -> Self {
        let socket =
            std::env::temp_dir().join(format!("m4d-test-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(env!("CARGO_BIN_EXE_m4d"))
            .env("M4D_SOCKET", &socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn m4d");

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if UnixStream::connect(&socket).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Self { child, socket }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[test]
fn daemon_answers_status_and_stops_on_shutdown() {
    let daemon = Daemon::start("status");

    let response = ipc::request(&daemon.socket, &Request::Status).expect("status reply");
    match response {
        Response::Status(status) => {
            assert!(
                status.connected || status.error.is_some(),
                "status must explain the state"
            );
        }
        other => panic!("unexpected reply: {other:?}"),
    }

    let response = ipc::request(&daemon.socket, &Request::Shutdown).expect("shutdown reply");
    assert_eq!(response, Response::Ok);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if UnixStream::connect(&daemon.socket).is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not exit after shutdown"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn daemon_rejects_garbage_without_dying() {
    let daemon = Daemon::start("garbage");

    let mut stream = UnixStream::connect(&daemon.socket).expect("connect");
    stream.write_all(b"not json\n").expect("write");
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).expect("reply");
    assert!(line.contains("bad request"), "unexpected reply: {line}");

    let response = ipc::request(&daemon.socket, &Request::Status).expect("still serving");
    assert!(matches!(response, Response::Status(_)));
}
