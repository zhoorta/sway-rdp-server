//! Login with the account's own password, checked through PAM, for `--auth pam`.
//!
//! This needs plain TLS login (no CredSSP/NLA), because NLA has to know the password itself, while PAM only answers
//! "right or wrong". The password therefore travels inside the TLS connection: verify the certificate fingerprint on
//! first use, and prefer a tunnel or VPN on networks you do not control.
//!
//! A process that is not root can only check its own password (PAM's `unix_chkpwd` helper does that), so then only the
//! user running the server can log in. Run as root (development setups only, see docs/MULTIUSER-DESIGN.md) and any
//! ordinary account (uid 1000 and up, or the ones named with `--allow-users`) can. Failures are slowed down and,
//! per account name, a few in a row are refused for a while.

use std::ffi::{c_char, c_int, c_void, CString};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ironrdp_server::{CredentialDecision, CredentialValidationError, CredentialValidator, Credentials};
use tracing::{info, warn};

/// An account, as the system knows it.
#[derive(Clone, Debug)]
pub struct UserInfo {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    /// All group ids of the account (including `gid`).
    pub groups: Vec<u32>,
}

/// Looks an account up by name.
pub fn lookup_user(name: &str) -> Option<UserInfo> {
    let cname = CString::new(name).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16384];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwnam_r(cname.as_ptr(), &mut pwd, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut res) };
    if rc != 0 || res.is_null() {
        return None;
    }
    let home = PathBuf::from(unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }.to_string_lossy().into_owned());
    let (uid, gid) = (pwd.pw_uid, pwd.pw_gid);
    let mut n: c_int = 64;
    let mut groups: Vec<libc::gid_t> = vec![0; n as usize];
    if unsafe { libc::getgrouplist(cname.as_ptr(), gid, groups.as_mut_ptr(), &mut n) } < 0 {
        groups = vec![0; n.max(1) as usize];
        if unsafe { libc::getgrouplist(cname.as_ptr(), gid, groups.as_mut_ptr(), &mut n) } < 0 {
            return None;
        }
    }
    groups.truncate(n.max(0) as usize);
    Some(UserInfo { name: name.to_owned(), uid, gid, home, groups })
}

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

/// Who may log in and how failures are counted. One per server.
pub struct PamPolicy {
    service: String,
    /// The account running the server.
    running: UserInfo,
    root: bool,
    /// Names allowed besides the running user (as root); None = every ordinary account.
    allow: Option<Vec<String>>,
    guards: Mutex<HashMap<String, Guard>>,
}

const FAIL_DELAY: Duration = Duration::from_secs(2);
const MAX_FAILURES: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);
/// Accounts tracked at once; unknown names are tracked too, so this bounds the memory an attacker can use.
const MAX_TRACKED: usize = 4096;

impl PamPolicy {
    pub fn new(service: &str, allow: Option<Vec<String>>) -> anyhow::Result<Arc<Self>> {
        let uid = unsafe { libc::getuid() };
        let name = unsafe {
            let pw = libc::getpwuid(uid);
            (!pw.is_null()).then(|| std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
        };
        let name = name.ok_or_else(|| anyhow::anyhow!("cannot find the name of the user running the server"))?;
        let running = lookup_user(&name).ok_or_else(|| anyhow::anyhow!("cannot look up {name}"))?;
        Ok(Arc::new(Self { service: service.to_owned(), running, root: uid == 0, allow, guards: Mutex::new(HashMap::new()) }))
    }

    pub fn running_user(&self) -> &str {
        &self.running.name
    }

    pub fn is_root(&self) -> bool {
        self.root
    }

    /// The account `name` if it may log in at all (before the password is looked at).
    fn candidate(&self, name: &str) -> Option<UserInfo> {
        let info = lookup_user(name)?;
        if info.uid == self.running.uid {
            return Some(info);
        }
        // Another account: only possible as root, never root itself or a system account.
        let ordinary = info.uid >= 1000 && info.uid != 65534;
        (self.root && ordinary && self.allow.as_ref().is_none_or(|a| a.iter().any(|n| n == name))).then_some(info)
    }

    /// Counts this attempt as a failure right away (it is cleared again on success), so an attempt that is cancelled
    /// half way, for example because the client reconnects at once, still counts. Returns false while locked out.
    fn begin_attempt(&self, name: &str) -> bool {
        let mut guards = self.guards.lock().unwrap();
        if guards.len() >= MAX_TRACKED {
            let now = Instant::now();
            guards.retain(|_, g| g.locked_until.is_some_and(|t| t > now));
        }
        let g = guards.entry(name.to_owned()).or_insert(Guard { failures: 0, locked_until: None });
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
            warn!(user = name, "too many failed logins: refusing this account for {}s", LOCKOUT.as_secs());
            return false;
        }
        true
    }

    fn succeeded(&self, name: &str) {
        self.guards.lock().unwrap().remove(name);
    }
}

/// The validator of one connection: the policy is shared, and `outcome` receives the account that logged in.
pub struct PamValidator {
    policy: Arc<PamPolicy>,
    outcome: Arc<Mutex<Option<UserInfo>>>,
}

impl PamValidator {
    pub fn new(policy: Arc<PamPolicy>, outcome: Arc<Mutex<Option<UserInfo>>>) -> Self {
        Self { policy, outcome }
    }
}

#[async_trait::async_trait]
impl CredentialValidator for PamValidator {
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        let name = credentials.username.clone();
        if !self.policy.begin_attempt(&name) {
            return Ok(CredentialDecision::Reject);
        }
        // Same answer and same delay for an unknown account and a wrong password; the domain part is ignored.
        let candidate = if credentials.password.is_empty() { None } else { self.policy.candidate(&name) };
        let accepted = match candidate {
            Some(info) => {
                let (service, user, password) = (self.policy.service.clone(), name.clone(), credentials.password.clone());
                let ok = tokio::task::spawn_blocking(move || pam_check(&service, &user, &password)).await.unwrap_or(false);
                ok.then_some(info)
            }
            None => None,
        };
        match accepted {
            Some(info) => {
                self.policy.succeeded(&name);
                info!(user = %name, uid = info.uid, "login accepted (PAM)");
                *self.outcome.lock().unwrap() = Some(info);
                Ok(CredentialDecision::Accept)
            }
            None => {
                warn!(user = %name, "login rejected (PAM)");
                tokio::time::sleep(FAIL_DELAY).await;
                Ok(CredentialDecision::Reject)
            }
        }
    }
}
