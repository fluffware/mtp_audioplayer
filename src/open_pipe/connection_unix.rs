use log::{debug, error, warn};
use std::fs::{create_dir_all, remove_file};
use std::future::Future;
use std::path::Path;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;
use tokio::net::{unix::OwnedWriteHalf, UnixListener, UnixStream};
use tokio::pin;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::io::AsyncWriteExt;

pub type DynResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

pub struct ConnectionUnix {
    stream: OwnedWriteHalf,
    recv: Receiver<Vec<u8>>,
}

async fn read_connection<R>(r: R, send: Sender<Vec<u8>>)
where
    R: AsyncRead + Unpin,
{
    let mut r = BufReader::new(r);
    loop {
        let mut line = String::new();
        match r.read_line(&mut line).await {
            Err(e) => {
                error!("Failed to read line from pipe: {}", e);
                break;
            }
            Ok(l) => {
                if l == 0 {
                    break;
                }
                debug!("Got line: {}", line);
                send.send(line.as_bytes().to_vec()).await.unwrap();
            }
        }
    }
}

impl ConnectionUnix {
    fn from_stream(stream: UnixStream) -> ConnectionUnix {
        let (r, w) = stream.into_split();
        let (msg_in, msg_out) = mpsc::channel(10);
        tokio::spawn(read_connection(r, msg_in));
        ConnectionUnix {
            stream: w,
            recv: msg_out,
        }
    }

    pub async fn server<H, F, S>(path: &str, handler: H, shutdown: S) -> DynResult<()>
    where
        H: Fn(ConnectionUnix) -> F,
        F: Future<Output = ()> + Send + 'static,
        S: Future<Output = ()> + Send + 'static,
    {
        if let Some(parent) = Path::new(path).parent() {
            create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(path)?;
        pin!(shutdown);
        loop {
            tokio::select! {
		res = listener.accept() => {
                    if let Ok((stream, _addr)) = res {
			let conn = ConnectionUnix::from_stream(stream);
			tokio::spawn(handler(conn));
                    } else {
			error!("Failed to accept connection");
                    }
		},
		_ = (&mut shutdown) => break
            }
        }
	if let Err(e) = remove_file(path) {
	    warn!("Failed to delete named pipe {}: {}", path, e);
	}

	debug!("Server exited");
	Ok(())
    }

    pub async fn client(path: &str) -> DynResult<ConnectionUnix> {
        let stream = UnixStream::connect(path).await?;
        Ok(Self::from_stream(stream))
	    
    }

    pub async fn send_data(&mut self, data: &[u8]) -> DynResult<()> {
	self.stream.write_all(data).await?;
	self.stream.flush().await?;
	Ok(())
    }

    pub async fn recv_data(&mut self) -> DynResult<Vec<u8>> {
	self.recv.recv().await.ok_or("Receiver queue closed".into())
    }
}
