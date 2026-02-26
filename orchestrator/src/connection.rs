use anyhow::{Context, Result};
use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixStream;

use space_lt_common::info;
use space_lt_common::protocol::{
    OrchestratorMsg, ServerOrcMsg, read_server_orc_msg, write_orchestrator_msg,
};

/// Unix socket connection to the server, mirroring client's TcpConnection pattern.
pub struct OrchestratorConnection {
    reader: BufReader<UnixStream>,
    writer: BufWriter<UnixStream>,
}

impl OrchestratorConnection {
    /// Connect to the server Unix socket at the given path.
    pub fn connect(socket_path: &str) -> Result<Self> {
        info!("[orchestrator] Connecting to {socket_path}...");

        let stream =
            UnixStream::connect(socket_path).context("connecting to server Unix socket")?;

        let reader = BufReader::new(
            stream
                .try_clone()
                .context("cloning Unix stream for reader")?,
        );
        let writer = BufWriter::new(stream);

        Ok(Self { reader, writer })
    }

    /// Send SessionStart and wait for Ready ack from server.
    pub fn send_session_start(&mut self, config_json: &str) -> Result<()> {
        write_orchestrator_msg(
            &mut self.writer,
            &OrchestratorMsg::SessionStart(config_json.to_string()),
        )?;

        let msg = read_server_orc_msg(&mut self.reader)
            .context("waiting for server Ready after SessionStart")?;
        match msg {
            ServerOrcMsg::Ready => {
                info!("[orchestrator] Server ready");
                Ok(())
            }
            ServerOrcMsg::Error(e) => {
                anyhow::bail!("Server error during session start: {e}")
            }
            ServerOrcMsg::TranscribedText(_) => {
                anyhow::bail!("Unexpected TranscribedText during session start")
            }
            ServerOrcMsg::FeedbackChoice(_) => {
                anyhow::bail!("Unexpected FeedbackChoice during session start")
            }
            ServerOrcMsg::SummaryRequest => {
                anyhow::bail!("Unexpected SummaryRequest during session start")
            }
            ServerOrcMsg::CancelExchange => {
                anyhow::bail!("Unexpected CancelExchange during session start")
            }
        }
    }

    /// Send ResponseText to server for TTS synthesis.
    #[allow(dead_code)] // Used in tests
    pub fn send_response_text(&mut self, text: &str) -> Result<()> {
        write_orchestrator_msg(
            &mut self.writer,
            &OrchestratorMsg::ResponseText(text.to_string()),
        )
    }

    /// Send SessionEnd to signal clean shutdown.
    #[allow(dead_code)]
    pub fn send_session_end(&mut self) -> Result<()> {
        write_orchestrator_msg(&mut self.writer, &OrchestratorMsg::SessionEnd)
    }

    /// Read next message from server.
    #[allow(dead_code)] // Used in tests
    pub fn read_server_msg(&mut self) -> Result<ServerOrcMsg> {
        read_server_orc_msg(&mut self.reader)
    }

    /// Get a clone of the underlying Unix stream for shutdown signaling.
    pub fn try_clone_stream(&self) -> Result<UnixStream> {
        self.writer
            .get_ref()
            .try_clone()
            .context("cloning Unix stream for shutdown")
    }

    /// Split into reader and writer for separate thread ownership.
    pub fn into_split(self) -> (BufReader<UnixStream>, BufWriter<UnixStream>) {
        (self.reader, self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_lt_common::protocol::{
        ServerMsg, read_orchestrator_msg, write_orchestrator_msg, write_server_msg,
    };
    use std::os::unix::net::UnixListener;

    /// Helper to construct an OrchestratorConnection from a pre-connected UnixStream pair.
    fn from_stream(stream: UnixStream) -> OrchestratorConnection {
        let reader = BufReader::new(stream.try_clone().unwrap());
        let writer = BufWriter::new(stream);
        OrchestratorConnection { reader, writer }
    }

    #[test]
    fn connect_to_unix_socket() {
        let dir = std::env::temp_dir();
        let sock_path = dir.join(format!("space_lt_conn_test_{}.sock", std::process::id()));
        // Clean up any leftover socket
        let _ = std::fs::remove_file(&sock_path);

        let listener = UnixListener::bind(&sock_path).unwrap();

        let sock_str = sock_path.to_str().unwrap().to_string();
        let handle =
            std::thread::spawn(move || OrchestratorConnection::connect(&sock_str).unwrap());

        let (_server_stream, _) = listener.accept().unwrap();
        let conn = handle.join().unwrap();
        drop(conn);

        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn session_start_handshake() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::SessionStart(json) => {
                    assert!(json.contains("agent_file"));
                }
                other => panic!("Expected SessionStart, got {other:?}"),
            }

            write_server_msg(&mut writer, &ServerMsg::Ready).unwrap();
        });

        let mut conn = from_stream(client_stream);
        conn.send_session_start(r#"{"agent_file": "test.md"}"#)
            .unwrap();

        server_handle.join().unwrap();
    }

    #[test]
    fn send_receive_round_trip() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Server sends TranscribedText to orchestrator
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("hello".into()),
            )
            .unwrap();

            // Server reads ResponseText from orchestrator
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "response"),
                other => panic!("Expected ResponseText, got {other:?}"),
            }
        });

        let mut conn = from_stream(client_stream);

        let msg = conn.read_server_msg().unwrap();
        match msg {
            ServerOrcMsg::TranscribedText(t) => assert_eq!(t, "hello"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }

        conn.send_response_text("response").unwrap();

        server_handle.join().unwrap();
    }
}
