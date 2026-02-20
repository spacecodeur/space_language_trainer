use anyhow::{Context, Result};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::Path;

use space_lt_common::info;

/// Start a TCP listener on 0.0.0.0:port for client connections.
pub fn start_tcp(port: u16) -> Result<TcpListener> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).context("binding TCP listener")?;
    info!("[server] TCP listener started on {addr}");
    Ok(listener)
}

/// Start a Unix socket listener for orchestrator connections.
/// Removes a stale socket file if one exists from a previous run.
pub fn start_unix(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        std::fs::remove_file(path).context("removing stale Unix socket")?;
    }
    let listener = UnixListener::bind(path).context("binding Unix socket")?;
    info!(
        "[server] Unix socket listener started on {}",
        path.display()
    );
    Ok(listener)
}
