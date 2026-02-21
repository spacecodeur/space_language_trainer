use anyhow::{Context, Result};
use std::io::{BufWriter, Write};
use std::path::Path;

use space_lt_common::info;
use space_lt_common::protocol::{
    OrchestratorMsg, ServerMsg, read_orchestrator_msg, write_server_msg,
};

use crate::listener;
use crate::session;
use crate::transcribe::Transcriber;
use crate::tts::TtsEngine;

/// Run the server in daemon mode: TCP listener for client + Unix socket for orchestrator.
///
/// Models must already be loaded and passed as trait objects.
pub fn run_daemon(
    transcriber: Box<dyn Transcriber>,
    tts: Box<dyn TtsEngine>,
    port: u16,
    socket_path: &Path,
) -> Result<()> {
    // Start listeners
    let tcp_listener = listener::start_tcp(port)?;
    let unix_listener = listener::start_unix(socket_path)?;

    info!("[server] Waiting for client connection on port {port}...");
    let (tcp_stream, client_addr) = tcp_listener
        .accept()
        .context("accepting TCP client connection")?;
    info!("[server] Client connected from {client_addr}");

    // Send Ready to client
    let mut client_writer = BufWriter::new(
        tcp_stream
            .try_clone()
            .context("cloning TCP stream for Ready")?,
    );
    write_server_msg(&mut client_writer, &ServerMsg::Ready)?;
    client_writer.flush()?;

    info!(
        "[server] Waiting for orchestrator connection on {}...",
        socket_path.display()
    );
    let (unix_stream, _) = unix_listener
        .accept()
        .context("accepting Unix socket orchestrator connection")?;
    info!("[server] Orchestrator connected");

    // SessionStart handshake: read SessionStart, send Ready back on Unix socket.
    // Use raw stream (not BufReader) to avoid read-ahead stealing bytes from the fd
    // that run_session's BufReaders would then miss.
    let msg = read_orchestrator_msg(&mut &unix_stream)
        .context("reading SessionStart from orchestrator")?;
    match msg {
        OrchestratorMsg::SessionStart(config) => {
            info!("[server] SessionStart received: {config}");
        }
        other => {
            anyhow::bail!("Expected SessionStart from orchestrator, got {other:?}");
        }
    }

    write_server_msg(&mut &unix_stream, &ServerMsg::Ready)?;
    info!("[server] Sent Ready to orchestrator");

    info!("[server] Starting session routing...");
    session::run_session(transcriber, tts, tcp_stream, unix_stream)?;

    // Clean up Unix socket file
    if socket_path.exists() {
        std::fs::remove_file(socket_path).ok();
    }

    info!("[server] Server shutdown complete");
    Ok(())
}
