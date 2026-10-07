//! One owned stdio app-server child. Requests wait for acceptance, never for a turn.
use circular_runtime::PeerFailure;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::task::Waker;

pub(super) trait Transport: Send {
    fn open(&mut self) -> Result<(), PeerFailure>;
    fn set_waker(&mut self, waker: Waker);
    fn request(&mut self, method: &str, params: Value) -> Result<Value, PeerFailure>;
    fn notifications(&mut self) -> Result<Vec<Value>, PeerFailure>;
    fn close(&mut self);
}

#[derive(Default)]
pub(super) struct Proxy {
    connection: Option<Connection>,
    waker: Option<Waker>,
}

impl Proxy {
    fn connect(&mut self) -> Result<&mut Connection, PeerFailure> {
        if self.connection.is_none() {
            let mut child = Command::new("codex")
                .args(["app-server"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|_| PeerFailure::AdapterUnavailable)?;
            let writer = child.stdin.take().expect("piped stdin");
            let reader = BufReader::new(child.stdout.take().expect("piped stdout"));
            let mut connection = Connection::new(reader, writer, Some(child), self.waker.clone())?;
            connection.request(
                "initialize",
                json!({
                    "clientInfo": {"name": "circular", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true}
                }),
            )?;
            connection.write(&json!({"method": "initialized"}))?;
            self.connection = Some(connection);
        }
        Ok(self.connection.as_mut().expect("connected above"))
    }
}

impl Transport for Proxy {
    fn open(&mut self) -> Result<(), PeerFailure> {
        self.connect().map(|_| ())
    }

    fn set_waker(&mut self, waker: Waker) {
        if let Some(connection) = &mut self.connection {
            *connection.waker.lock().expect("waker lock") = Some(waker.clone());
        }
        self.waker = Some(waker);
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, PeerFailure> {
        self.connect()?.request(method, params)
    }

    fn notifications(&mut self) -> Result<Vec<Value>, PeerFailure> {
        match &mut self.connection {
            Some(connection) => connection.notifications(),
            None => Ok(Vec::new()),
        }
    }

    fn close(&mut self) {
        self.connection = None;
    }
}

/// The reader only demultiplexes this connection. It neither drives actors nor
/// acknowledges provider items. EOF wakes the owner just like a notification.
struct Connection {
    writer: Box<dyn Write + Send>,
    incoming: mpsc::Receiver<Result<Value, PeerFailure>>,
    pending: VecDeque<Value>,
    waker: Arc<Mutex<Option<Waker>>>,
    child: Option<Child>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_id: u64,
}

impl Connection {
    fn new(
        mut reader: impl BufRead + Send + 'static,
        writer: impl Write + Send + 'static,
        mut child: Option<Child>,
        waker: Option<Waker>,
    ) -> Result<Self, PeerFailure> {
        let (send, incoming) = mpsc::channel();
        let waker = Arc::new(Mutex::new(waker));
        let wake = waker.clone();
        let worker = std::thread::Builder::new()
            .name("codex-peer-read".into())
            .spawn(move || {
                loop {
                    let mut line = String::new();
                    let frame = match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => Err(PeerFailure::AdapterUnavailable),
                        Ok(_) => {
                            serde_json::from_str(&line).map_err(|_| PeerFailure::AdapterUnavailable)
                        }
                    };
                    let ended = frame.is_err();
                    if send.send(frame).is_err() {
                        break;
                    }
                    let waker = wake.lock().expect("waker lock").clone();
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                    if ended {
                        break;
                    }
                }
            });
        let reader = match worker {
            Ok(worker) => worker,
            Err(_) => {
                if let Some(child) = &mut child {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(PeerFailure::AdapterUnavailable);
            }
        };
        Ok(Self {
            writer: Box::new(writer),
            incoming,
            pending: VecDeque::new(),
            waker,
            child,
            reader: Some(reader),
            next_id: 1,
        })
    }

    fn write(&mut self, frame: &Value) -> Result<(), PeerFailure> {
        serde_json::to_writer(&mut self.writer, frame)
            .map_err(|_| PeerFailure::AdapterUnavailable)?;
        self.writer
            .write_all(b"\n")
            .and_then(|()| self.writer.flush())
            .map_err(|_| PeerFailure::AdapterUnavailable)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, PeerFailure> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"id": id, "method": method, "params": params}))?;
        loop {
            let frame = self
                .incoming
                .recv()
                .map_err(|_| PeerFailure::AdapterUnavailable)??;
            if frame.get("method").is_some() {
                if frame.get("id").is_some() {
                    self.write(&json!({"id": frame["id"], "error": {
                        "code": -32601, "message": "Method not found"
                    }}))?;
                } else {
                    self.pending.push_back(frame);
                }
                continue;
            }
            if frame["id"].as_u64() != Some(id) {
                return Err(PeerFailure::AdapterUnavailable);
            }
            if frame.get("error").is_some() {
                return Err(PeerFailure::AdapterUnavailable);
            }
            return frame
                .get("result")
                .cloned()
                .ok_or(PeerFailure::AdapterUnavailable);
        }
    }

    fn notifications(&mut self) -> Result<Vec<Value>, PeerFailure> {
        loop {
            match self.incoming.try_recv() {
                Ok(Ok(frame)) if frame.get("method").is_some() => {
                    if frame.get("id").is_some() {
                        self.write(&json!({"id": frame["id"], "error": {
                            "code": -32601, "message": "Method not found"
                        }}))?;
                    } else {
                        self.pending.push_back(frame);
                    }
                }
                Ok(Ok(_)) | Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(PeerFailure::AdapterUnavailable);
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(self.pending.drain(..).collect()),
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};

    struct Input {
        chunks: mpsc::Receiver<Vec<u8>>,
        bytes: std::io::Cursor<Vec<u8>>,
    }
    impl Read for Input {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            loop {
                let n = self.bytes.read(output)?;
                if n != 0 {
                    return Ok(n);
                }
                match self.chunks.recv() {
                    Ok(bytes) => self.bytes = std::io::Cursor::new(bytes),
                    Err(_) => return Ok(0),
                }
            }
        }
    }
    struct Output(mpsc::Sender<Vec<u8>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .send(bytes.to_vec())
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn pipe() -> (Output, BufReader<Input>) {
        let (send, chunks) = mpsc::channel();
        (
            Output(send),
            BufReader::new(Input {
                chunks,
                bytes: std::io::Cursor::new(Vec::new()),
            }),
        )
    }
    fn read(reader: &mut impl BufRead) -> Value {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        serde_json::from_str(&line).unwrap()
    }
    fn write(writer: &mut impl Write, frame: Value) {
        serde_json::to_writer(&mut *writer, &frame).unwrap();
        writer.write_all(b"\n").unwrap();
    }

    struct WakeSignal(mpsc::Sender<()>);
    impl std::task::Wake for WakeSignal {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }
    }

    #[test]
    fn codex_reader_wakes_on_notification_and_eof_without_polling_timer() {
        let (client_out, _server_in) = pipe();
        let (mut server_out, client_in) = pipe();
        let (wake, woken) = mpsc::channel();
        let mut connection = Connection::new(
            client_in,
            client_out,
            None,
            Some(Waker::from(Arc::new(WakeSignal(wake)))),
        )
        .unwrap();
        write(
            &mut server_out,
            json!({"method": "turn/completed", "params": {"threadId": "thread-A"}}),
        );
        woken.recv().unwrap();
        assert_eq!(
            connection.notifications().unwrap(),
            [json!({"method": "turn/completed", "params": {"threadId": "thread-A"}})]
        );
        drop(server_out);
        woken.recv().unwrap();
        assert_eq!(
            connection.notifications(),
            Err(PeerFailure::AdapterUnavailable)
        );
    }
}
