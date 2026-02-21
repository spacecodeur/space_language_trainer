use anyhow::{Context, Result};
use std::io::{BufReader, BufWriter};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use space_lt_common::protocol::{ServerMsg, read_server_msg};
use space_lt_common::{info, warn};

// Re-export from common for use by main.rs
pub use space_lt_common::protocol::is_disconnect;

/// Connect timeout for TCP connection attempts.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum reconnection attempts with exponential backoff.
const MAX_CONNECT_ATTEMPTS: u32 = 3;
/// Exponential backoff delays in seconds (1s, 2s, 4s).
const BACKOFF_SECS: [u64; 3] = [1, 2, 4];

/// TCP connection to the server, replacing the old SSH-based RemoteTranscriber.
pub struct TcpConnection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl TcpConnection {
    /// Connect to the server at the given address, wait for Ready handshake.
    ///
    /// Times out after 10 seconds if the server is unreachable.
    pub fn connect(addr: &str) -> Result<Self> {
        info!("[client] Connecting to {addr}...");

        let socket_addr: SocketAddr = addr
            .parse()
            .context("invalid server address (expected IP:port)")?;
        let stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)
            .context("connecting to server")?;

        // Disable Nagle's algorithm for low-latency audio streaming
        stream.set_nodelay(true).context("setting TCP_NODELAY")?;

        let reader = BufReader::new(
            stream
                .try_clone()
                .context("cloning TCP stream for reader")?,
        );
        let writer = BufWriter::new(stream);

        let mut conn = Self { reader, writer };

        // Wait for Ready from server
        let msg = conn.read_server_msg().context("waiting for server Ready")?;
        match msg {
            ServerMsg::Ready => info!("[client] Server ready"),
            other => anyhow::bail!("Expected Ready, got {other:?}"),
        }

        Ok(conn)
    }

    /// Connect with exponential backoff retry (1s, 2s, 4s), max 3 attempts.
    ///
    /// Useful when the server may not be ready at client startup, or after a
    /// TCP connection drop.
    pub fn connect_with_retry(addr: &str) -> Result<Self> {
        let mut last_err = None;
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            if attempt > 1 {
                let delay = BACKOFF_SECS[attempt as usize - 2];
                info!("[client] Retrying in {delay}s...");
                std::thread::sleep(Duration::from_secs(delay));
            }
            match Self::connect(addr) {
                Ok(conn) => return Ok(conn),
                Err(e) => {
                    warn!(
                        "[client] Connection attempt {attempt}/{MAX_CONNECT_ATTEMPTS} failed: {e:#}"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err
            .unwrap()
            .context("all TCP connection attempts failed"))
    }

    /// Read the next server message.
    pub fn read_server_msg(&mut self) -> Result<ServerMsg> {
        read_server_msg(&mut self.reader)
    }

    /// Get a clone of the underlying TCP stream for shutdown signaling.
    pub fn try_clone_stream(&self) -> Result<TcpStream> {
        self.writer
            .get_ref()
            .try_clone()
            .context("cloning TCP stream for shutdown")
    }

    /// Split into reader and writer for separate thread ownership.
    pub fn into_split(self) -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
        (self.reader, self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_lt_common::protocol::{ClientMsg, write_client_msg, write_server_msg};
    use std::io::BufWriter as StdBufWriter;
    use std::net::TcpListener;

    #[test]
    fn connect_receives_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = StdBufWriter::new(stream);
            write_server_msg(&mut writer, &ServerMsg::Ready).unwrap();
            // Keep connection alive briefly
            std::thread::sleep(Duration::from_millis(100));
        });

        let conn = TcpConnection::connect(&format!("127.0.0.1:{port}")).unwrap();
        drop(conn);
        server_handle.join().unwrap();
    }

    #[test]
    fn connect_rejects_non_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = StdBufWriter::new(stream);
            write_server_msg(&mut writer, &ServerMsg::Error("bad".into())).unwrap();
        });

        let result = TcpConnection::connect(&format!("127.0.0.1:{port}"));
        assert!(result.is_err());
        server_handle.join().unwrap();
    }

    #[test]
    fn connect_with_retry_succeeds_on_second_attempt() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = std::thread::spawn(move || {
            // First connection: send Error (client will retry)
            let (stream, _) = listener.accept().unwrap();
            let mut writer = StdBufWriter::new(stream);
            write_server_msg(&mut writer, &ServerMsg::Error("not ready".into())).unwrap();
            drop(writer);

            // Second connection: send Ready (client succeeds)
            let (stream, _) = listener.accept().unwrap();
            let mut writer = StdBufWriter::new(stream);
            write_server_msg(&mut writer, &ServerMsg::Ready).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });

        let conn = TcpConnection::connect_with_retry(&format!("127.0.0.1:{port}")).unwrap();
        drop(conn);
        server_handle.join().unwrap();
    }

    #[test]
    fn split_send_audio_and_read_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let read_stream = stream.try_clone().unwrap();
            let mut writer = StdBufWriter::new(stream);
            let mut reader = BufReader::new(read_stream);

            // Send Ready
            write_server_msg(&mut writer, &ServerMsg::Ready).unwrap();

            // Read AudioSegment from client
            let msg = space_lt_common::protocol::read_client_msg(&mut reader).unwrap();
            match msg {
                ClientMsg::AudioSegment(samples) => assert_eq!(samples.len(), 1600),
                other => panic!("Expected AudioSegment, got {other:?}"),
            }

            // Send TtsAudioChunk + TtsEnd
            write_server_msg(&mut writer, &ServerMsg::TtsAudioChunk(vec![42i16; 4000])).unwrap();
            write_server_msg(&mut writer, &ServerMsg::TtsEnd).unwrap();
        });

        let conn = TcpConnection::connect(&format!("127.0.0.1:{port}")).unwrap();
        let (mut reader, mut writer) = conn.into_split();

        // Send AudioSegment via writer half
        write_client_msg(&mut writer, &ClientMsg::AudioSegment(vec![0i16; 1600])).unwrap();

        // Read TtsAudioChunk + TtsEnd via reader half
        let msg = read_server_msg(&mut reader).unwrap();
        match msg {
            ServerMsg::TtsAudioChunk(samples) => {
                assert_eq!(samples.len(), 4000);
                assert_eq!(samples[0], 42);
            }
            other => panic!("Expected TtsAudioChunk, got {other:?}"),
        }

        let msg = read_server_msg(&mut reader).unwrap();
        assert!(matches!(msg, ServerMsg::TtsEnd));

        server_handle.join().unwrap();
    }
}
