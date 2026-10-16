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
/// Failures in a row for one account, and for one source address (higher: a office may share one address).
const MAX_FAILURES_USER: u32 = 5;
const MAX_FAILURES_SOURCE: u32 = 20;
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
        // Never root, whoever runs the server (a root daemon would otherwise match "the account running the server").
        if info.uid == 0 {
            return None;
        }
        if info.uid == self.running.uid {
            return Some(info);
        }
        // Another account: only possible as root, and only an ordinary one, never a system account.
        let ordinary = info.uid >= 1000 && info.uid != 65534;
        (self.root && ordinary && self.allow.as_ref().is_none_or(|a| a.iter().any(|n| n == name))).then_some(info)
    }

    /// Counts this attempt as a failure right away (it is cleared again on success), so an attempt that is cancelled
    /// half way, for example because the client reconnects at once, still counts. Returns false while locked out.
    fn begin_attempt(&self, name: &str, max_failures: u32) -> bool {
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
        if g.failures > max_failures {
            g.locked_until = Some(Instant::now() + LOCKOUT);
            warn!(key = name, "too many failed logins: refusing this one for {}s", LOCKOUT.as_secs());
            return false;
        }
        true
    }

    fn succeeded(&self, key: &str) {
        self.guards.lock().unwrap().remove(key);
    }

    /// Checks `password` for the account `name`, coming from `source` (an address, for the failure counters).
    /// Blocking: PAM can take a while and a failed attempt is delayed on purpose. Unknown accounts and wrong
    /// passwords get the same answer and the same delay.
    pub fn authenticate(&self, name: &str, password: &str, source: &str) -> Option<UserInfo> {
        let (user_key, source_key) = (format!("user:{name}"), format!("ip:{source}"));
        if !self.begin_attempt(&user_key, MAX_FAILURES_USER) || !self.begin_attempt(&source_key, MAX_FAILURES_SOURCE) {
            return None;
        }
        let candidate = if password.is_empty() { None } else { self.candidate(name) };
        let accepted = candidate.filter(|_| pam_check(&self.service, name, password));
        match &accepted {
            Some(info) => {
                self.succeeded(&user_key);
                self.succeeded(&source_key);
                info!(user = %name, uid = info.uid, %source, "login accepted (PAM)");
            }
            None => {
                warn!(user = %name, %source, "login rejected (PAM)");
                std::thread::sleep(FAIL_DELAY);
            }
        }
        accepted
    }
}

/// The validator of one connection when PAM runs in this process: `outcome` receives the account that logged in.
pub struct PamValidator {
    policy: Arc<PamPolicy>,
    outcome: Arc<Mutex<Option<UserInfo>>>,
    source: String,
}

impl PamValidator {
    pub fn new(policy: Arc<PamPolicy>, outcome: Arc<Mutex<Option<UserInfo>>>, source: String) -> Self {
        Self { policy, outcome, source }
    }
}

#[async_trait::async_trait]
impl CredentialValidator for PamValidator {
    async fn validate(&self, credentials: &Credentials) -> Result<CredentialDecision, CredentialValidationError> {
        let (policy, name, password, source) = (self.policy.clone(), credentials.username.clone(), credentials.password.clone(), self.source.clone());
        let accepted = tokio::task::spawn_blocking(move || policy.authenticate(&name, &password, &source)).await.unwrap_or(None);
        Ok(match accepted {
            Some(info) => {
                *self.outcome.lock().unwrap() = Some(info);
                CredentialDecision::Accept
            }
            None => CredentialDecision::Reject,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_system_accounts_never_log_in() {
        // Whoever runs the tests (root in the build container, an ordinary user elsewhere) the answer is the same.
        let policy = PamPolicy::new("sway-rdp-server", None).unwrap();
        assert!(policy.candidate("root").is_none());
        assert!(policy.candidate("nobody").is_none());
        assert!(policy.candidate("no such account").is_none());
        assert!(policy.candidate("").is_none());
        assert!(policy.candidate("a\0b").is_none());
    }

    #[test]
    fn failures_are_counted_per_key_and_lock_out() {
        let policy = PamPolicy::new("sway-rdp-server", None).unwrap();
        for _ in 0..5 {
            assert!(policy.begin_attempt("user:x", 5));
        }
        assert!(!policy.begin_attempt("user:x", 5), "the sixth failure in a row is refused");
        assert!(policy.begin_attempt("user:y", 5), "another account is not affected");
        policy.succeeded("user:y");
    }
}
