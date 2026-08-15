//! What a compiled EO program leans on while it runs.
//!
//! The compiler emits calls into this by name, so everything here is `extern
//! "C"` and unmangled. It is linked into the binary and reaches the operating
//! system through libc, which is also the route Windows will need, having no
//! stable syscall numbering of its own.

use std::{cell::RefCell, ffi::CStr, io::Write};

/// How many arguments a system call may be handed.
const ARGUMENTS: usize = 4;

/// Report a dataized number, the way the Java runtime reports one.
///
/// This goes to the error stream, not the output one. What a program writes
/// for itself is its own, and the value it dataizes to is something the
/// harness says about it, so the two must not run together. The Java runtime
/// draws the same line.
#[unsafe(no_mangle)]
pub extern "C" fn eo_print(value: f64) {
    let mut out = std::io::stderr().lock();
    let _ = writeln!(out, "{value:?}");
    let _ = out.flush();
}

/// Make the system call the program named, and hand back its code.
///
/// The name arrives already decided: it is a literal at every call site the
/// runtime library has, so the compiler folds it rather than leaving a string
/// to be read here, which is what the Java runtime does instead.
///
/// Arguments arrive as integers, which is what a system call takes: a
/// descriptor, a count, or the address of bytes the program laid down.
///
/// # Safety
///
/// Called from generated code, which passes a pointer to a string it holds
/// static and as many arguments as it says it does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eo_posix(
    name: *const u8,
    count: usize,
    first: i64,
    second: i64,
    third: i64,
    fourth: i64,
) -> f64 {
    let handed = [first, second, third, fourth];
    let Ok(named) = unsafe { CStr::from_ptr(name.cast()) }.to_str() else {
        refuse("a name that is not text")
    };
    if count > ARGUMENTS {
        refuse("more arguments than a system call takes here");
    }
    match named {
        "close" => f64::from(unsafe { libc::close(handed[0] as i32) }),
        "getpid" => f64::from(unsafe { libc::getpid() }),
        "write" => unsafe {
            libc::write(
                handed[0] as i32,
                handed[1] as *const libc::c_void,
                handed[2] as usize,
            ) as f64
        },
        other => refuse(other),
    }
}

// Where objects live. They are never freed: a program that runs to an answer
// and stops does not need them to be, and freeing them properly means knowing
// which are still reachable, which is a question this does not answer yet.
thread_local! {
    static ARENA: RefCell<Vec<Box<[u64]>>> = const { RefCell::new(Vec::new()) };
}

/// An object is a shape and then one slot per attribute the shape names.
///
/// The slot at index `i` holds whatever the attribute named at `i` in the
/// shape is bound to. A number carries its value in the first slot instead,
/// its shape naming nothing.
const HEADER: usize = 2;

/// Where a number keeps itself, in the slot after the header.
const DATUM: usize = 1;

/// Make an object of one shape, with room for what the shape names.
///
/// A shape is a run of words the compiler laid down: how many attributes, then
/// their names, then the body of each. It is read here and never written, so
/// two objects of a kind share one and nothing is copied.
///
/// # Safety
///
/// Called from generated code, which passes a shape it laid down itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eo_make(shape: *const u64) -> *mut u64 {
    let count = unsafe { *shape } as usize;
    let mut room = vec![0u64; HEADER + count].into_boxed_slice();
    room[0] = shape as u64;
    let at = room.as_mut_ptr();
    ARENA.with(|arena| arena.borrow_mut().push(room));
    at
}

/// Wrap a number so it can be held where an object is held.
#[unsafe(no_mangle)]
pub extern "C" fn eo_number(value: f64) -> *mut u64 {
    let mut room = vec![0u64; HEADER + DATUM].into_boxed_slice();
    room[0] = 0;
    room[HEADER] = value.to_bits();
    let at = room.as_mut_ptr();
    ARENA.with(|arena| arena.borrow_mut().push(room));
    at
}

/// The number an object is, when it is one.
///
/// # Safety
///
/// Called from generated code with an object it made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eo_as_number(object: *const u64) -> f64 {
    if unsafe { *object } != 0 {
        refuse("asking for the number of something that is not one");
    }
    f64::from_bits(unsafe { *object.add(HEADER) })
}

/// Find what an object binds under a name, the name being a number the
/// compiler interned.
///
/// This is the lookup the whole compiler exists to avoid: it runs only where
/// the shape of the object could not be worked out ahead of time.
///
/// An attribute is a body, not a value. It runs the first time it is asked
/// for, with the object as what it was dispatched from, and the answer is kept
/// in the slot so it runs once. That is what makes an attribute nobody asks
/// for cost nothing.
///
/// # Safety
///
/// Called from generated code with an object it made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eo_dispatch(object: *const u64, name: u64) -> *const u64 {
    let shape = unsafe { *object } as *const u64;
    if shape.is_null() {
        refuse("dispatching on a number");
    }
    let count = unsafe { *shape } as usize;
    for slot in 0..count {
        if unsafe { *shape.add(1 + slot) } != name {
            continue;
        }
        let held = unsafe { *object.add(HEADER + slot) };
        if held != 0 {
            return held as *const u64;
        }
        let body = unsafe { *shape.add(1 + count + slot) };
        if body == 0 {
            refuse("an attribute with nothing behind it");
        }
        let run: extern "C" fn(*const u64) -> *const u64 =
            unsafe { std::mem::transmute::<u64, extern "C" fn(*const u64) -> *const u64>(body) };
        let made = run(object);
        unsafe { *object.cast_mut().add(HEADER + slot) = made as u64 };
        return made;
    }
    refuse("an attribute the object does not have")
}

/// Stop, saying what was asked for and could not be done.
fn refuse(what: &str) -> ! {
    let mut out = std::io::stderr().lock();
    let _ = writeln!(out, "eo: {what} is not implemented");
    let _ = out.flush();
    std::process::abort()
}
