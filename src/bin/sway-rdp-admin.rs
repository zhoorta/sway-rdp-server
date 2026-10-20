//! `sway-rdp-admin`: who may use the remote desktop, and what they have running.
//!
//!   sway-rdp-admin users list [--json]
//!   sway-rdp-admin users add NAME [--admin] [--sudo] [--no-rdp] [--json]
//!   sway-rdp-admin users remove NAME [--delete-home]
//!   sway-rdp-admin users passwd NAME [--stdin] [--json]
//!   sway-rdp-admin users enable NAME | disable NAME [--keep-session]
//!   sway-rdp-admin users admin NAME on|off       (the group sway-rdp-admins: Flatpak installs without a password)
//!   sway-rdp-admin users sudo NAME on|off        (the group sudo)
//!   sway-rdp-admin sessions list [--json]
//!   sway-rdp-admin sessions end NAME
//!
//! Listing is open to everybody (the account database is). Everything else needs root (`sudo`, or `pkexec` from the panel).
//! The work is done by the system's own tools (useradd, userdel, gpasswd, chpasswd, systemctl, loginctl), started by their
//! absolute path with fixed arguments and no shell, after every name and password has been checked. Passwords only travel
//! over a pipe to `chpasswd`, never as arguments, and are never logged. Accounts below uid 1000 and root are never touched.

use std::ffi::CString;
use std::io::{Read as _, Write as _};
use std::os::raw::c_char;
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context as _, Result};
use serde_json::json;

const RDP_GROUP: &str = "sway-rdp-users";
const ADMIN_GROUP: &str = "sway-rdp-admins";
const SUDO_GROUP: &str = "sudo";
const MIN_UID: u32 = 1000;
const NOBODY: u32 = 65534;

fn main() {
    if let Err(e) = run() {
        eprintln!("sway-rdp-admin: {e:#}");
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
    format!("sway-rdp-session@{uid}.service")
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
        ["sessions", "list"] => sessions_list(json_out),
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
usage: sway-rdp-admin users list [--json]
       sway-rdp-admin users add NAME [--admin] [--sudo] [--no-rdp] [--json]
       sway-rdp-admin users remove NAME [--delete-home]
       sway-rdp-admin users passwd NAME [--stdin] [--json]
       sway-rdp-admin users enable NAME | disable NAME [--keep-session]
       sway-rdp-admin users admin NAME on|off   (Flatpak installs without a password)
       sway-rdp-admin users sudo NAME on|off
       sway-rdp-admin sessions list [--json] | end NAME
";

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
        eprintln!("note: the group {RDP_GROUP} does not exist, so nobody may log in (is the sway-rdp package installed?)");
    }
    let mut rows = Vec::new();
    for a in accounts()? {
        let (session, since) = session_state(a.uid);
        rows.push((a.name.clone(), a.uid, in_group(&a, RDP_GROUP), in_group(&a, ADMIN_GROUP), in_group(&a, SUDO_GROUP), login_shell(&a.shell, &shells), session, since));
    }
    if json_out {
        let list: Vec<_> = rows
            .iter()
            .map(|(name, uid, rdp, admin, sudo, shell, session, since)| {
                json!({ "name": name, "uid": uid, "rdp": rdp, "flatpak_admin": admin, "sudo": sudo, "login_shell": shell, "session": session, "session_since": since })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let yes = |v: bool| if v { "yes" } else { "-" };
    println!("{:<20} {:>5}  {:<4} {:<8} {:<5} {:<7} SESSION", "ACCOUNT", "UID", "RDP", "FLATPAK", "SUDO", "SHELL");
    for (name, uid, rdp, admin, sudo, shell, session, _) in &rows {
        println!("{:<20} {:>5}  {:<4} {:<8} {:<5} {:<7} {}", name, uid, yes(*rdp), yes(*admin), yes(*sudo), yes(*shell), session);
    }
    Ok(())
}

fn sessions_list(json_out: bool) -> Result<()> {
    let mut rows = Vec::new();
    for a in accounts()? {
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
    ensure!(group_gid(group).is_some(), "the group {group} does not exist (is the sway-rdp package installed?)");
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
        println!("The account can change it at once: open a terminal on the desktop and run passwd.");
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
    println!("removed {}{}", a.name, if rest.contains(&"--delete-home") { " and its home folder" } else { " (its home folder was kept)" });
    Ok(())
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
        println!("new password for {} (shown once): {password}", a.name);
    } else {
        println!("changed the password of {}", a.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
