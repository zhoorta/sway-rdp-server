//! The daemon's side of the helper protocol: check a password and get a session through the root helper.
//! Blocking calls; callers use `spawn_blocking`.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context as _, Result};
use ironrdp_server::{CredentialDecision, CredentialValidationError, CredentialValidator, Credentials};
use tracing::warn;

use crate::fdpass;
use crate::proto::{self, HelperReq, HelperResp};

/// One connection to the helper, for one client of the daemon.
pub struct HelperConn {
    sock: Mutex<UnixStream>,
}

impl HelperConn {
    pub fn connect(path: &Path) -> io::Result<Self> {
        Ok(Self { sock: Mutex::new(UnixStream::connect(path)?) })
    }

    fn call(&self, sock: &mut UnixStream, req: &HelperReq) -> Result<HelperResp> {
        proto::write_frame(sock, &req.encode())?;
        let body = proto::read_frame(sock)?.context("the helper closed the connection")?;
        Ok(HelperResp::decode(&body)?)
    }

    pub fn authenticate(&self, user: &str, password: &str, source: &str) -> Result<bool> {
        let mut sock = self.sock.lock().unwrap();
        let req = HelperReq::Authenticate { user: user.to_owned(), password: password.to_owned(), source: source.to_owned() };
        match self.call(&mut sock, &req)? {
            HelperResp::AuthOk => Ok(true),
            HelperResp::AuthFail => Ok(false),
            HelperResp::Error { message } => bail!("helper: {message}"),
            other => bail!("unexpected answer to a login: {other:?}"),
        }
    }

    /// Starts the session of the account that logged in on this connection; returns our end of its socket.
    pub fn start_session(&self) -> Result<UnixStream> {
        let mut sock = self.sock.lock().unwrap();
        match self.call(&mut sock, &HelperReq::StartSession)? {
            HelperResp::Started => {}
            HelperResp::Error { message } => bail!("helper: {message}"),
            other => bail!("unexpected answer to a session request: {other:?}"),
        }
        Ok(UnixStream::from(fdpass::recv_fd(&sock).context("receiving the session socket")?))
    }
}

impl Drop for HelperConn {
    fn drop(&mut self) {
        if let Ok(mut sock) = self.sock.lock() {
            let _ = proto::write_frame(&mut *sock, &HelperReq::Bye.encode());
        }
    }
}

/// Login check of one connection, done by the helper.
pub struct HelperValidator {
    conn: Arc<HelperConn>,
    source: String,
    /// Set when the login was accepted.
    authed: Arc<AtomicBool>,
}

impl HelperValidator {
    pub fn new(conn: Arc<HelperConn>, source: String, authed: Arc<AtomicBool>) -> Self {
        Self { conn, source, authed }
    }
}

#[async_trait::async_trait]
impl CredentialValidator for HelperValidator {
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        let (conn, user, password, source) = (self.conn.clone(), credentials.username.clone(), credentials.password.clone(), self.source.clone());
        let result = tokio::task::spawn_blocking(move || conn.authenticate(&user, &password, &source)).await;
        Ok(match result {
            Ok(Ok(true)) => {
                self.authed.store(true, Ordering::Release);
                CredentialDecision::Accept
            }
            Ok(Ok(false)) => CredentialDecision::Reject,
            Ok(Err(e)) => {
                warn!("login check failed: {e:#}");
                CredentialDecision::Reject
            }
            Err(e) => {
                warn!("login check task failed: {e}");
                CredentialDecision::Reject
            }
        })
    }
}
