//! macOS permission to record system audio ("Screen & System Audio Recording").
//!
//! There is no public API to check or request it, so this uses the private TCC
//! framework (as Apple's own tools and AudioCap do): `TCCAccessPreflight` for
//! the current state. The prompt itself appears the first time a process tap is
//! started. macOS attributes the permission to the *responsible* app, which is
//! the terminal (or whatever launched us), not to this binary.
//!
//! Once the user denies it, macOS never asks again by itself. So when we find
//! it denied we reset that decision (`tccutil reset AudioCapture <app>`): the
//! run fails with instructions, and the next run with `--system` asks again.

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const SERVICE: &str = "kTCCServiceAudioCapture";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
    /// Never asked (or the private API is unavailable).
    Unknown,
}

type CFStringRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn responsibility_get_pid_responsible_for_pid(pid: c_int) -> c_int;
    fn proc_pidpath(pid: c_int, buf: *mut c_void, size: u32) -> c_int;
}

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const RTLD_LAZY: c_int = 1;

/// The current state of the permission, for the app responsible for us.
pub fn access() -> Access {
    type Preflight = unsafe extern "C" fn(CFStringRef, *const c_void) -> c_int;
    unsafe {
        let lib = dlopen(c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(), RTLD_LAZY);
        if lib.is_null() {
            return Access::Unknown;
        }
        let sym = dlsym(lib, c"TCCAccessPreflight".as_ptr());
        if sym.is_null() {
            return Access::Unknown;
        }
        let preflight: Preflight = std::mem::transmute(sym);
        let service = CString::new(SERVICE).unwrap();
        let s = CFStringCreateWithCString(std::ptr::null(), service.as_ptr(), CF_STRING_ENCODING_UTF8);
        let r = preflight(s, std::ptr::null());
        CFRelease(s);
        match r {
            0 => Access::Granted,
            1 => Access::Denied,
            _ => Access::Unknown,
        }
    }
}

/// Wait for the user to answer the prompt (which a just-started tap shows).
pub fn wait_for_answer(timeout: Duration) -> Access {
    let start = Instant::now();
    loop {
        let a = access();
        if a != Access::Unknown || start.elapsed() > timeout {
            return a;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// The app macOS holds responsible for this process: (name, bundle id).
pub fn responsible_app() -> Option<(String, String)> {
    let pid = unsafe { responsibility_get_pid_responsible_for_pid(std::process::id() as c_int) };
    if pid <= 0 {
        return None;
    }
    let mut buf = [0u8; 4096];
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    let exe = PathBuf::from(CStr::from_bytes_until_nul(&buf).ok()?.to_str().ok()?);
    // The outermost .app bundle in the path (helpers live inside their app).
    let app = exe.ancestors().filter(|p| p.extension().is_some_and(|e| e == "app")).last()?;
    let name = app.file_stem()?.to_string_lossy().into_owned();
    Some((name, bundle_id(app)?))
}

fn bundle_id(app: &Path) -> Option<String> {
    let info = app.join("Contents/Info");
    let out = Command::new("/usr/bin/defaults").arg("read").arg(&info).arg("CFBundleIdentifier").output().ok()?;
    let id = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !id.is_empty()).then_some(id)
}

/// Forget the user's earlier "Don't Allow", so macOS asks again next time.
pub fn reset(bundle_id: &str) -> bool {
    Command::new("/usr/bin/tccutil")
        .args(["reset", "AudioCapture", bundle_id])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The message for a denied permission. Resets the decision first, so the
/// next run asks again.
pub fn denied_message() -> String {
    let app = responsible_app();
    let who = app.as_ref().map(|(n, _)| n.as_str()).unwrap_or("the app you started synestesia from");
    let reset_done = app.as_ref().is_some_and(|(_, id)| reset(id));
    let mut s = format!(
        "--system needs permission to record system audio, and {who} does not have it.\n\
         \n\
         To allow it:\n"
    );
    if reset_done {
        s += &format!("  - Run synestesia with --system again: macOS will ask, choose Allow.\n");
    }
    s += &format!(
        "  - {}Open System Settings > Privacy & Security > Screen & System Audio Recording,\n    \
         and turn on {who} under \"System Audio Recording Only\".\n    \
         (open \"x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture\")\n",
        if reset_done { "Or: " } else { "" }
    );
    s += "\nOr use the microphone instead (no --system).";
    s
}
