//! Minimal runtime for a DLL that links without the C runtime.
//!
//! The MSVC target expects `memcpy` and friends from the CRT, an `_fltused` marker for
//! floating point and a C++ exception personality routine referenced by the precompiled
//! `core` unwind tables; `no_std` also needs a panic handler. The CRT-named symbols are
//! defined through `global_asm!` (a jump to a private Rust function) so that they are
//! *not* exported from the DLL, which should expose only `RunDll` and `DllMain`.
//!
//! The memory routines use volatile accesses so the compiler cannot turn the loops back
//! into calls to themselves.

use core::ffi::c_void;
use core::panic::PanicInfo;

/// The hook runs inside other applications; a panic there must not hang the thread, so
/// it traps instead (the code is written to be panic-free, this is the backstop).
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    trap()
}

/// Raises a fatal exception (illegal instruction / breakpoint) and never returns.
#[inline(always)]
fn trap() -> ! {
    // SAFETY: the instruction faults and never falls through.
    unsafe {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        core::arch::asm!("ud2", options(noreturn));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("brk #0xf003", options(noreturn));
    }
}

unsafe extern "C" fn wg_memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dest as *mut u8, src as *const u8);
    let mut i = 0;
    while i < n {
        core::ptr::write_volatile(d.add(i), core::ptr::read_volatile(s.add(i)));
        i += 1;
    }
    dest
}

unsafe extern "C" fn wg_memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dest as *mut u8, src as *const u8);
    if (d as usize) <= (s as usize) {
        let mut i = 0;
        while i < n {
            core::ptr::write_volatile(d.add(i), core::ptr::read_volatile(s.add(i)));
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            core::ptr::write_volatile(d.add(i), core::ptr::read_volatile(s.add(i)));
        }
    }
    dest
}

unsafe extern "C" fn wg_memset(dest: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let mut i = 0;
    while i < n {
        core::ptr::write_volatile(d.add(i), c as u8);
        i += 1;
    }
    dest
}

unsafe extern "C" fn wg_memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a as *const u8, b as *const u8);
    let mut i = 0;
    while i < n {
        let (x, y) = (core::ptr::read_volatile(a.add(i)), core::ptr::read_volatile(b.add(i)));
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        i += 1;
    }
    0
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl memcpy",
    "memcpy:",
    "jmp {memcpy}",
    ".globl memmove",
    "memmove:",
    "jmp {memmove}",
    ".globl memset",
    "memset:",
    "jmp {memset}",
    ".globl memcmp",
    "memcmp:",
    "jmp {memcmp}",
    ".globl bcmp",
    "bcmp:",
    "jmp {memcmp}",
    // Never called: only referenced by unwind tables, and we build with panic = "abort".
    ".globl __CxxFrameHandler3",
    "__CxxFrameHandler3:",
    "xor eax, eax",
    "ret",
    ".section .rdata,\"dr\"",
    ".globl _fltused",
    "_fltused:",
    ".long 0",
    ".text",
    memcpy = sym wg_memcpy,
    memmove = sym wg_memmove,
    memset = sym wg_memset,
    memcmp = sym wg_memcmp,
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".globl memcpy",
    "memcpy:",
    "b {memcpy}",
    ".globl memmove",
    "memmove:",
    "b {memmove}",
    ".globl memset",
    "memset:",
    "b {memset}",
    ".globl memcmp",
    "memcmp:",
    "b {memcmp}",
    ".globl bcmp",
    "bcmp:",
    "b {memcmp}",
    ".globl __CxxFrameHandler3",
    "__CxxFrameHandler3:",
    "mov w0, #0",
    "ret",
    ".section .rdata,\"dr\"",
    ".globl _fltused",
    "_fltused:",
    ".long 0",
    ".text",
    memcpy = sym wg_memcpy,
    memmove = sym wg_memmove,
    memset = sym wg_memset,
    memcmp = sym wg_memcmp,
);

#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl _memcpy",
    "_memcpy:",
    "jmp {memcpy}",
    ".globl _memmove",
    "_memmove:",
    "jmp {memmove}",
    ".globl _memset",
    "_memset:",
    "jmp {memset}",
    ".globl _memcmp",
    "_memcmp:",
    "jmp {memcmp}",
    ".globl _bcmp",
    "_bcmp:",
    "jmp {memcmp}",
    ".globl ___CxxFrameHandler3",
    "___CxxFrameHandler3:",
    "xor eax, eax",
    "ret",
    ".section .rdata,\"dr\"",
    ".globl __fltused",
    "__fltused:",
    ".long 0",
    ".text",
    memcpy = sym wg_memcpy,
    memmove = sym wg_memmove,
    memset = sym wg_memset,
    memcmp = sym wg_memcmp,
);

/// 32-bit MSVC code calls `__aulldiv` / `__aullrem` for 64-bit unsigned division. They are
/// only reachable from `core`'s panic-message formatting, which our panic handler never
/// runs, but they are implemented correctly (restoring long division, no `/` on `u64`
/// that would recurse) in case that ever changes. Both use the stdcall-style helper
/// convention: arguments on the stack, callee pops 16 bytes, result in `edx:eax`.
#[cfg(target_arch = "x86")]
mod x86_helpers {
    fn udivmod(n: u64, d: u64) -> (u64, u64) {
        if d == 0 {
            // Division by zero traps, as it does in the CRT helper.
            super::trap()
        }
        let (mut q, mut r) = (0u64, 0u64);
        let mut i = 64;
        while i > 0 {
            i -= 1;
            r = (r << 1) | ((n >> i) & 1);
            if r >= d {
                r -= d;
                q |= 1u64 << i;
            }
        }
        (q, r)
    }

    pub extern "system" fn aulldiv(n_lo: u32, n_hi: u32, d_lo: u32, d_hi: u32) -> u64 {
        udivmod(u64::from(n_hi) << 32 | u64::from(n_lo), u64::from(d_hi) << 32 | u64::from(d_lo)).0
    }

    pub extern "system" fn aullrem(n_lo: u32, n_hi: u32, d_lo: u32, d_hi: u32) -> u64 {
        udivmod(u64::from(n_hi) << 32 | u64::from(n_lo), u64::from(d_hi) << 32 | u64::from(d_lo)).1
    }
}

#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl __aulldiv",
    "__aulldiv:",
    "jmp {div}",
    ".globl __aullrem",
    "__aullrem:",
    "jmp {rem}",
    div = sym x86_helpers::aulldiv,
    rem = sym x86_helpers::aullrem,
);
