use crate::util::error::DynResult;
use log::error;
use std::future::Future;
use std::io;
use tokio::io::Interest;
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::{self, Duration};
use winapi::shared::winerror;

pub struct ConnectionWindows {
    send: Sender<Vec<u8>>,
    recv: Receiver<Vec<u8>>,
}

fn find_eol(a: &[u8], start: usize) -> Option<usize> {
    for p in start..a.len() {
        let c = a[p];
        if c == b'\r' || c == b'\n' {
            return Some(p);
        }
    }
    None
}

macro_rules! rw_pipe_def {
    ($name: ident, $P: ident) => {
        async fn $name(
            pipe: $P,
            recv: Sender<Vec<u8>>,
            mut send: Receiver<Vec<u8>>,
        ) -> DynResult<()> {
            let mut write_buffer: Option<Vec<u8>> = None;
            let mut read_buffer = Vec::with_capacity(200);
            loop {
                let interest = if write_buffer.is_some() {
                    Interest::READABLE | Interest::WRITABLE
                } else {
                    Interest::READABLE
                };
                tokio::select! {
                    ready = pipe.ready(interest) => {
                        match ready {
                            Ok(ready) => {
                                if ready.is_readable() {
                                    let mut pos = read_buffer.len();
                                    // Make room for more data
                                    read_buffer.resize(pos + 100, 0);

                                    match pipe.try_read(&mut read_buffer[pos..]) {
                                        Ok(n) => {
                                            read_buffer.truncate(pos + n);
                                            let mut start = 0;
                                            loop {
                                                if let Some(end) = find_eol(&read_buffer, pos) {
                                                    let line = &read_buffer[start .. end];
                                                    if !line.is_empty() {
                                                        if recv.send(line.to_vec()).await.is_err() {
                                                            return Ok(())
                                                        }
                                                    }
                                                    start = end +1;
                                                    if start == read_buffer.len() {
                                                        read_buffer.clear();
                                                    } else {
                                                        read_buffer.drain(0..start);
                                                    }
                                                    start =0;
                                                    pos = 0;
                                                } else {
                                                    break;
                                                }
                                            }
                                        },
                                        Err(e) => {
                                            read_buffer.truncate(pos);
                                            if e.kind() != io::ErrorKind::WouldBlock {
                                                return Err(e.into())
                                            }
                                        }
                                    }
                                }
                                if ready.is_writable() {
                                    if let Some(buffer) = &write_buffer {
                                        match pipe.try_write(&buffer) {
                                            Ok(_) => {
                                                write_buffer = None;
                                            },
                                            Err(e) => {
                                                if e.kind() != io::ErrorKind::WouldBlock {
                                                    return Err(e.into())
                                                }
                                            }
                                        }
                                    }
                                }

                            },
                            Err(e) => return Err(e.into())
                        }

                    },
                    res = (&mut send).recv() => {
                        match res {
                            Some(data) => write_buffer = Some(data),
                            None => return Ok(())
                        }
                    }
                }
            }
        }
    };
}

rw_pipe_def! {rw_pipe_client, NamedPipeClient}
rw_pipe_def! {rw_pipe_server, NamedPipeServer}

impl ConnectionWindows {
    pub async fn server<H, F, S>(path: &str, handler: H, _shutdown: S) -> DynResult<()>
    where
        H: Fn(ConnectionWindows) -> F,
        F: Future<Output = ()> + Send + 'static,
        S: Future<Output = ()> + Send + 'static,
    {
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(path)?;
        loop {
            server.connect().await?;
            let connected = server;
            server = ServerOptions::new().create(path)?;

            let (send_tx, send_rx) = mpsc::channel(3);
            let (recv_tx, recv_rx) = mpsc::channel(3);
            tokio::spawn(async move {
                if let Err(e) = rw_pipe_server(connected, recv_tx, send_rx).await {
                    error!("Server thread failed: {}", e);
                }
            });
            let conn = ConnectionWindows {
                send: send_tx,
                recv: recv_rx,
            };
            tokio::spawn(handler(conn));
        }
        //Ok(())
    }

    pub async fn client(path: &str) -> DynResult<ConnectionWindows> {
        let mut retries = 5;
        let client = loop {
            match ClientOptions::new().open(path) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(winerror::ERROR_PIPE_BUSY as i32) => {
                    // Try again
                    if retries == 0 {
                        return Err("Named pipe busy, too many retries".into());
                    }
                    retries -= 1;
                }
                Err(e) => return Err(e.into()),
            };
            time::sleep(Duration::from_millis(50)).await
        };
        let (send_tx, send_rx) = mpsc::channel(3);
        let (recv_tx, recv_rx) = mpsc::channel(3);

        tokio::spawn(async move {
            if let Err(e) = rw_pipe_client(client, recv_tx, send_rx).await {
                error!("Client thread failed: {}", e);
            }
        });
        let conn = ConnectionWindows {
            send: send_tx,
            recv: recv_rx,
        };

        Ok(conn)
    }

    pub async fn send_data(&mut self, data: &[u8]) -> DynResult<()> {
        self.send.send(data.to_vec()).await?;
        Ok(())
    }

    pub async fn recv_data(&mut self) -> DynResult<Vec<u8>> {
        self.recv.recv().await.ok_or("Receiver queue closed".into())
    }
}
