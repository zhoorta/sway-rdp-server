//! `waydesk-admin`: who may use the remote desktop, and what they have running.
//!
//!   waydesk-admin users list [--json]
//!   waydesk-admin users add NAME [--admin] [--sudo] [--no-rdp] [--json]
//!   waydesk-admin users remove NAME [--delete-home]
//!   waydesk-admin users passwd NAME [--stdin] [--json]
//!   waydesk-admin users enable NAME | disable NAME [--keep-session]
//!   waydesk-admin users admin NAME on|off       (the group waydesk-admins: Flatpak installs without a password)
//!   waydesk-admin users sudo NAME on|off        (the group sudo)
//!   waydesk-admin sessions list [--json]
//!   waydesk-admin sessions end NAME
//!   waydesk-admin status [--json] [--watch [SECONDS]]   (memory, swap, load, the GPUs and how busy they are, whether a new session would be accepted, and what each
//!                                                account uses and runs: renderer and video encoder; `--watch` repeats it, `--json --watch` prints one line of JSON each time)
//!   waydesk-admin users limit NAME [show | reset | --memory SIZE] [--swap SIZE] [--cpu-weight N] [--tasks N]
//!   waydesk-admin users set NAME [--desktop sway|cosmic|hyprland|default] [--gpu auto|off|renderDN|default]     the compositor and the GPU of one account's sessions (default: the server's setting)
//!   waydesk-admin config check NAME [--json]     whether the account's own compositor setup will work as a Waydesk session (docs/BRING-YOUR-OWN-CONFIG.md)
//!   waydesk-admin access [show | local | lan | vpn | custom ...]     who can reach the RDP port (scripts in /usr/lib/waydesk/admin)
//!   waydesk-admin cert [status | local-ca | reissue | export-ca | install | self-signed]     the certificate clients check
//!   waydesk-admin vpn [status | init | add NAME | show NAME | list | remove NAME]     a WireGuard VPN for the port
//!
//! Listing and `status` are open to everybody (the account database and the cgroup files are). Everything else needs root (`sudo`, or `pkexec` from the panel).
//! The work is done by the system's own tools (useradd, userdel, gpasswd, chpasswd, systemctl, loginctl), started by their
//! absolute path with fixed arguments and no shell, after every name and password has been checked. Passwords only travel
//! over a pipe to `chpasswd`, never as arguments, and are never logged. Accounts below uid 1000 and root are never touched.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::io::{Read as _, Write as _};
use std::os::raw::c_char;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context as _, Result};
use serde_json::json;

#[path = "../configcheck.rs"]
mod configcheck;
#[path = "../gpuinfo.rs"]
mod gpuinfo;
#[path = "../memory.rs"]
mod memory;
#[path = "../sessionstatus.rs"]
mod sessionstatus;

const RDP_GROUP: &str = "waydesk-users";
const ADMIN_GROUP: &str = "waydesk-admins";
const SUDO_GROUP: &str = "sudo";
const MIN_UID: u32 = 1000;
const NOBODY: u32 = 65534;

fn main() {
    // Rust ignores SIGPIPE, so `waydesk-admin status | head -3` ended in a panic ("failed printing to stdout: Broken pipe"). A command line tool
    // should simply stop when the reader goes away, as cat does.
    // SAFETY: restoring the default action of a signal, before any thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(e) = run() {
        eprintln!("waydesk-admin: {e:#}");
        std::process::exit(1);
    }
}

// ---- what is checked before anything is done ----

/// A name `useradd` accepts on Debian (its default NAME_REGEX): lower case letters, digits, `_` and `-`, not starting with a digit
/// or `-`, at most 32 characters.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    first_ok && name.len() <= 32 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A password that can be handed to `chpasswd` on a line of its own.
fn valid_password(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && !p.contains(['\n', '\r', '\0'])
}

/// 16 characters of a 32 letter alphabet (80 bits), in groups of four: easy to read out and to type.
fn make_password(random: &[u8; 16]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let letters: Vec<char> = random.iter().map(|b| ALPHABET[usize::from(*b) % 32] as char).collect();
    letters.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Whether the names in `members` would be left without anybody if `name` went.
fn would_leave_nobody(members: &[String], name: &str) -> bool {
    members.iter().any(|m| m == name) && members.iter().all(|m| m == name)
}

/// Whether `shell` is a shell an interactive login can use (listed in `/etc/shells`, not one of the "no login" programs).
fn login_shell(shell: &str, listed: &str) -> bool {
    let program = shell.rsplit('/').next().unwrap_or(shell);
    !shell.is_empty() && !matches!(program, "nologin" | "false" | "true" | "sync" | "halt" | "shutdown") && listed.lines().map(str::trim).any(|l| !l.starts_with('#') && l == shell)
}

// ---- the account database ----

#[derive(Clone, Debug)]
struct Account {
    name: String,
    uid: u32,
    gid: u32,
    shell: String,
}

/// One line of `getent passwd`.
fn parse_passwd(line: &str) -> Option<Account> {
    let f: Vec<&str> = line.split(':').collect();
    if f.len() < 7 {
        return None;
    }
    Some(Account { name: f[0].to_owned(), uid: f[2].parse().ok()?, gid: f[3].parse().ok()?, shell: f[6].to_owned() })
}

fn run_tool(path: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(path).args(args).stdin(Stdio::null()).output().with_context(|| format!("cannot run {path}"))?;
    ensure!(out.status.success(), "{path} {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The ordinary accounts (uid 1000 and up, not nobody), as the system's name service has them.
fn accounts() -> Result<Vec<Account>> {
    let mut list: Vec<Account> = run_tool("/usr/bin/getent", &["passwd"])?.lines().filter_map(parse_passwd).filter(|a| a.uid >= MIN_UID && a.uid != NOBODY).collect();
    list.sort_by_key(|a| a.uid);
    Ok(list)
}

fn account(name: &str) -> Result<Account> {
    ensure!(valid_name(name), "{name:?} is not a valid account name");
    accounts()?.into_iter().find(|a| a.name == name).with_context(|| format!("no ordinary account called {name}"))
}

fn group_gid(name: &str) -> Option<u32> {
    let cname = CString::new(name).ok()?;
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16384];
    let mut res: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrnam_r(cname.as_ptr(), &mut grp, buf.as_mut_ptr() as *mut c_char, buf.len(), &mut res) };
    (rc == 0 && !res.is_null()).then_some(grp.gr_gid)
}

/// Every group of the account, the primary one too.
fn groups_of(a: &Account) -> Vec<u32> {
    let Ok(cname) = CString::new(a.name.as_str()) else { return vec![] };
    let mut n: libc::c_int = 64;
    let mut groups: Vec<libc::gid_t> = vec![0; n as usize];
    if unsafe { libc::getgrouplist(cname.as_ptr(), a.gid, groups.as_mut_ptr(), &mut n) } < 0 {
        groups = vec![0; n.max(1) as usize];
        if unsafe { libc::getgrouplist(cname.as_ptr(), a.gid, groups.as_mut_ptr(), &mut n) } < 0 {
            return vec![];
        }
    }
    groups.truncate(n.max(0) as usize);
    groups
}

fn in_group(a: &Account, group: &str) -> bool {
    group_gid(group).is_some_and(|gid| groups_of(a).contains(&gid))
}

fn members_of(group: &str) -> Result<Vec<String>> {
    Ok(accounts()?.into_iter().filter(|a| in_group(a, group)).map(|a| a.name).collect())
}

/// Who may be looked at: everybody for root and the administrators, only the account of the caller for the others.
fn only_visible(all: Vec<Account>, caller: u32, caller_is_admin: bool) -> Vec<Account> {
    if caller == 0 || caller_is_admin {
        all
    } else {
        all.into_iter().filter(|a| a.uid == caller).collect()
    }
}

/// The accounts the caller may look at. Administrators are the group `sudo`, as for the polkit rules and the Users & Access panel; the
/// other accounts see their own row only (the data is in /proc and the cgroups anyway: this keeps the tool from collecting it for them;
/// see the README for `hidepid`).
fn visible_accounts() -> Result<Vec<Account>> {
    let all = accounts()?;
    let uid = unsafe { libc::getuid() };
    let is_admin = all.iter().find(|a| a.uid == uid).is_some_and(|me| in_group(me, SUDO_GROUP));
    Ok(only_visible(all, uid, is_admin))
}

/// The account that asked, when this runs through sudo or pkexec (so it cannot be used to remove oneself).
fn caller_uid() -> Option<u32> {
    ["PKEXEC_UID", "SUDO_UID"].iter().find_map(|k| std::env::var(k).ok()?.parse().ok())
}

fn require_root() -> Result<()> {
    ensure!(unsafe { libc::geteuid() } == 0, "this needs root: run it with sudo");
    Ok(())
}

// ---- sessions ----

fn session_unit(uid: u32) -> String {
    format!("waydesk-session@{uid}.service")
}

/// ("running" or "none", since when).
fn session_state(uid: u32) -> (String, String) {
    let out = run_tool("/usr/bin/systemctl", &["show", &session_unit(uid), "-p", "ActiveState", "-p", "ActiveEnterTimestamp"]).unwrap_or_default();
    let get = |key: &str| out.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("").trim().to_owned();
    if get("ActiveState=") == "active" {
        ("running".into(), get("ActiveEnterTimestamp="))
    } else {
        ("none".into(), String::new())
    }
}

fn end_session(a: &Account) {
    // The unit is the headless sway and what runs in it; terminating the account's login as well leaves nothing of it behind.
    let _ = run_tool("/usr/bin/systemctl", &["stop", &session_unit(a.uid)]);
    let _ = run_tool("/usr/bin/loginctl", &["terminate-user", &a.name]);
    // terminate-user returns at once and logind finishes in the background. A login in that window starts the account's user
    // manager while logind is still stopping the old runtime directory; the two collide and the new start hangs until it times
    // out (about 90 s). So return only when logind has let go of the account.
    for _ in 0..60 {
        if run_tool("/usr/bin/loginctl", &["show-user", &a.name]).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

// ---- commands ----

fn run() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let json_out = take_flag(&mut args, "--json");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["users", "list"] => users_list(json_out),
        ["users", "add", name, rest @ ..] => users_add(name, rest, json_out),
        ["users", "remove", name, rest @ ..] => users_remove(name, rest),
        ["users", "passwd", name, rest @ ..] => users_passwd(name, rest, json_out),
        ["users", "enable", name] => set_group(name, RDP_GROUP, true),
        ["users", "disable", name, rest @ ..] => users_disable(name, rest),
        ["users", "admin", name, onoff] => set_group(name, ADMIN_GROUP, parse_onoff(onoff)?),
        ["users", "sudo", name, onoff] => users_sudo(name, parse_onoff(onoff)?),
        ["users", "set", name, rest @ ..] => users_set(name, rest),
        ["config", "check", name] => config_check(name, json_out),
        ["sessions", "list"] => sessions_list(json_out),
        ["status", rest @ ..] => status(json_out, parse_watch(rest)?),
        ["--version"] | ["-V"] => {
            println!("waydesk-admin {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        // Who can reach the port, what certificate it shows, a VPN for it: shell scripts that check every value they are given
        // (they ask for root themselves where they change something).
        [which @ ("access" | "cert" | "vpn"), rest @ ..] => run_admin_script(which, rest),
        ["users", "limit", name, rest @ ..] => users_limit(name, rest),
        ["sessions", "end", name] => {
            require_root()?;
            let a = account(name)?;
            end_session(&a);
            println!("ended the session of {}", a.name);
            Ok(())
        }
        _ => {
            eprintln!("{}", USAGE.trim_end());
            std::process::exit(2);
        }
    }
}

const USAGE: &str = "
usage: waydesk-admin users list [--json]
       waydesk-admin users add NAME [--admin] [--sudo] [--no-rdp] [--json]
       waydesk-admin users remove NAME [--delete-home]
       waydesk-admin users passwd NAME [--stdin] [--json]
       waydesk-admin users enable NAME | disable NAME [--keep-session]
       waydesk-admin users admin NAME on|off   (Flatpak installs without a password)
       waydesk-admin users sudo NAME on|off
       waydesk-admin sessions list [--json] | end NAME
       waydesk-admin status [--json] [--watch [SECONDS]]
       waydesk-admin users limit NAME [show | reset | --memory SIZE] [--swap SIZE] [--cpu-weight N] [--tasks N]
       waydesk-admin users set NAME [--desktop sway|cosmic|hyprland|default] [--gpu auto|off|renderDN|default]
       waydesk-admin config check NAME [--json]
       waydesk-admin access [show | local | lan [--address IP] [--allow LIST] | vpn | custom --bind ADDR:PORT [--allow LIST]]
       waydesk-admin cert [status | local-ca [--san NAME] | reissue | export-ca | install --cert FILE --key FILE | self-signed]
       waydesk-admin vpn [status | init [--endpoint HOST] | add NAME | show NAME | list | remove NAME]
";

/// Runs one of the scripts in /usr/lib/waydesk/admin (by absolute path, the arguments as they are: no shell on the way) and ends with its status.
fn run_admin_script(name: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(format!("/usr/lib/waydesk/admin/{name}"))
        .args(args)
        .status()
        .with_context(|| format!("cannot run /usr/lib/waydesk/admin/{name}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}

fn parse_onoff(s: &str) -> Result<bool> {
    match s {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("say on or off, not {s:?}"),
    }
}

fn users_list(json_out: bool) -> Result<()> {
    let shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    if group_gid(RDP_GROUP).is_none() {
        eprintln!("note: the group {RDP_GROUP} does not exist, so nobody may log in (is the waydesk package installed?)");
    }
    let mut rows = Vec::new();
    for a in visible_accounts()? {
        let (session, since) = session_state(a.uid);
        let (desktop, source) = desktop_of(a.uid);
        let gpu = gpu_of(a.uid);
        let running = sessionstatus::read(Path::new(sessionstatus::DIR), a.uid).and_then(|s| s.compositor);
        rows.push((a.name.clone(), a.uid, in_group(&a, RDP_GROUP), in_group(&a, ADMIN_GROUP), in_group(&a, SUDO_GROUP), login_shell(&a.shell, &shells), session, since, desktop, source, gpu, running));
    }
    if json_out {
        let list: Vec<_> = rows
            .iter()
            .map(|(name, uid, rdp, admin, sudo, shell, session, since, desktop, source, gpu, running)| {
                json!({ "name": name, "uid": uid, "rdp": rdp, "flatpak_admin": admin, "sudo": sudo, "login_shell": shell, "session": session, "session_since": since,
                        "desktop": desktop, "desktop_from": source, "desktop_running": running, "gpu": gpu.0, "gpu_from": gpu.1 })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let yes = |v: bool| if v { "yes" } else { "-" };
    println!("{:<20} {:>5}  {:<4} {:<8} {:<5} {:<7} {:<10} {:<12} SESSION", "ACCOUNT", "UID", "RDP", "FLATPAK", "SUDO", "SHELL", "DESKTOP", "GPU");
    for (name, uid, rdp, admin, sudo, shell, session, _, desktop, source, gpu, running) in &rows {
        // A star: the account has a setting of its own. A running session keeps the desktop it started with, whatever is set now.
        let mut cell = format!("{desktop}{}", if *source == "account" { "*" } else { "" });
        let session_cell = match running {
            Some(r) if r != desktop => format!("{session} on {r} (restart to get {desktop})"),
            Some(r) => format!("{session} on {r}"),
            None => session.clone(),
        };
        cell.truncate(10);
        // `renderD129` for a node; a star as for the desktop.
        let mut gcell = format!("{}{}", gpu.0.rsplit('/').next().unwrap_or(&gpu.0), if gpu.1 == "account" { "*" } else { "" });
        gcell.truncate(12);
        println!("{:<20} {:>5}  {:<4} {:<8} {:<5} {:<7} {:<10} {:<12} {}", name, uid, yes(*rdp), yes(*admin), yes(*sudo), yes(*shell), cell, gcell, session_cell);
    }
    Ok(())
}

fn sessions_list(json_out: bool) -> Result<()> {
    let mut rows = Vec::new();
    for a in visible_accounts()? {
        let (state, since) = session_state(a.uid);
        if state == "running" {
            rows.push((a.name, a.uid, since));
        }
    }
    if json_out {
        let list: Vec<_> = rows.iter().map(|(name, uid, since)| json!({ "name": name, "uid": uid, "since": since })).collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else if rows.is_empty() {
        println!("no sessions running");
    } else {
        println!("{:<20} {:>5}  SINCE", "ACCOUNT", "UID");
        for (name, uid, since) in &rows {
            println!("{:<20} {:>5}  {}", name, uid, since);
        }
    }
    Ok(())
}

fn random_password() -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).context("cannot read /dev/urandom")?;
    Ok(make_password(&bytes))
}

fn set_password(name: &str, password: &str) -> Result<()> {
    ensure!(valid_password(password), "the password has to be 1 to 128 characters on one line");
    let mut child = Command::new("/usr/sbin/chpasswd").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().context("cannot run chpasswd")?;
    child.stdin.take().context("no pipe to chpasswd")?.write_all(format!("{name}:{password}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    ensure!(out.status.success(), "chpasswd failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

fn read_stdin_password() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().take(1024).read_to_string(&mut line)?;
    Ok(line.trim_end_matches(['\n', '\r']).to_owned())
}

fn change_group(name: &str, group: &str, add: bool) -> Result<()> {
    ensure!(group_gid(group).is_some(), "the group {group} does not exist (is the waydesk package installed?)");
    run_tool("/usr/bin/gpasswd", &[if add { "-a" } else { "-d" }, name, group]).map(|_| ())
}

fn set_group(name: &str, group: &str, on: bool) -> Result<()> {
    require_root()?;
    let a = account(name)?;
    if in_group(&a, group) == on {
        println!("{} is {} {group} already", a.name, if on { "in" } else { "not in" });
        return Ok(());
    }
    change_group(&a.name, group, on)?;
    println!("{} {} {group}", a.name, if on { "is now in" } else { "is no longer in" });
    Ok(())
}

fn users_disable(name: &str, rest: &[&str]) -> Result<()> {
    let keep = rest.contains(&"--keep-session");
    ensure!(rest.iter().all(|r| *r == "--keep-session"), "unknown option for disable");
    require_root()?;
    let a = account(name)?;
    ensure!(caller_uid() != Some(a.uid), "you cannot switch off your own access");
    change_group_if(&a, RDP_GROUP, false)?;
    if !keep {
        end_session(&a);
        println!("{} may no longer log in, and the session was ended", a.name);
    } else {
        println!("{} may no longer log in; the running session was left alone", a.name);
    }
    Ok(())
}

fn change_group_if(a: &Account, group: &str, on: bool) -> Result<()> {
    if in_group(a, group) != on {
        change_group(&a.name, group, on)?;
    }
    Ok(())
}

fn users_sudo(name: &str, on: bool) -> Result<()> {
    require_root()?;
    let a = account(name)?;
    if !on {
        ensure!(caller_uid() != Some(a.uid), "you cannot take administrator rights from yourself");
        ensure!(!would_leave_nobody(&members_of(SUDO_GROUP)?, &a.name), "{} is the only administrator (group {SUDO_GROUP}): make another one first", a.name);
    }
    set_group(name, SUDO_GROUP, on)
}

fn users_add(name: &str, rest: &[&str], json_out: bool) -> Result<()> {
    for r in rest {
        ensure!(["--admin", "--sudo", "--no-rdp"].contains(r), "unknown option {r} for add");
    }
    require_root()?;
    ensure!(valid_name(name), "{name:?} is not a valid account name (lower case letters, digits, _ and -, at most 32)");
    // Any account of that name, system accounts too (accounts() only lists the ordinary ones).
    ensure!(run_tool("/usr/bin/getent", &["passwd", name]).is_err(), "{name} exists already");
    run_tool("/usr/sbin/useradd", &["-m", "-s", "/bin/bash", name])?;
    let a = account(name).context("the new account does not look like an ordinary one: remove it by hand")?;
    let password = random_password()?;
    set_password(&a.name, &password)?;
    let mut granted = Vec::new();
    if !rest.contains(&"--no-rdp") {
        change_group(&a.name, RDP_GROUP, true)?;
        granted.push(RDP_GROUP);
    }
    if rest.contains(&"--admin") {
        change_group(&a.name, ADMIN_GROUP, true)?;
        granted.push(ADMIN_GROUP);
    }
    if rest.contains(&"--sudo") {
        change_group(&a.name, SUDO_GROUP, true)?;
        granted.push(SUDO_GROUP);
    }
    if json_out {
        println!("{}", json!({ "name": a.name, "uid": a.uid, "password": password, "groups": granted }));
    } else {
        println!("made {} (uid {}), groups: {}", a.name, a.uid, if granted.is_empty() { "none".into() } else { granted.join(", ") });
        println!("temporary password (shown once): {password}");
        println!("The account should change it at once: Change password, in the desktop's power menu.");
    }
    Ok(())
}

fn users_remove(name: &str, rest: &[&str]) -> Result<()> {
    for r in rest {
        ensure!(*r == "--delete-home", "unknown option {r} for remove");
    }
    require_root()?;
    let a = account(name)?;
    ensure!(caller_uid() != Some(a.uid), "you cannot remove your own account");
    ensure!(!would_leave_nobody(&members_of(SUDO_GROUP)?, &a.name), "{} is the only administrator (group {SUDO_GROUP}): make another one first", a.name);
    end_session(&a);
    // loginctl returns before everything is gone: give userdel a moment.
    for _ in 0..20 {
        if run_tool("/usr/bin/pgrep", &["-u", &a.uid.to_string()]).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let mut userdel = vec![];
    if rest.contains(&"--delete-home") {
        userdel.push("-r");
    }
    userdel.push(&a.name);
    run_tool("/usr/sbin/userdel", &userdel)?;
    // The account's settings belong to its user id, which a new account can get again: they go with it.
    forget_account_files(a.uid, &[Path::new(configcheck::ACCOUNT_SETTINGS_DIR)]);
    println!("removed {}{}", a.name, if rest.contains(&"--delete-home") { " and its home folder" } else { " (its home folder was kept)" });
    Ok(())
}

/// Removes `UID.env` from each directory (it is fine if there is none).
fn forget_account_files(uid: u32, dirs: &[&Path]) {
    for d in dirs {
        let _ = std::fs::remove_file(d.join(format!("{uid}.env")));
    }
}

fn users_passwd(name: &str, rest: &[&str], json_out: bool) -> Result<()> {
    for r in rest {
        ensure!(*r == "--stdin", "unknown option {r} for passwd");
    }
    require_root()?;
    let a = account(name)?;
    let (password, shown) = if rest.contains(&"--stdin") { (read_stdin_password()?, false) } else { (random_password()?, true) };
    set_password(&a.name, &password)?;
    if shown && json_out {
        println!("{}", json!({ "name": a.name, "password": password }));
    } else if shown {
        println!("new password for {} (shown once): {password}  (the account should change it: Change password, in the power menu)", a.name);
    } else {
        println!("changed the password of {}", a.name);
    }
    Ok(())
}

// ---- what the machine and the sessions use ----

const USER_SLICES: &str = "/sys/fs/cgroup/user.slice";

fn user_cgroup(uid: u32) -> PathBuf {
    Path::new(USER_SLICES).join(format!("user-{uid}.slice"))
}

/// A number in a cgroup file ("max" and anything else that is not one: None).
fn read_number(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Microseconds of CPU the cgroup has used so far.
fn cpu_usec(dir: &Path) -> Option<u64> {
    std::fs::read_to_string(dir.join("cpu.stat")).ok()?.lines().find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok())
}

struct Usage {
    name: String,
    uid: u32,
    session: bool,
    mem_mb: u64,
    swap_mb: u64,
    tasks: u64,
    /// Share of one core over the sample (a busy browser on two cores is 200).
    cpu_percent: f64,
    cap_mb: Option<u64>,
    /// The programs of the account that hold the most memory (resident, shared pages counted for each process).
    top: Vec<(String, u64)>,
    /// What the session says it runs (renderer, encoder, quantiser, frame rate): only for a running session whose file the caller may read.
    video: Option<sessionstatus::Status>,
}

/// The resident memory (kB) of the programs of each of the accounts, by program name.
fn resident_by_program(uids: &HashSet<u32>) -> HashMap<u32, HashMap<String, u64>> {
    let page_kb = (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64) / 1024;
    let mut out: HashMap<u32, HashMap<String, u64>> = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().filter(|n| n.bytes().all(|b| b.is_ascii_digit())) else { continue };
        let Ok(meta) = std::fs::metadata(entry.path()) else { continue };
        if !uids.contains(&meta.uid()) {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_owned();
        let rss_pages: u64 = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok().and_then(|t| t.split_whitespace().nth(1)?.parse().ok()).unwrap_or(0);
        if !comm.is_empty() {
            *out.entry(meta.uid()).or_default().entry(comm).or_default() += rss_pages * page_kb;
        }
    }
    out
}

/// What every account that has a cgroup or a session uses; the CPU is measured over `sample`.
fn gather_usage(sample: std::time::Duration) -> Result<Vec<Usage>> {
    let accounts = visible_accounts()?;
    let first: Vec<Option<u64>> = accounts.iter().map(|a| cpu_usec(&user_cgroup(a.uid))).collect();
    let started = std::time::Instant::now();
    std::thread::sleep(sample);
    let elapsed_us = started.elapsed().as_micros().max(1) as f64;
    let uids: HashSet<u32> = accounts.iter().map(|a| a.uid).collect();
    let by_program = resident_by_program(&uids);
    let mut out = Vec::new();
    for (a, before) in accounts.into_iter().zip(first) {
        let dir = user_cgroup(a.uid);
        let session = session_state(a.uid).0 == "running";
        if !dir.exists() && !session {
            continue;
        }
        let cpu_percent = match (before, cpu_usec(&dir)) {
            (Some(b), Some(n)) if n >= b => (n - b) as f64 / elapsed_us * 100.0,
            _ => 0.0,
        };
        let mut top: Vec<(String, u64)> = by_program.get(&a.uid).map(|m| m.iter().map(|(k, v)| (k.clone(), v / 1024)).collect()).unwrap_or_default();
        top.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        top.truncate(3);
        out.push(Usage {
            name: a.name,
            uid: a.uid,
            session,
            mem_mb: read_number(&dir.join("memory.current")).unwrap_or(0) / 1_048_576,
            swap_mb: read_number(&dir.join("memory.swap.current")).unwrap_or(0) / 1_048_576,
            tasks: read_number(&dir.join("pids.current")).unwrap_or(0),
            cpu_percent,
            cap_mb: read_number(&dir.join("memory.max")).map(|b| b / 1_048_576),
            top,
            video: if session { sessionstatus::read(Path::new(sessionstatus::DIR), a.uid) } else { None },
        });
    }
    Ok(out)
}

/// "1.4 GB" or "620 MB".
fn human_mb(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

/// What the helper was started with: (--min-available-mb, --max-active-users).
fn helper_policy() -> (Option<u64>, usize) {
    let pid: u32 = run_tool("/usr/bin/systemctl", &["show", "waydesk-helper.service", "-p", "MainPID", "--value"]).ok().and_then(|o| o.trim().parse().ok()).unwrap_or(0);
    let cmdline = if pid > 0 { std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default() } else { Vec::new() };
    policy_from_args(&cmdline.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>())
}

fn policy_from_args(args: &[String]) -> (Option<u64>, usize) {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1));
    (value("--min-available-mb").and_then(|v| v.parse().ok()), value("--max-active-users").and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// The swap devices: (name, size kB, used kB).
fn swap_devices() -> Vec<(String, u64, u64)> {
    let text = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    text.lines().skip(1).filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        Some((f.first()?.to_string(), f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?))
    }).collect()
}

/// `--watch` and its optional number of seconds (default 2, at least 1: a refresh takes a second of sampling anyway).
fn parse_watch(rest: &[&str]) -> Result<Option<std::time::Duration>> {
    let secs = |t: &str| -> Result<std::time::Duration> {
        let v: f64 = t.parse().map_err(|_| anyhow::anyhow!("--watch wants a number of seconds, not {t:?}"))?;
        ensure!(v.is_finite() && (1.0..=3600.0).contains(&v), "--watch wants between 1 and 3600 seconds");
        Ok(std::time::Duration::from_secs_f64(v))
    };
    match rest {
        [] => Ok(None),
        ["--watch"] => Ok(Some(std::time::Duration::from_secs(2))),
        ["--watch", n] => Ok(Some(secs(n)?)),
        [one] if one.starts_with("--watch=") => Ok(Some(secs(&one["--watch=".len()..])?)),
        _ => {
            eprintln!("{}", USAGE.trim_end());
            std::process::exit(2);
        }
    }
}

/// `WAYDESK_GPU` as `/etc/default/waydesk` sets it (what the sessions that start next will use): `auto`, `off` or a render node.
fn gpu_setting() -> String {
    let text = std::fs::read_to_string("/etc/default/waydesk").unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("WAYDESK_GPU="))
        .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
        .last()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "auto".to_owned())
}

/// `WAYDESK_COMPOSITOR` as `/etc/default/waydesk` sets it (what the sessions that start next will run on): `sway` unless it says `hyprland`.
fn compositor_setting() -> String {
    let text = std::fs::read_to_string("/etc/default/waydesk").unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("WAYDESK_COMPOSITOR="))
        .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase())
        .last()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "sway".to_owned())
}

/// The pretty name of the distribution out of an os-release file ("Arch Linux", "Debian GNU/Linux 13 (trixie)").
fn os_pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned())
        .filter(|v| !v.is_empty())
}

/// "Arch Linux, kernel 7.2.11-arch1-1".
fn system_line() -> String {
    let os = os_pretty_name(&std::fs::read_to_string("/etc/os-release").unwrap_or_default()).unwrap_or_else(|| "unknown distribution".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    match kernel.trim() {
        "" => os,
        k => format!("{os}, kernel {k}"),
    }
}

/// What a session runs, for the table: "gpu" or "cpu" for the renderer, "nvenc q25 17fps" for the encoder.
fn video_cells(v: Option<&sessionstatus::Status>) -> (String, String) {
    let Some(v) = v else { return ("-".into(), "-".into()) };
    let render = if v.render.is_some() { "gpu" } else { "cpu" }.to_owned();
    let Some(enc) = &v.encoder else { return (render, "-".into()) };
    let mut cell = enc.clone();
    if sessionstatus::is_fresh(v) {
        if let Some(q) = v.qp {
            cell.push_str(&format!(" q{q}"));
        }
        if let Some(f) = v.fps {
            cell.push_str(&format!(" {f:.0}fps"));
        }
    }
    (render, cell)
}

fn status(json_out: bool, watch: Option<std::time::Duration>) -> Result<()> {
    let Some(every) = watch else { return status_once(json_out, false) };
    // On a terminal each round replaces the last; piped, the rounds follow each other (one line of JSON each with --json).
    let tty = !json_out && unsafe { libc::isatty(1) } == 1;
    loop {
        let started = std::time::Instant::now();
        if tty {
            print!("\x1b[H\x1b[2J");
        }
        status_once(json_out, true)?;
        if !json_out {
            println!("\nevery {:.0} s, Ctrl-C to stop", every.as_secs_f64());
        }
        std::io::stdout().flush()?;
        if let Some(rest) = every.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn status_once(json_out: bool, compact: bool) -> Result<()> {
    let info = memory::read_meminfo().context("cannot read /proc/meminfo")?;
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default().split_whitespace().take(3).map(str::to_owned).collect::<Vec<_>>();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let (min_mb, max_users) = helper_policy();
    // The GPU counters and the CPU of the accounts are measured over the same second.
    let mut gpus = gpuinfo::list();
    let sampler = gpuinfo::begin(&gpus);
    let usage = gather_usage(std::time::Duration::from_millis(1000))?;
    gpuinfo::finish(sampler, &mut gpus);
    let setting = gpu_setting();
    // The admission rule counts every account with a session, whoever the caller may look at.
    let running = accounts()?.iter().filter(|a| session_state(a.uid).0 == "running").count();
    let admission = memory::admission(&info, running, min_mb, max_users);
    let needed_mb = min_mb.unwrap_or_else(|| memory::auto_min_available_mb(info.total_kb));
    let (open, reason) = match &admission {
        memory::Admission::Open => (true, String::new()),
        memory::Admission::Full { available_mb, needed_mb } => (false, format!("{available_mb} MB available, {needed_mb} MB needed")),
        memory::Admission::TooManyUsers { users, max } => (false, format!("{users} of {max} active users")),
    };
    let swaps = swap_devices();
    if json_out {
        let round = |x: f64| (x * 10.0).round() / 10.0;
        let users: Vec<_> = usage.iter().map(|u| json!({
            "name": u.name, "uid": u.uid, "session": u.session, "mem_mb": u.mem_mb, "swap_mb": u.swap_mb, "tasks": u.tasks,
            "cpu_percent": round(u.cpu_percent), "cap_mb": u.cap_mb,
            "top": u.top.iter().map(|(n, mb)| json!({ "name": n, "mb": mb })).collect::<Vec<_>>(),
            "video": u.video.as_ref().map(|v| json!({
                "render": v.render, "encoder": v.encoder, "compositor": v.compositor, "qp": v.qp, "fps": v.fps.map(round), "kbps": v.kbps, "current": sessionstatus::is_fresh(v),
            })),
        })).collect();
        let gpu_list: Vec<_> = gpus.iter().map(|g| json!({
            "node": g.node, "vendor": g.vendor, "model": g.model, "driver": g.driver, "pci": g.pci,
            "busy_percent": g.util.busy.map(round), "encoder_percent": g.util.encoder.map(round),
            "mem_used_mb": g.util.mem_used_mb, "mem_total_mb": g.util.mem_total_mb, "mhz": g.util.mhz, "watts": g.util.watts.map(round),
        })).collect();
        let doc = json!({
            "machine": {
                "mem_total_mb": info.total_kb / 1024, "mem_available_mb": info.available_kb / 1024,
                "swap_total_mb": info.swap_total_kb / 1024, "swap_used_mb": (info.swap_total_kb - info.swap_free_kb.min(info.swap_total_kb)) / 1024,
                "swap_devices": swaps.iter().map(|(n, size, used)| json!({ "name": n, "size_mb": size / 1024, "used_mb": used / 1024 })).collect::<Vec<_>>(),
                "load": load, "cores": cores,
                "os": os_pretty_name(&std::fs::read_to_string("/etc/os-release").unwrap_or_default()),
                "kernel": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
            },
            "compositor_setting": compositor_setting(),
            "gpus": gpu_list,
            "gpu_setting": setting,
            "admission": { "open": open, "reason": reason, "min_available_mb": needed_mb, "max_active_users": max_users, "sessions": running },
            "users": users,
        });
        println!("{}", if compact { serde_json::to_string(&doc)? } else { serde_json::to_string_pretty(&doc)? });
        return Ok(());
    }
    println!("Memory    {} in all, {} available", human_mb(info.total_kb / 1024), human_mb(info.available_kb / 1024));
    if swaps.is_empty() {
        println!("Swap      none");
    } else {
        let list: Vec<String> = swaps.iter().map(|(n, size, used)| format!("{} {} ({} used)", n.rsplit('/').next().unwrap_or(n), human_mb(size / 1024), human_mb(used / 1024))).collect();
        println!("Swap      {}", list.join(", "));
    }
    println!("Load      {} ({cores} cores)", load.join(" "));
    println!("System    {}", system_line());
    if gpus.is_empty() {
        println!("GPU       none (no render node: sessions render and encode on the CPU)");
    } else {
        for (i, g) in gpus.iter().enumerate() {
            println!("{}{}", if i == 0 { "GPU       " } else { "          " }, gpuinfo::describe(g));
        }
        println!("          new sessions use: {setting}");
    }
    println!("Desktop   {} for new sessions (WAYDESK_COMPOSITOR: sway, or hyprland, which needs a GPU)", compositor_setting());
    let limit = if max_users > 0 { format!(", at most {max_users} active users") } else { String::new() };
    if open {
        println!("Sessions  {running} running; a new one would be accepted (needs {} available{limit})", human_mb(needed_mb));
    } else {
        println!("Sessions  {running} running; a new one would be REFUSED: {reason}{limit}");
    }
    if usage.is_empty() {
        println!("\nNobody is logged in.");
        return Ok(());
    }
    println!("\n{:<18} {:<8} {:>9} {:>7} {:>6} {:>8} {:<6} {:<17} {:<9}  PROGRAMS (resident)", "ACCOUNT", "SESSION", "MEMORY", "SWAP", "CPU", "LIMIT", "RENDER", "ENCODER", "DESKTOP");
    for u in &usage {
        let top: Vec<String> = u.top.iter().map(|(n, mb)| format!("{n} {mb} MB")).collect();
        let (render, encoder) = video_cells(u.video.as_ref());
        println!(
            "{:<18} {:<8} {:>9} {:>7} {:>5.0}% {:>8} {:<6} {:<17} {:<9}  {}",
            u.name,
            if u.session { "running" } else { "-" },
            human_mb(u.mem_mb),
            if u.swap_mb > 0 { human_mb(u.swap_mb) } else { "-".into() },
            u.cpu_percent,
            u.cap_mb.map(human_mb).unwrap_or_else(|| "none".into()),
            render,
            encoder,
            u.video.as_ref().and_then(|v| v.compositor.clone()).unwrap_or_else(|| "-".into()),
            top.join(", ")
        );
    }
    Ok(())
}

// ---- the desktop of one account, and a check of its configuration ----

fn account_settings_path(uid: u32) -> PathBuf {
    Path::new(configcheck::ACCOUNT_SETTINGS_DIR).join(format!("{uid}.env"))
}

fn account_settings(uid: u32) -> configcheck::AccountSettings {
    std::fs::read_to_string(account_settings_path(uid)).ok().map(|t| configcheck::parse_account_env(&t)).unwrap_or_default()
}

/// The desktop an account's sessions start with, and where that comes from: `account` (its settings file) or `server` (/etc/default/waydesk).
fn desktop_of(uid: u32) -> (String, &'static str) {
    match account_settings(uid).desktop.as_deref().and_then(configcheck::normalize_desktop) {
        Some(d) if d != "default" => (d.to_owned(), "account"),
        _ => (compositor_setting(), "server"),
    }
}

/// The GPU setting an account's new sessions start with (`auto`, `off` or a render node), and where it comes from: `account` or `server`.
fn gpu_of(uid: u32) -> (String, &'static str) {
    match account_settings(uid).gpu {
        Some(g) => (g, "account"),
        None => (gpu_setting(), "server"),
    }
}

/// `users set NAME [--desktop sway|cosmic|hyprland|default] [--gpu auto|off|renderDN|default]`: what this account's sessions start with. The session unit reads
/// the file after /etc/default/waydesk, so it wins there; a running session keeps what it has until it is restarted.
fn users_set(name: &str, opts: &[&str]) -> Result<()> {
    require_root()?;
    let a = account(name)?;
    let (mut desktop_arg, mut gpu_arg) = (None, None);
    let mut it = opts.iter();
    while let Some(flag) = it.next() {
        match *flag {
            "--desktop" => desktop_arg = Some(*it.next().context("--desktop needs sway, cosmic, hyprland or default")?),
            "--gpu" => gpu_arg = Some(*it.next().context("--gpu needs auto, off, default or a render node such as renderD129")?),
            other => bail!("unknown option {other}"),
        }
    }
    if desktop_arg.is_none() && gpu_arg.is_none() {
        bail!("nothing to set: --desktop sway|hyprland|default and/or --gpu auto|off|renderDN|default");
    }
    let mut settings = account_settings(a.uid);
    if let Some(wanted) = desktop_arg {
        let desktop = configcheck::normalize_desktop(wanted).with_context(|| format!("{wanted:?} is not a desktop Waydesk can start (sway, cosmic, hyprland, or default for the server's setting)"))?;
        settings.desktop = (desktop != "default").then(|| desktop.to_owned());
    }
    if let Some(wanted) = gpu_arg {
        let nodes: Vec<String> = gpuinfo::list().into_iter().map(|g| g.node).collect();
        settings.gpu = configcheck::normalize_gpu(wanted, &nodes).map_err(anyhow::Error::msg)?;
    }
    let path = account_settings_path(a.uid);
    if settings.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!("cannot remove {}: {e}", path.display()),
        }
    } else {
        let dir = Path::new(configcheck::ACCOUNT_SETTINGS_DIR);
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let tmp = dir.join(format!("{}.env.tmp", a.uid));
        std::fs::write(&tmp, configcheck::render_account_env(&a.name, &settings, &opts.join(" "))).with_context(|| format!("cannot write {}", tmp.display()))?;
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        }
        std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    }
    let (now, source) = desktop_of(a.uid);
    let from = |s: &str| if s == "account" { "set for this account" } else { "the server's setting" };
    println!("{}: sessions start with {now} ({})", a.name, from(source));
    let (gpu, gsource) = gpu_of(a.uid);
    let gpu_text = match gpu.as_str() {
        "off" => "no GPU (software rendering)".to_owned(),
        "auto" => "the first usable GPU".to_owned(),
        node => node.to_owned(),
    };
    println!("{}: new sessions use {gpu_text} ({})", a.name, from(gsource));
    if now == "hyprland" && (gpuinfo::list().is_empty() || gpu == "off") {
        eprintln!("note: Hyprland has no software renderer and needs a GPU (this server has none, or the account's GPU is off): the session will not start");
    }
    match sessionstatus::read(Path::new(sessionstatus::DIR), a.uid).and_then(|s| s.compositor) {
        Some(r) if r != now => println!("{} has a session running on {r}; it changes when that session is restarted (waydesk-admin sessions end {})", a.name, a.name),
        Some(_) if gpu_arg.is_some() => println!("{} has a session running; its GPU changes when that session is restarted (waydesk-admin sessions end {})", a.name, a.name),
        _ => {}
    }
    println!("check the account's own configuration with: waydesk-admin config check {}", a.name);
    Ok(())
}

/// Runs a program as the account (never as root, so that nothing the account controls is read or run with more rights than its own), at most 20 s.
fn run_as(a: &Account, home: &str, program: &str, args: &[&str]) -> Result<(bool, String)> {
    let runuser = ["/usr/sbin/runuser", "/usr/bin/runuser"].into_iter().find(|p| Path::new(p).exists()).context("runuser is not installed")?;
    let out = Command::new("/usr/bin/timeout")
        .args(["20", runuser, "-u", &a.name, "--", "/usr/bin/env"])
        .arg(format!("HOME={home}"))
        .arg(format!("XDG_RUNTIME_DIR=/run/user/{}", a.uid))
        .arg(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("cannot run runuser")?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// A text file of the account, read as the account (at most 512 KB).
fn read_as(a: &Account, home: &str, path: &str) -> Option<String> {
    let (ok, text) = run_as(a, home, "/usr/bin/head", &["-c", "524288", "--", path]).ok()?;
    ok.then_some(text)
}

fn exists_as(a: &Account, home: &str, path: &str, flag: &str) -> bool {
    run_as(a, home, "/usr/bin/test", &[flag, path]).map(|(ok, _)| ok).unwrap_or(false)
}

fn home_of(a: &Account) -> Result<String> {
    let line = run_tool("/usr/bin/getent", &["passwd", &a.name])?;
    line.trim().split(':').nth(5).filter(|h| h.starts_with('/')).map(str::to_owned).with_context(|| format!("no home directory for {}", a.name))
}

/// Whether a program the configuration starts exists: on the PATH sessions get (Waydesk's own directory first), or at the path given.
fn program_exists(a: &Account, home: &str, program: &str) -> bool {
    if let Some(rest) = program.strip_prefix("~/") {
        return exists_as(a, home, &format!("{home}/{rest}"), "-e");
    }
    if program.starts_with('/') {
        return exists_as(a, home, program, "-e");
    }
    if program.contains('$') || program.contains('`') {
        return true; // not a name to look up
    }
    if ["/usr/lib/waydesk/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/local/sbin", "/usr/sbin", "/sbin"].iter().any(|d| Path::new(d).join(program).exists()) {
        return true;
    }
    // The account's own folders (ML4W puts its programs in ~/.local/bin, which its configuration adds to the PATH).
    [".local/bin", "bin", ".cargo/bin"].iter().any(|d| exists_as(a, home, &format!("{home}/{d}/{program}"), "-x"))
}

/// `waydesk-admin config check NAME`: whether the account's own compositor setup will work as a Waydesk session, from what the account has and what the server has.
/// Reads only; files of the account are read as the account.
fn config_check(name: &str, json_out: bool) -> Result<()> {
    use configcheck::{Finding, Level};
    require_root()?;
    let a = account(name)?;
    let home = home_of(&a)?;
    let mut f: Vec<Finding> = Vec::new();

    // The account and the server
    if in_group(&a, RDP_GROUP) {
        f.push(Finding::new(Level::Ok, "account.rdp", format!("{} may log in over RDP (group {RDP_GROUP})", a.name)));
    } else {
        f.push(Finding::new(Level::Fail, "account.rdp", format!("{} is not in the group {RDP_GROUP}, so it cannot log in over RDP", a.name)).hint(format!("waydesk-admin users enable {}", a.name)));
    }
    if Path::new(&format!("/run/user/{}", a.uid)).is_dir() {
        f.push(Finding::new(Level::Ok, "account.runtime-dir", "the account has a runtime directory (its user manager is running)"));
    } else {
        f.push(Finding::new(Level::Warn, "account.runtime-dir", "the account has no runtime directory yet: its sessions need one").hint(format!("loginctl enable-linger {}", a.name)));
    }
    let (desktop, source) = desktop_of(a.uid);
    f.push(Finding::new(Level::Info, "desktop", format!("sessions start with {desktop} ({})", if source == "account" { "set for this account" } else { "the server's setting" })));
    if !["/usr/bin/foot", "/usr/local/bin/foot"].iter().any(|p| Path::new(p).exists()) {
        f.push(Finding::new(Level::Warn, "server.foot", "foot is not installed: the first terminal of a session does not open").hint("install foot, or start the sessions with --no-terminal"));
    }

    if desktop == "hyprland" {
        // Hyprland: version, format, the configuration itself, the GPU
        let version = Command::new("Hyprland").arg("--version").stdin(Stdio::null()).output().ok().and_then(|o| configcheck::parse_hyprland_version(&String::from_utf8_lossy(&o.stdout)));
        f.push(configcheck::hyprland_version_finding(version));
        let lua = format!("{home}/.config/hypr/hyprland.lua");
        let has_lua = exists_as(&a, &home, &lua, "-e");
        f.push(configcheck::hyprland_format_finding(has_lua, exists_as(&a, &home, &format!("{home}/.config/hypr/hyprland.conf"), "-e")));
        if has_lua {
            match run_as(&a, &home, "Hyprland", &["--verify-config", "--config", &lua]) {
                Ok((ok, out)) => f.push(configcheck::verify_config_finding(&out, ok)),
                Err(e) => f.push(Finding::new(Level::Warn, "hyprland.config-valid", format!("could not run Hyprland's own check: {e:#}"))),
            }
            // What the configuration does: the Lua files of its folder (symlinks followed), read as the account
            let listing = run_as(&a, &home, "/usr/bin/find", &["-L", &format!("{home}/.config/hypr"), "-maxdepth", "5", "-name", "*.lua", "-type", "f"]).map(|(_, t)| t).unwrap_or_default();
            let mut programs: Vec<String> = Vec::new();
            let mut power = Vec::new();
            for file in listing.lines().filter(|l| l.starts_with('/')).take(400) {
                let Some(text) = read_as(&a, &home, file) else { continue };
                let short = file.strip_prefix(&format!("{home}/.config/hypr/")).unwrap_or(file);
                power.extend(configcheck::power_command_lines(&text, 3).into_iter().map(|(n, l)| (short.to_owned(), n, l)));
                for p in configcheck::hypr_exec_programs(&text) {
                    if !programs.contains(&p) {
                        programs.push(p);
                    }
                }
            }
            let lines: Vec<(usize, String)> = power.iter().take(4).map(|(file, n, l)| (*n, format!("{file}: {l}"))).collect();
            f.extend(configcheck::power_findings(&lines, "hypr"));
            let missing: Vec<&String> = programs.iter().filter(|p| !configcheck::is_common_program(p) && !program_exists(&a, &home, p)).collect();
            if missing.is_empty() {
                if !programs.is_empty() {
                    f.push(Finding::new(Level::Ok, "hyprland.autostart-programs", format!("the {} programs the configuration starts are installed", programs.len())));
                }
            } else {
                f.push(Finding::new(Level::Warn, "hyprland.autostart-programs", format!("the configuration starts programs that are not installed: {}", missing.iter().take(8).map(|s| s.as_str()).collect::<Vec<_>>().join(", "))).hint("install them; the session starts without them"));
            }
            if let Some(idle) = read_as(&a, &home, &format!("{home}/.config/hypr/hypridle.conf")) {
                f.extend(configcheck::idle_findings(&configcheck::parse_hypridle(&idle), "hypridle.conf"));
            }
        }
        // The GPU: the render node and the display node the account must open
        let gpus = gpuinfo::list();
        if gpus.is_empty() {
            f.push(Finding::new(Level::Fail, "hyprland.gpu", "this server has no GPU, and Hyprland has no software renderer: the session cannot start").hint("use sway for this account"));
        } else {
            for g in &gpus {
                let card = format!("/dev/dri/{}", g.card);
                let usable = |path: &str| exists_as(&a, &home, path, "-r") && exists_as(&a, &home, path, "-w");
                let (render_ok, card_ok) = (usable(&g.node), !g.card.is_empty() && usable(&card));
                if render_ok && card_ok {
                    f.push(Finding::new(Level::Ok, "hyprland.gpu", format!("the account can open {} and {card} ({})", g.node, g.model)));
                } else if render_ok {
                    f.push(Finding::new(Level::Fail, "hyprland.gpu", format!("the account can open {} but not {card}: Hyprland needs the display node as well", g.node))
                        .hint("give the account that node: the group video, or the rule in /usr/share/waydesk/udev/71-waydesk-hyprland-card.rules for the group waydesk-users (a choice, with security notes: docs/HYPRLAND.md)"));
                } else {
                    f.push(Finding::new(Level::Fail, "hyprland.gpu", format!("the account cannot open {} ({})", g.node, g.model)).hint(format!("it must be in the group waydesk-users (the udev rule gives that group the render node), or {RDP_GROUP} is missing")));
                }
            }
        }
    } else {
        // sway: which configuration is used, what is in it
        let home_path = Path::new(&home);
        let exists = |p: &Path| exists_as(&a, &home, &p.to_string_lossy(), "-f");
        if !["/usr/lib/waydesk/bin/sway", "/usr/bin/sway", "/usr/local/bin/sway"].iter().any(|p| Path::new(p).exists()) {
            f.push(Finding::new(Level::Fail, "sway.binary", "sway is not installed").hint("install sway (or waydesk-sway)"));
        }
        let source = configcheck::sway_source_for(Some(home_path), &desktop, exists);
        let (path, why) = match &source {
            configcheck::SwaySource::AccountWaydesk(p) => (Some(p.clone()), "the account's own file for Waydesk sessions".to_owned()),
            configcheck::SwaySource::SystemWaydesk(p) => (Some(p.clone()), "the Waydesk desktop package's configuration".to_owned()),
            configcheck::SwaySource::SwayLookup(p) => (Some(p.clone()), "sway's own lookup (the account's ~/.config/sway/config, else the system's)".to_owned()),
            configcheck::SwaySource::Minimal(p) => (Some(p.clone()), "the minimal configuration of the package (nothing else was found)".to_owned()),
            configcheck::SwaySource::Nothing => (None, String::new()),
        };
        match &path {
            Some(p) => f.push(Finding::new(Level::Info, "sway.config", format!("sessions start sway with {} ({why})", p.display()))),
            None => f.push(Finding::new(Level::Fail, "sway.config", "no sway configuration was found and the minimal one is not installed: sway would not start")),
        }
        if let Some(ignored) = configcheck::ignored_account_sway_config(Some(home_path), |p| exists_as(&a, &home, &p.to_string_lossy(), "-f")) {
            f.push(Finding::new(Level::Warn, "sway.own-config-ignored", format!("{} is not used: the Waydesk desktop package's configuration takes priority", ignored.display()))
                .hint(format!("to use it, put it (or a link to it) at {home}/.config/waydesk/config, or leave the desktop package out")));
            // What it would do if it were used: that is the next thing the administrator will want to know.
            if let Some(text) = read_as(&a, &home, &ignored.to_string_lossy()) {
                let label = format!("{} (not in use)", ignored.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                f.extend(configcheck::sway_config_findings(&text, &label));
                f.extend(configcheck::power_findings(&configcheck::power_command_lines(&text, 3), &label));
            }
        }
        if let Some(p) = path {
            let mut texts = Vec::new();
            if let Some(main) = read_as(&a, &home, &p.to_string_lossy()) {
                for inc in configcheck::sway_includes(&main).into_iter().take(6) {
                    let inc = inc.replacen('~', &home, 1);
                    let inc = if inc.starts_with('/') { inc } else { format!("{}/{inc}", p.parent().map(|d| d.display().to_string()).unwrap_or_default()) };
                    if let Some(t) = read_as(&a, &home, &inc) {
                        texts.push((inc, t));
                    }
                }
                texts.insert(0, (p.to_string_lossy().into_owned(), main));
            } else {
                f.push(Finding::new(Level::Warn, "sway.config-unreadable", format!("{} cannot be read by the account", p.display())));
            }
            for (file, text) in &texts {
                f.extend(configcheck::sway_config_findings(text, file.rsplit('/').next().unwrap_or(file)));
                f.extend(configcheck::power_findings(&configcheck::power_command_lines(text, 3), file.rsplit('/').next().unwrap_or(file)));
            }
        }
    }

    f.sort_by_key(|x| std::cmp::Reverse(x.level));
    let worst = f.iter().map(|x| x.level).max().unwrap_or(Level::Ok);
    if json_out {
        let list: Vec<_> = f.iter().map(|x| json!({ "level": x.level.word(), "id": x.id, "message": x.message, "hint": x.hint })).collect();
        println!("{}", serde_json::to_string_pretty(&json!({ "account": a.name, "desktop": desktop, "result": worst.word(), "findings": list }))?);
    } else {
        println!("waydesk config check for {} ({desktop})\n", a.name);
        for x in &f {
            println!("{:<5} {}", x.level.word(), x.message);
            if let Some(h) = &x.hint {
                println!("      -> {h}");
            }
        }
        let count = |l: Level| f.iter().filter(|x| x.level == l).count();
        println!("\n{} problems, {} warnings", count(Level::Fail), count(Level::Warn));
    }
    if worst == Level::Fail {
        std::process::exit(1);
    }
    Ok(())
}

// ---- limits of one account ----

/// "8G", "512M", "100K", "2T" in bytes; 0 is allowed (no swap at all).
fn parse_size_bytes(s: &str) -> Option<u64> {
    if s == "0" {
        return Some(0);
    }
    let (digits, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = digits.parse().ok().filter(|n| *n > 0)?;
    let factor: u64 = match unit {
        "K" | "k" => 1 << 10,
        "M" | "m" => 1 << 20,
        "G" | "g" => 1 << 30,
        "T" | "t" => 1 << 40,
        _ => return None,
    };
    n.checked_mul(factor)
}

/// The systemd properties for `users limit` options (the memory ceiling also sets a high mark at 90 %, where the kernel starts
/// reclaiming from the account instead of letting it grow to the ceiling).
fn limit_properties(opts: &[&str]) -> Result<Vec<String>> {
    let mut props = Vec::new();
    let mut it = opts.iter();
    while let Some(flag) = it.next() {
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match *flag {
            "--memory" => {
                let max = parse_size_bytes(value).filter(|b| *b > 0).with_context(|| format!("{value:?} is not a size (for example 6G or 800M)"))?;
                props.push(format!("MemoryHigh={}", max / 10 * 9));
                props.push(format!("MemoryMax={max}"));
            }
            "--swap" => props.push(format!("MemorySwapMax={}", parse_size_bytes(value).with_context(|| format!("{value:?} is not a size (for example 2G, or 0 for none)"))?)),
            "--cpu-weight" => {
                let w: u32 = value.parse().ok().filter(|w| (1..=10000).contains(w)).with_context(|| format!("{value:?} is not a CPU weight (1 to 10000, the default is 100)"))?;
                props.push(format!("CPUWeight={w}"));
            }
            "--tasks" => {
                let t: u32 = value.parse().ok().filter(|t| *t >= 64).with_context(|| format!("{value:?} is not a number of tasks (64 or more)"))?;
                props.push(format!("TasksMax={t}"));
            }
            other => bail!("unknown option {other} for limit"),
        }
    }
    ensure!(!props.is_empty(), "say what to limit: --memory, --swap, --cpu-weight or --tasks (or show, or reset)");
    Ok(props)
}

fn users_limit(name: &str, rest: &[&str]) -> Result<()> {
    let a = account(name)?;
    let unit = format!("user-{}.slice", a.uid);
    match rest {
        [] | ["show"] => {
            ensure!(visible_accounts()?.iter().any(|v| v.uid == a.uid), "only an administrator may look at the limits of another account");
            let out = run_tool("/usr/bin/systemctl", &["show", &unit, "-p", "MemoryHigh", "-p", "MemoryMax", "-p", "MemorySwapMax", "-p", "CPUWeight", "-p", "TasksMax"])?;
            println!("limits of {} ({unit}):\n{}", a.name, out.trim_end());
            Ok(())
        }
        ["reset"] => {
            require_root()?;
            run_tool("/usr/bin/systemctl", &["revert", &unit])?;
            println!("{} has the default limits again", a.name);
            Ok(())
        }
        opts => {
            require_root()?;
            let props = limit_properties(opts)?;
            let mut args = vec!["set-property", unit.as_str()];
            args.extend(props.iter().map(String::as_str));
            run_tool("/usr/bin/systemctl", &args)?;
            println!("limits of {} set: {}", a.name, props.join(" "));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_takes_an_optional_number_of_seconds() {
        assert_eq!(parse_watch(&[]).unwrap(), None);
        assert_eq!(parse_watch(&["--watch"]).unwrap(), Some(std::time::Duration::from_secs(2)));
        assert_eq!(parse_watch(&["--watch", "5"]).unwrap(), Some(std::time::Duration::from_secs(5)));
        assert_eq!(parse_watch(&["--watch=3"]).unwrap(), Some(std::time::Duration::from_secs(3)));
        assert!(parse_watch(&["--watch", "0"]).is_err(), "a refresh takes a second of sampling");
        assert!(parse_watch(&["--watch", "soon"]).is_err());
        assert!(parse_watch(&["--watch", "99999"]).is_err());
    }

    #[test]
    fn removing_an_account_removes_its_settings_and_placement_files_and_only_those() {
        let dir = std::env::temp_dir().join(format!("waydesk-forget-{}", std::process::id()));
        let (a, b) = (dir.join("settings"), dir.join("gpu"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        for f in [a.join("1002.env"), b.join("1002.env"), a.join("1003.env")] {
            std::fs::write(f, "x").unwrap();
        }
        forget_account_files(1002, &[&a, &b]);
        forget_account_files(1002, &[&a, &b]); // again: nothing left, no error
        assert!(!a.join("1002.env").exists() && !b.join("1002.env").exists());
        assert!(a.join("1003.env").exists(), "another account's file stays");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_session_cells_say_what_runs_and_how_it_does() {
        use crate::sessionstatus::Status;
        assert_eq!(video_cells(None), ("-".to_string(), "-".to_string()));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let gpu = Status { render: Some("/dev/dri/renderD129".into()), encoder: Some("nvenc".into()), compositor: None, qp: Some(25), fps: Some(17.4), kbps: Some(9000), stats_at: now };
        assert_eq!(video_cells(Some(&gpu)), ("gpu".to_string(), "nvenc q25 17fps".to_string()));
        let soft = Status { render: None, encoder: Some("openh264".into()), compositor: None, qp: None, fps: Some(9.0), kbps: None, stats_at: now };
        assert_eq!(video_cells(Some(&soft)), ("cpu".to_string(), "openh264 9fps".to_string()));
        let idle = Status { stats_at: now - 600, ..gpu };
        assert_eq!(video_cells(Some(&idle)).1, "nvenc", "old statistics are not shown as current");
    }

    #[test]
    fn names_are_what_useradd_takes_and_nothing_that_could_be_an_option_or_a_path() {
        for ok in ["jose", "a", "_x", "rdp-test_2", &"a".repeat(32)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "Jose", "1abc", "-r", "--delete", "a b", "a/b", "../x", "a:b", "a\nb", "ä", &"a".repeat(33), "root;id", "$(x)", "a\0b"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn passwords_stay_on_one_line() {
        assert!(valid_password("correct horse battery"));
        assert!(valid_password("with:colon and spaces "));
        assert!(!valid_password(""));
        assert!(!valid_password("a\nroot:x"), "a second line would be a second account for chpasswd");
        assert!(!valid_password("a\rb"));
        assert!(!valid_password("a\0b"));
        assert!(!valid_password(&"x".repeat(129)));
    }

    #[test]
    fn generated_passwords_have_the_shape_and_the_alphabet() {
        let p = make_password(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 31]);
        assert_eq!(p, "abcd-efgh-ijkl-mno7");
        assert_eq!(p.len(), 19);
        assert!(p.chars().all(|c| c == '-' || c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
        assert!(valid_password(&p));
        // 256 is a multiple of 32: every letter is as likely as any other.
        assert_eq!(256 % 32, 0);
    }

    #[test]
    fn the_last_member_of_a_group_is_recognised() {
        let one = vec!["a".to_string()];
        let two = vec!["a".to_string(), "b".to_string()];
        assert!(would_leave_nobody(&one, "a"));
        assert!(!would_leave_nobody(&one, "b"), "not a member: nothing changes");
        assert!(!would_leave_nobody(&two, "a"));
        assert!(!would_leave_nobody(&[], "a"));
    }

    #[test]
    fn a_line_of_getent_becomes_an_account() {
        let a = parse_passwd("rdptest2:x:1002:1002:,,,:/home/rdptest2:/bin/bash").unwrap();
        assert_eq!((a.name.as_str(), a.uid, a.gid, a.shell.as_str()), ("rdptest2", 1002, 1002, "/bin/bash"));
        assert!(parse_passwd("broken:line").is_none());
        assert!(parse_passwd("x:x:notanumber:1:::/bin/sh").is_none());
    }

    #[test]
    fn the_distribution_name_comes_out_of_os_release() {
        assert_eq!(os_pretty_name("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n"), Some("Arch Linux".into()));
        assert_eq!(os_pretty_name("PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n"), Some("Debian GNU/Linux 13 (trixie)".into()));
        assert_eq!(os_pretty_name("NAME=Foo\n"), None);
        assert_eq!(os_pretty_name("PRETTY_NAME=\"\"\n"), None);
    }

    #[test]
    fn login_shells_are_told_from_service_shells() {
        let listed = "# shells\n/bin/sh\n/bin/bash\n";
        assert!(login_shell("/bin/bash", listed));
        assert!(!login_shell("/usr/sbin/nologin", listed));
        assert!(!login_shell("", listed));
        assert!(!login_shell("/bin/zsh", listed));
    }

    #[test]
    fn flags_are_taken_out_of_the_arguments() {
        let mut args: Vec<String> = ["users", "list", "--json"].iter().map(|s| s.to_string()).collect();
        assert!(take_flag(&mut args, "--json"));
        assert_eq!(args, vec!["users", "list"]);
        assert!(!take_flag(&mut args, "--json"));
    }

    #[test]
    fn sizes_are_read_with_their_unit() {
        assert_eq!(parse_size_bytes("8G"), Some(8 << 30));
        assert_eq!(parse_size_bytes("512m"), Some(512 << 20));
        assert_eq!(parse_size_bytes("0"), Some(0));
        for bad in ["", "G", "8", "8GB", "-1G", "1.5G", "0G", "99999999999T", "8 G"] {
            assert_eq!(parse_size_bytes(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn limits_become_systemd_properties_and_nothing_else() {
        assert_eq!(limit_properties(&["--memory", "10G"]).unwrap(), vec!["MemoryHigh=9663676416", "MemoryMax=10737418240"]);
        assert_eq!(limit_properties(&["--cpu-weight", "200", "--swap", "0", "--tasks", "4096"]).unwrap(), vec!["CPUWeight=200", "MemorySwapMax=0", "TasksMax=4096"]);
        for bad in [&["--memory", "0"][..], &["--memory"], &["--cpu-weight", "0"], &["--cpu-weight", "10001"], &["--tasks", "5"], &["--nice", "1"], &[], &["--memory", "1G;rm"]] {
            assert!(limit_properties(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_policy_of_the_helper_is_read_from_its_arguments() {
        let args: Vec<String> = ["waydesk-server", "--helper", "--min-available-mb", "1200", "--max-active-users", "6"].iter().map(|s| s.to_string()).collect();
        assert_eq!(policy_from_args(&args), (Some(1200), 6));
        assert_eq!(policy_from_args(&args[..2]), (None, 0));
    }

    #[test]
    fn sizes_are_shown_in_mb_or_gb() {
        assert_eq!(human_mb(620), "620 MB");
        assert_eq!(human_mb(1536), "1.5 GB");
    }

    #[test]
    fn an_ordinary_account_sees_itself_and_an_administrator_everybody() {
        let acct = |name: &str, uid: u32| Account { name: name.into(), uid, gid: uid, shell: "/bin/bash".into() };
        let all = vec![acct("jose", 1000), acct("ana", 1001), acct("rui", 1002)];
        let names = |v: Vec<Account>| v.into_iter().map(|a| a.name).collect::<Vec<_>>();
        assert_eq!(names(only_visible(all.clone(), 1001, false)), vec!["ana"]);
        assert_eq!(names(only_visible(all.clone(), 1001, true)), vec!["jose", "ana", "rui"]);
        assert_eq!(names(only_visible(all.clone(), 0, false)), vec!["jose", "ana", "rui"], "root");
        assert!(only_visible(all, 33, false).is_empty(), "a system account has no row");
    }
}
