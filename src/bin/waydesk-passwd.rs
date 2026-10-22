//! `waydesk-passwd`: the account that runs it changes its own password.
//!
//! The current and the new password come on stdin, one per line (it reads two lines, not up to the end of the input) (never as arguments, never logged). The current one is checked
//! first through PAM (so a wrong one gets a plain answer), then the system's own `passwd` changes it, so its rules apply (password
//! quality modules and the like). `passwd` is the setuid program that may write the shadow file: PAM called from a process
//! that is not root cannot (`pam_unix: setuid failed`). It needs no privileges of its own.
//! Nothing is printed when it worked; otherwise the reason is on stderr and the exit code is 1.

use std::ffi::{CStr, CString};
use std::io::{BufRead as _, Read as _, Write as _};
use std::os::raw::{c_char, c_int, c_void};
use std::process::{Command, Stdio};

const MIN_LEN: usize = 8;
const MAX_LEN: usize = 128;
/// The PAM service that checks the current password (the one the RDP login uses).
const CHECK_SERVICE: &str = "waydesk-server";

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

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
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

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
        for i in 0..n as usize {
            let style = (**msgs.add(i)).msg_style;
            if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
                (*answers.add(i)).resp = libc::strdup(appdata as *const c_char);
            }
        }
        *resp = answers;
    }
    PAM_SUCCESS
}

/// True when PAM accepts `password` for `user`.
fn password_is_right(user: &str, password: &CStr) -> bool {
    let (Ok(service), Ok(user)) = (CString::new(CHECK_SERVICE), CString::new(user)) else {
        return false;
    };
    let conv = PamConv { conv: conversation, appdata_ptr: password.as_ptr() as *mut c_void };
    let mut handle: *mut c_void = std::ptr::null_mut();
    unsafe {
        if pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle) != PAM_SUCCESS {
            return false;
        }
        let ok = pam_authenticate(handle, 0) == PAM_SUCCESS;
        pam_end(handle, if ok { PAM_SUCCESS } else { 1 });
        ok
    }
}

/// The name of the account running this (from the account database, not from the environment).
fn my_name() -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        (!pw.is_null()).then(|| CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
    }
}

/// What `passwd` said, without its prompts and its own wrap-up lines: the reason it refused, if it gave one.
fn reason(output: &str) -> String {
    let mut text = output.replace("Current password:", "\n").replace("New password:", "\n").replace("Retype new password:", "\n");
    text = text.replace('\r', "\n");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("passwd:") && !l.starts_with("Changing password for") && !l.starts_with("Password change has been aborted"))
        .collect();
    if lines.is_empty() {
        "The password was not changed.".into()
    } else {
        lines.join(" ")
    }
}

fn change(old: &str, new: &str) -> Result<(), String> {
    let mut child = Command::new("/usr/bin/passwd")
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "The password program is not available.".to_string())?;
    let mut stdin = child.stdin.take().ok_or("The password program is not available.")?;
    let _ = stdin.write_all(format!("{old}\n{new}\n{new}\n").as_bytes());
    drop(stdin);
    let out = child.wait_with_output().map_err(|_| "The password program failed.".to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
    all.push('\n');
    all.push_str(&String::from_utf8_lossy(&out.stderr));
    Err(reason(&all))
}

fn run() -> Result<(), String> {
    // Exactly two lines, without waiting for the end of the input (the caller may keep the pipe open).
    let mut stdin = std::io::stdin().lock();
    let mut read_line = || -> Result<String, String> {
        let mut line = String::new();
        (&mut stdin).take(1024).read_line(&mut line).map_err(|_| "Could not read the passwords.".to_string())?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    };
    let (old, new) = (read_line()?, read_line()?);
    let (old, new) = (old.as_str(), new.as_str());
    if old.is_empty() {
        return Err("Enter your current password.".into());
    }
    if new.len() < MIN_LEN {
        return Err(format!("The new password needs at least {MIN_LEN} characters."));
    }
    if new.len() > MAX_LEN {
        return Err(format!("The new password can have at most {MAX_LEN} characters."));
    }
    if new == old {
        return Err("The new password is the same as the current one.".into());
    }
    let user = my_name().ok_or("Unknown account.")?;
    let old_c = CString::new(old).map_err(|_| "The current password is not valid.".to_string())?;
    if new.contains('\0') {
        return Err("The new password cannot contain a null character.".into());
    }
    if !password_is_right(&user, &old_c) {
        return Err("The current password is wrong.".into());
    }
    change(old, new)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::reason;

    #[test]
    fn keeps_only_the_reason_passwd_gave() {
        let out = "Current password: New password: Retype new password: You must choose a longer password.\nNew password: Password change has been aborted.\npasswd: Authentication token manipulation error\npasswd: password unchanged\nChanging password for pwtest.\n";
        assert_eq!(reason(out), "You must choose a longer password.");
        assert_eq!(reason("Current password: New password: Retype new password: Sorry, passwords do not match.\npasswd: password unchanged\n"), "Sorry, passwords do not match.");
    }

    #[test]
    fn says_something_when_passwd_said_nothing() {
        assert_eq!(reason("passwd: password unchanged\nChanging password for x.\n"), "The password was not changed.");
        assert_eq!(reason(""), "The password was not changed.");
    }
}
