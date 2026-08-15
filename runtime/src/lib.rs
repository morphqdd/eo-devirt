//! What a compiled EO program leans on while it runs.
//!
//! The compiler emits calls into this by name, so everything here is `extern
//! "C"` and unmangled. It is linked into the binary and reaches the operating
//! system through libc, which is also the route Windows will need, having no
//! stable syscall numbering of its own.

use std::ffi::CStr;
use std::io::Write;

/// How many arguments a system call may be handed.
const ARGUMENTS: usize = 4;

unsafe extern "C" {
    fn close(descriptor: i32) -> i32;
    fn getpid() -> i32;
}

/// Write out a dataized number, the way the Java runtime prints one.
#[unsafe(no_mangle)]
pub extern "C" fn eo_print(value: f64) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value:?}");
    let _ = out.flush();
}

/// Make the system call the program named, and hand back its code.
///
/// The name arrives already decided: it is a literal at every call site the
/// runtime library has, so the compiler folds it rather than leaving a string
/// to be read here, which is what the Java runtime does instead.
///
/// # Safety
///
/// Called from generated code, which passes a pointer to a string it holds
/// static and as many arguments as it says it does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eo_posix(
    name: *const u8,
    count: usize,
    first: f64,
    second: f64,
    third: f64,
    fourth: f64,
) -> f64 {
    let handed = [first, second, third, fourth];
    let named = match unsafe { CStr::from_ptr(name.cast()) }.to_str() {
        Ok(named) => named,
        Err(_) => refuse("a name that is not text"),
    };
    if count > ARGUMENTS {
        refuse("more arguments than a system call takes here");
    }
    match named {
        "close" => f64::from(unsafe { close(handed[0] as i32) }),
        "getpid" => f64::from(unsafe { getpid() }),
        other => refuse(other),
    }
}

/// Stop, saying what was asked for and could not be done.
fn refuse(what: &str) -> ! {
    let mut out = std::io::stderr().lock();
    let _ = writeln!(out, "eo: {what} is not implemented");
    let _ = out.flush();
    std::process::abort()
}
