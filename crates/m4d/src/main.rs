mod worker;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use ipc::{Event, Request, Response, Status};

fn main() -> ExitCode {
    let (listener, path) = match bind(ipc::socket_path()) {
        Ok(bound) => bound,
        Err(error) => {
            eprintln!("m4d: cannot bind {}: {error}", ipc::socket_path().display());
            let fallback =
                std::path::PathBuf::from(format!("/tmp/m4d-{}.sock", std::process::id()));
            match bind(fallback) {
                Ok(bound) => bound,
                Err(error) => {
                    eprintln!("m4d: cannot bind a fallback socket either: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    eprintln!("m4d: listening on {}", path.display());

    let state = Arc::new(RwLock::new(Status::default()));
    let subscribers: Arc<Mutex<Vec<Sender<Event>>>> = Arc::default();
    let (commands, receiver) = mpsc::channel();
    let worker = worker::spawn(receiver, state.clone(), subscribers.clone());
    std::thread::spawn(move || {
        let _ = worker.join();
        eprintln!("m4d: worker stopped; exiting so the supervisor can restart it");
        std::process::exit(1);
    });

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => serve(stream, commands.clone(), subscribers.clone(), state.clone()),
            Err(error) => eprintln!("m4d: accept failed: {error}"),
        }
    }
    ExitCode::SUCCESS
}

fn bind(path: std::path::PathBuf) -> std::io::Result<(UnixListener, std::path::PathBuf)> {
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok((listener, path))
}

fn serve(
    stream: UnixStream,
    commands: Sender<worker::Command>,
    subscribers: Arc<Mutex<Vec<Sender<Event>>>>,
    state: Arc<RwLock<Status>>,
) {
    std::thread::spawn(move || {
        let Ok(mut writer) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let request: Request = match serde_json::from_str(line.trim()) {
            Ok(request) => request,
            Err(error) => {
                let _ = reply(
                    &mut writer,
                    &Response::Error {
                        message: format!("bad request: {error}"),
                    },
                );
                return;
            }
        };

        if matches!(request, Request::Watch) {
            let (sender, receiver) = mpsc::channel();
            if let Ok(mut list) = subscribers.lock() {
                list.push(sender);
            }
            let snapshot = Event::State {
                status: state.read().map(|s| s.clone()).unwrap_or_default(),
            };
            if write_event(&mut writer, &snapshot).is_err() {
                return;
            }
            for event in receiver {
                if write_event(&mut writer, &event).is_err() {
                    return;
                }
            }
            return;
        }

        let shutdown = matches!(request, Request::Shutdown);
        let (reply_tx, reply_rx) = mpsc::channel();
        if commands
            .send(worker::Command::Request {
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            let _ = reply(
                &mut writer,
                &Response::Error {
                    message: "daemon is shutting down".to_owned(),
                },
            );
            return;
        }
        match reply_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(response) => {
                let _ = reply(&mut writer, &response);
                if shutdown {
                    let _ = fs::remove_file(ipc::socket_path());
                    std::process::exit(0);
                }
            }
            Err(_) => {
                let _ = reply(
                    &mut writer,
                    &Response::Error {
                        message: "daemon did not answer".to_owned(),
                    },
                );
            }
        }
    });
}

fn reply(writer: &mut UnixStream, response: &Response) -> std::io::Result<()> {
    let line = serde_json::to_string(response).map_err(std::io::Error::other)?;
    writeln!(writer, "{line}")
}

fn write_event(writer: &mut UnixStream, event: &Event) -> std::io::Result<()> {
    let line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    writeln!(writer, "{line}")
}
