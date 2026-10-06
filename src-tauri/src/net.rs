// SPDX-FileCopyrightText: 2026 BunnyCloud.IT
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-BunnyCloud-Commercial
//! TCP connections for the non-SSH protocols, directly or through an SSH jump host.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};
use crate::ssh;
use crate::store::Store;

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Stream for T {}

/// Connects to `address:port`, through `jump` when given. Messages worth showing the
/// user are appended to `notices`. The address is the local socket's (a placeholder
/// when tunnelled).
pub async fn connect(
    label: &str,
    address: &str,
    port: u16,
    jump: Option<&ssh::Target>,
    store: &Arc<Store>,
    notices: &mut Vec<String>,
) -> AppResult<(Box<dyn Stream>, SocketAddr)> {
    let timeout = Duration::from_secs(20);
    let timed_out = || AppError::Other(format!("Timed out connecting to {label}."));
    match jump {
        Some(j) => {
            let s = tokio::time::timeout(timeout, ssh::tunnel_stream(j, store.clone(), address, port, notices))
                .await
                .map_err(|_| timed_out())??;
            Ok((Box::new(s), SocketAddr::from(([127, 0, 0, 1], 0))))
        }
        None => {
            let tcp = tokio::time::timeout(timeout, TcpStream::connect((address, port)))
                .await
                .map_err(|_| timed_out())?
                .map_err(|e| AppError::Other(format!("Could not connect to {label}: {e}")))?;
            let _ = tcp.set_nodelay(true);
            let addr = tcp.local_addr()?;
            Ok((Box::new(tcp), addr))
        }
    }
}
