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
    /// The login shell of the account (empty when it has none).
    pub shell: String,
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
    let shell = if pwd.pw_shell.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(pwd.pw_shell) }.to_string_lossy().into_owned() };
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
    Some(UserInfo { name: name.to_owned(), uid, gid, home, groups, shell })
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
    /// How many times this key has been locked (the next lockout is longer).
    lockouts: u32,
    /// The last attempt.
    last: Instant,
}

/// How long the `lockouts`-th lockout of a key lasts.
fn lockout_for(lockouts: u32) -> Duration {
    LOCKOUT.saturating_mul(1u32 << lockouts.saturating_sub(1).min(16)).min(LOCKOUT_MAX)
}

/// Whether an account with this login shell is meant to have an interactive login: the shell is listed in `/etc/shells` (the
/// system's own list of login shells) and is not one of the programs that stand for "no login". A desktop session gives its
/// account a terminal and a file manager, so an account that sshd or the console would refuse (a service or file-transfer
/// account) must not get one here.
fn shell_allows_login(shell: &str, listed: &str) -> bool {
    let shell = shell.trim();
    if shell.is_empty() {
        return false;
    }
    let program = shell.rsplit('/').next().unwrap_or(shell);
    if matches!(program, "nologin" | "false" | "true" | "sync" | "halt" | "shutdown") {
        return false;
    }
    listed.lines().map(str::trim).any(|l| !l.starts_with('#') && l == shell)
}

/// The id of the group called `name`.
fn group_gid(name: &str) -> Option<u32> {
    let cname = CString::new(name).ok()?;
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16384];
    let mut res: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrnam_r(cname.as_ptr(), &mut grp, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut res) };
    (rc == 0 && !res.is_null()).then_some(grp.gr_gid)
}

/// Who may log in and how failures are counted. One per server.
pub struct PamPolicy {
    service: String,
    /// The account running the server.
    running: UserInfo,
    root: bool,
    /// Names allowed besides the running user (as root); None = every ordinary account.
    allow: Option<Vec<String>>,
    /// A group every account has to be in (None = no such rule). A group that does not exist lets nobody in.
    require_group: Option<String>,
    guards: Mutex<HashMap<String, Guard>>,
    /// Failures per account over the last `WATCH_WINDOW`, from any address: (start of the window, failures, warned).
    watch: Mutex<HashMap<String, (Instant, u32, bool)>>,
}

const FAIL_DELAY: Duration = Duration::from_secs(2);
/// Failures in a row for one account from one source address, and for one source address in all (higher: an office may share
/// one address). A lockout lasts `LOCKOUT` and doubles each time the same key is locked again, up to `LOCKOUT_MAX`; a key that
/// has been quiet for `DECAY` starts again from the beginning.
const MAX_FAILURES_PAIR: u32 = 5;
const MAX_FAILURES_SOURCE: u32 = 20;
const LOCKOUT: Duration = Duration::from_secs(60);
const LOCKOUT_MAX: Duration = Duration::from_secs(3600);
const DECAY: Duration = Duration::from_secs(3600);
/// Failures of one account, from anywhere, that are worth a warning in the log (no rule per address sees a guess spread over
/// many addresses), counted over `WATCH_WINDOW`.
const WATCH_FAILURES: u32 = 20;
const WATCH_WINDOW: Duration = Duration::from_secs(600);
/// Keys tracked at once; unknown names are tracked too, so this bounds the memory an attacker can use.
const MAX_TRACKED: usize = 4096;

impl PamPolicy {
    pub fn new(service: &str, allow: Option<Vec<String>>, require_group: Option<String>) -> anyhow::Result<Arc<Self>> {
        if let Some(group) = &require_group {
            if group_gid(group).is_none() {
                warn!(group = ?group, "the group that logins require does not exist: nobody can log in until it is made");
            }
        }
        let uid = unsafe { libc::getuid() };
        let name = unsafe {
            let pw = libc::getpwuid(uid);
            (!pw.is_null()).then(|| std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
        };
        let name = name.ok_or_else(|| anyhow::anyhow!("cannot find the name of the user running the server"))?;
        let running = lookup_user(&name).ok_or_else(|| anyhow::anyhow!("cannot look up {name}"))?;
        Ok(Arc::new(Self { service: service.to_owned(), running, root: uid == 0, allow, require_group, guards: Mutex::new(HashMap::new()), watch: Mutex::new(HashMap::new()) }))
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
        if !shell_allows_login(&info.shell, &std::fs::read_to_string("/etc/shells").unwrap_or_default()) {
            return None;
        }
        if let Some(group) = &self.require_group {
            // `info.groups` has every group of the account (the primary one and the others).
            if !group_gid(group).is_some_and(|gid| info.groups.contains(&gid)) {
                return None;
            }
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
    fn begin_attempt(&self, key: &str, max_failures: u32) -> bool {
        self.begin_attempt_at(key, max_failures, Instant::now())
    }

    fn begin_attempt_at(&self, key: &str, max_failures: u32, now: Instant) -> bool {
        let mut guards = self.guards.lock().unwrap();
        if guards.len() >= MAX_TRACKED {
            guards.retain(|_, g| g.locked_until.is_some_and(|t| t > now));
        }
        let g = guards.entry(key.to_owned()).or_insert(Guard { failures: 0, locked_until: None, lockouts: 0, last: now });
        if let Some(t) = g.locked_until {
            if t > now {
                return false;
            }
            g.locked_until = None;
            g.failures = 0;
        }
        if now.duration_since(g.last) >= DECAY {
            g.failures = 0;
            g.lockouts = 0;
        }
        g.last = now;
        g.failures += 1;
        if g.failures > max_failures {
            g.lockouts += 1;
            let for_how_long = lockout_for(g.lockouts);
            g.locked_until = Some(now + for_how_long);
            warn!(key = ?key, lockouts = g.lockouts, "too many failed logins: refusing this one for {}s", for_how_long.as_secs());
            return false;
        }
        true
    }

    /// The key is forgotten (a login that worked ends the run of failures of that account from that address).
    fn succeeded(&self, key: &str) {
        self.guards.lock().unwrap().remove(key);
    }

    /// A success counts for one failure less, not for none: logging in once with an account of one's own must not wipe the
    /// failures that an address has piled up against other accounts.
    fn eased(&self, key: &str) {
        if let Some(g) = self.guards.lock().unwrap().get_mut(key) {
            g.failures = g.failures.saturating_sub(1);
        }
    }

    /// Notes a failed login of `name` (from any address) and warns once per window when there are many.
    fn note_failure(&self, name: &str, now: Instant) {
        let mut watch = self.watch.lock().unwrap();
        if watch.len() >= MAX_TRACKED {
            watch.retain(|_, (start, _, _)| now.duration_since(*start) < WATCH_WINDOW);
        }
        let entry = watch.entry(name.to_owned()).or_insert((now, 0, false));
        if now.duration_since(entry.0) >= WATCH_WINDOW {
            *entry = (now, 0, false);
        }
        entry.1 += 1;
        if entry.1 >= WATCH_FAILURES && !entry.2 {
            entry.2 = true;
            warn!(user = ?name, failures = entry.1, "many failed logins of one account in {} minutes, from any address: guessing?", WATCH_WINDOW.as_secs() / 60);
        }
    }

    /// Checks `password` for the account `name`, coming from `source` (an address, for the failure counters).
    /// Blocking: PAM can take a while and a failed attempt is delayed on purpose. Unknown accounts and wrong
    /// passwords get the same answer and the same delay.
    pub fn authenticate(&self, name: &str, password: &str, source: &str) -> Option<UserInfo> {
        // Per account and address together (with the length of the name, so no two pairs can be written the same way), and per
        // address: someone who guesses at an account from elsewhere cannot lock its owner out of their own address.
        let (pair_key, source_key) = (format!("pair:{}:{name}:{source}", name.len()), format!("ip:{source}"));
        if !self.begin_attempt(&pair_key, MAX_FAILURES_PAIR) || !self.begin_attempt(&source_key, MAX_FAILURES_SOURCE) {
            return None;
        }
        let candidate = if password.is_empty() { None } else { self.candidate(name) };
        let accepted = candidate.filter(|_| pam_check(&self.service, name, password));
        match &accepted {
            Some(info) => {
                self.succeeded(&pair_key);
                self.eased(&source_key);
                info!(user = ?name, uid = info.uid, source = ?source, "login accepted (PAM)");
            }
            None => {
                self.note_failure(name, Instant::now());
                warn!(user = ?name, source = ?source, "login rejected (PAM)");
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
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        assert!(policy.candidate("root").is_none());
        assert!(policy.candidate("nobody").is_none());
        assert!(policy.candidate("no such account").is_none());
        assert!(policy.candidate("").is_none());
        assert!(policy.candidate("a\0b").is_none());
    }

    #[test]
    fn a_required_group_that_does_not_exist_lets_nobody_in() {
        assert_eq!(group_gid("root"), Some(0));
        assert_eq!(group_gid("no-such-group-waydesk"), None);
        assert_eq!(group_gid("a\0b"), None);
        let policy = PamPolicy::new("waydesk-server", None, Some("no-such-group-waydesk".into())).unwrap();
        let me = unsafe { libc::getuid() };
        let name = unsafe { std::ffi::CStr::from_ptr((*libc::getpwuid(me)).pw_name) }.to_string_lossy().into_owned();
        assert!(policy.candidate(&name).is_none(), "even the account running the tests");
    }

    #[test]
    fn only_accounts_with_a_login_shell_may_log_in() {
        let listed = "# /etc/shells\n/bin/sh\n/bin/bash\n/usr/bin/zsh\n";
        assert!(shell_allows_login("/bin/bash", listed));
        assert!(shell_allows_login("/usr/bin/zsh", listed));
        assert!(!shell_allows_login("/usr/sbin/nologin", listed), "an account that may not log in");
        assert!(!shell_allows_login("/bin/false", listed));
        assert!(!shell_allows_login("", listed), "no shell");
        assert!(!shell_allows_login("/usr/bin/rssh", listed), "a restricted shell that /etc/shells does not list");
        assert!(!shell_allows_login("/bin/bash", "# /bin/bash\n"), "a commented line is not a listing");
        assert!(!shell_allows_login("/bin/bash", ""), "no list at all");
    }

    #[test]
    fn failures_are_counted_per_key_and_lock_out() {
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(policy.begin_attempt_at("pair:x", 5, t0));
        }
        assert!(!policy.begin_attempt_at("pair:x", 5, t0), "the sixth failure in a row is refused");
        assert!(policy.begin_attempt_at("pair:y", 5, t0), "another key is not affected");
        policy.succeeded("pair:y");
    }

    #[test]
    fn every_lockout_of_a_key_lasts_longer_and_a_quiet_hour_starts_again() {
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        let mut now = Instant::now();
        let mut locked_for = Vec::new();
        for _ in 0..4 {
            for _ in 0..5 {
                assert!(policy.begin_attempt_at("pair:z", 5, now));
            }
            assert!(!policy.begin_attempt_at("pair:z", 5, now), "locked");
            let wait = lockout_for(policy.guards.lock().unwrap()["pair:z"].lockouts);
            locked_for.push(wait.as_secs());
            now += wait + Duration::from_secs(1); // the lockout has ended
        }
        assert_eq!(locked_for, vec![60, 120, 240, 480], "doubling");
        assert_eq!(lockout_for(40).as_secs(), 3600, "and never more than an hour");
        // An hour without attempts: the key is as new.
        now += DECAY;
        assert!(policy.begin_attempt_at("pair:z", 5, now));
        assert_eq!(policy.guards.lock().unwrap()["pair:z"].lockouts, 0);
    }

    #[test]
    fn a_stranger_cannot_lock_an_account_out_of_its_owners_address() {
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        let t0 = Instant::now();
        let stranger = format!("pair:{}:alice:203.0.113.9", "alice".len());
        let owner = format!("pair:{}:alice:198.51.100.7", "alice".len());
        for _ in 0..5 {
            assert!(policy.begin_attempt_at(&stranger, 5, t0));
        }
        assert!(!policy.begin_attempt_at(&stranger, 5, t0), "the stranger is locked");
        assert!(policy.begin_attempt_at(&owner, 5, t0), "the owner, from the owner's address, is not");
    }

    #[test]
    fn a_success_eases_an_address_by_one_failure_and_does_not_wipe_it() {
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(policy.begin_attempt_at("ip:a", 20, t0));
        }
        policy.eased("ip:a");
        assert_eq!(policy.guards.lock().unwrap()["ip:a"].failures, 9);
        policy.eased("ip:unknown"); // nothing tracked: nothing happens
    }

    #[test]
    fn many_failures_of_one_account_are_noted_once_per_window() {
        let policy = PamPolicy::new("waydesk-server", None, None).unwrap();
        let t0 = Instant::now();
        for _ in 0..30 {
            policy.note_failure("bob", t0);
        }
        assert_eq!(policy.watch.lock().unwrap()["bob"].1, 30);
        assert!(policy.watch.lock().unwrap()["bob"].2, "warned");
        policy.note_failure("bob", t0 + WATCH_WINDOW);
        let w = policy.watch.lock().unwrap()["bob"];
        assert_eq!((w.1, w.2), (1, false), "a new window starts again");
    }
}
