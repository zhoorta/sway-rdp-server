//! Login with the account's own password, checked through PAM, for `--auth pam`.
//!
//! This needs plain TLS login (no CredSSP/NLA), because NLA has to know the password itself, while PAM only answers
//! "right or wrong". The password therefore travels inside the TLS connection: verify the certificate fingerprint on
//! first use, and prefer a tunnel or VPN on networks you do not control.
//!
//! Only the user that runs the server can log in (a process that is not root can only check its own password; PAM's
//! `unix_chkpwd` helper does that). Failures are slowed down and, after a few in a row, refused for a while.

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ironrdp_server::{CredentialDecision, CredentialValidationError, CredentialValidator, Credentials};
use tracing::{info, warn};

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}
#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const PamConv, handle: *mut *mut c_void) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// Answers every password prompt with the password in `appdata_ptr`; other messages need no answer.
extern "C" fn conversation(n: c_int, msgs: *mut *const PamMessage, resp: *mut *mut PamResponse, appdata: *mut c_void) -> c_int {
    if n <= 0 || msgs.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    unsafe {
        let answers = libc::calloc(n as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if answers.is_null() {
            return PAM_CONV_ERR;
        }
        let password = appdata as *const c_char;
        for i in 0..n as usize {
            let style = (**msgs.add(i)).msg_style;
            if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
                (*answers.add(i)).resp = libc::strdup(password);
            }
        }
        *resp = answers;
    }
    PAM_SUCCESS
}

/// True when PAM accepts `password` for `user` under the PAM service `service`.
fn pam_check(service: &str, user: &str, password: &str) -> bool {
    let (Ok(service), Ok(user), Ok(password)) = (CString::new(service), CString::new(user), CString::new(password)) else {
        return false; // a NUL byte can never be part of a valid password
    };
    let conv = PamConv { conv: conversation, appdata_ptr: password.as_ptr() as *mut c_void };
    let mut handle: *mut c_void = std::ptr::null_mut();
    unsafe {
        if pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) != PAM_SUCCESS {
            return false;
        }
        let ok = pam_authenticate(handle, 0) == PAM_SUCCESS && pam_acct_mgmt(handle, 0) == PAM_SUCCESS;
        pam_end(handle, if ok { PAM_SUCCESS } else { 1 });
        ok
    }
}

struct Guard {
    failures: u32,
    locked_until: Option<Instant>,
}

pub struct PamValidator {
    service: String,
    /// The only account that may log in: the one running this server.
    user: String,
    guard: Mutex<Guard>,
}

const FAIL_DELAY: Duration = Duration::from_secs(2);
const MAX_FAILURES: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);

impl PamValidator {
    pub fn new(service: &str) -> anyhow::Result<Self> {
        let user = std::env::var("USER").ok().filter(|u| !u.is_empty()).or_else(|| {
            let pw = unsafe { libc::getpwuid(libc::getuid()) };
            (!pw.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().into_owned())
        });
        let user = user.ok_or_else(|| anyhow::anyhow!("cannot find the name of the user running the server"))?;
        Ok(Self { service: service.to_owned(), user, guard: Mutex::new(Guard { failures: 0, locked_until: None }) })
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// Counts this attempt as a failure right away (it is cleared again on success), so an attempt that is cancelled
    /// half way, for example because the client reconnects at once, still counts. Returns false while locked out.
    fn begin_attempt(&self) -> bool {
        let mut g = self.guard.lock().unwrap();
        if let Some(t) = g.locked_until {
            if t > Instant::now() {
                return false;
            }
            g.locked_until = None;
            g.failures = 0;
        }
        g.failures += 1;
        if g.failures > MAX_FAILURES {
            g.locked_until = Some(Instant::now() + LOCKOUT);
            warn!("too many failed logins: refusing all logins for {}s", LOCKOUT.as_secs());
            return false;
        }
        true
    }

    fn succeeded(&self) {
        self.guard.lock().unwrap().failures = 0;
    }
}

#[async_trait::async_trait]
impl CredentialValidator for PamValidator {
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        if !self.begin_attempt() {
            return Ok(CredentialDecision::Reject);
        }
        // Only the account running the server; the domain part is ignored.
        let ok = if credentials.username == self.user && !credentials.password.is_empty() {
            let (service, user, password) = (self.service.clone(), self.user.clone(), credentials.password.clone());
            tokio::task::spawn_blocking(move || pam_check(&service, &user, &password)).await.unwrap_or(false)
        } else {
            false
        };
        if ok {
            self.succeeded();
            info!(user = %self.user, "login accepted (PAM)");
            Ok(CredentialDecision::Accept)
        } else {
            warn!("login rejected (PAM)");
            tokio::time::sleep(FAIL_DELAY).await;
            Ok(CredentialDecision::Reject)
        }
    }
}
