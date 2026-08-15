//! What a compiled EO program leans on while it runs.
//!
//! The compiler emits calls into this by name, so everything here is `extern
//! "C"` and unmangled. It is linked into the binary and reaches the operating
//! system through libc, which is also the route Windows will need, having no
//! stable syscall numbering of its own.

use std::io::Write;

/// Write out a dataized number, the way the Java runtime prints one.
///
/// # Safety
///
/// Called from generated code, which passes one double and nothing else.
#[unsafe(no_mangle)]
pub extern "C" fn eo_print(value: f64) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value:?}");
    let _ = out.flush();
}
