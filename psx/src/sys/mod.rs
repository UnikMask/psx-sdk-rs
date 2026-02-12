//! BIOS function wrappers.
//!
//! This module contains wrappers for functions provided by the BIOS.

use crate::{hw::{irq, Register},
            sys::kernel::{psx_enter_critical_section, psx_exit_critical_section},
            CriticalSection};
use core::ffi::CStr;

pub mod fs;
pub mod gamepad;
pub mod heap;
pub mod irq_handler;
pub mod kernel;
pub mod rng;
pub mod setjmp;
pub mod tty;

/// Calls the given function in an interrupt-free critical section using BIOS
/// syscalls.
///
/// # Safety
///
/// Exception handlers might not support nested exceptions so make sure to not
/// call this from a critical section.
pub unsafe fn critical_section<F: FnMut(&mut CriticalSection) -> R, R>(mut f: F) -> R {
    let changed_state = unsafe { kernel::psx_enter_critical_section() };
    // SAFETY: We are in a critical section so we can create this
    let mut cs = unsafe { CriticalSection::new() };
    let res = f(&mut cs);
    if changed_state {
        unsafe {
            kernel::psx_exit_critical_section();
        }
    };
    res
}

/// Returns the kernel's version string.
pub fn get_system_version() -> &'static CStr {
    // SAFETY: Calling get_system_info with index 2 gives a pointer with a
    // static lifetime to the version string. There are no safety requirement.
    let version = unsafe { kernel::psx_get_system_info(2) as *const i8 };
    // SAFETY: Let's hope the BIOS returned a pointer to a null-terminated string
    // to its own memory.
    unsafe { CStr::from_ptr(version) }
}

/// Returns the kernel's date in BCD (e.g. 0x19951204).
pub fn get_system_date() -> u32 {
    unsafe { kernel::psx_get_system_info(0) }
}

/// Handle for entering and exiting a critical section (all interrupts
/// disabled), using direct hardware-register based interrupt masking.
pub struct FastCriticalSection {
    saved_mask: u16,
}

/// Handle for entering and exiting a critical section (all interrupts
/// disabled), using BIOS commands.
pub struct BiosCriticalSection {
    return_from_critical_section: bool,
}

impl FastCriticalSection {
    /// Enter a new fast critical section
    pub fn new() -> Self {
        let mut irq_mask = irq::Mask::new();
        let saved_mask = *irq_mask.as_ref();
        irq_mask.assign(0).store();
        Self { saved_mask }
    }

    /// Run code within a fast critical section
    pub fn with<'a, T>(func: impl FnOnce() -> T + 'a) -> T {
        let mut irq_mask = irq::Mask::new();
        let saved_mask = *irq_mask.as_ref();
        irq_mask.assign(0).store();
        let res = func();
        irq_mask.assign(saved_mask).store();
        res
    }
}

impl Drop for FastCriticalSection {
    fn drop(&mut self) {
        irq::Mask::skip_load().assign(self.saved_mask).store();
    }
}

impl BiosCriticalSection {
    /// Enter a new BIOS critical section
    pub fn new() -> Self {
        Self {
            return_from_critical_section: unsafe { psx_enter_critical_section() },
        }
    }

    /// Run code within a fast critical section
    pub fn with<'a, T>(func: impl FnOnce() -> T + 'a) -> T {
        let ret = unsafe { psx_enter_critical_section() };
        let res = func();
        if ret {
            unsafe {
                psx_exit_critical_section();
            }
        }
        res
    }
}

impl Drop for BiosCriticalSection {
    fn drop(&mut self) {
        if self.return_from_critical_section {
            unsafe { psx_exit_critical_section() };
        }
    }
}
