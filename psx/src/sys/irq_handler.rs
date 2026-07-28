//! Handler for interrupts
//!
//! Module provides functions to listen to interrupts rather than
//! making acks manually.
//! This is not set up as a static interface as only a single handler exists
//! in the PS1 system.
//!
//! Having a handler system makes sure that as much of the PS1's unstable
//! IRQ handling system is safely controlled
//!
//! Heavily based on [psN00bSdk's interrupt management
//! system](ttps://github.com/Lameguy64/PSn00bSDK/blob/master/libpsn00b/psxetc/
//! interrupts.c)
use crate::{breakpoint,
            hw::{irq::{self, IRQ},
                 mmio::MemRegister,
                 Register},
            sys::{kernel::{psx_change_clear_pad, psx_change_clear_rcnt,
                           psx_enter_critical_section, psx_exit_critical_section,
                           psx_return_from_exception, psx_set_custom_exit_from_exception},
                  setjmp::JumpBuffer}};

const NUM_IRQ_CHANNELS: usize = 11;
static mut HANDLERS: [Option<fn()>; NUM_IRQ_CHANNELS] = [None; _];
static mut IRQ_HANDLER_INSTALLED: bool = false;

static mut SAVED_IRQ_MASK: u16 = 0;

// Jump buffer passed to the BIOS
static mut IRQ_JUMP_BUFFER: JumpBuffer = JumpBuffer::zeroed();

// Pre-allocated stack for the irq handlers - by setting the stack pointer
// to that location, and pre-declaring it to make sure it's not taken by
// anything else, we make sure that that memory area is used for the interrupt
// handlers only.
static mut IRQ_HANDLER_STACK: [u8; 0x1000] = [0; _];

/// Jump buffer structure set up to pass to set_custom_exit_from_exception.
/// Set up as-per [the lord's recommendations](https://psx-spx.consoledev.net/kernelbios/#b19h-hookentryintaddr)
fn get_irq_handler_jmp_buf() -> JumpBuffer {
    unsafe {
        JumpBuffer {
            // > Pointer to "psx_return_from_exception" function
            ra: global_irq_handler as u32,
            // > usually exception stacktop, minus 4, for whatever reason
            sp: ((&raw const IRQ_HANDLER_STACK)
                .cast::<u32>()
                .add(0x1000)
                .addr()) as u32 -
                4,
            ..JumpBuffer::zeroed()
        }
    }
}

fn global_irq_handler() {
    let (mut irq_status, mut irq_mask) = (irq::Status::new(), irq::Mask::new());
    let mut stat = irq_status.as_ref() & irq_mask.as_ref();

    // Loop while some interrupt stats are up
    while stat >= 1 {
        for i in 0..NUM_IRQ_CHANNELS {
            if stat >> i == 0 {
                break;
            }
            if (stat >> i) & 1 == 0 {
                continue;
            }
            irq_status.assign((1 << i) ^ 0xffff).store();
            unsafe {
                if let Some(func) = HANDLERS[i] {
                    func();
                }
            }
        }
        stat = irq_status.load().as_ref() & irq_mask.load().as_ref();
    }
    unsafe { psx_return_from_exception() };
}

/// Set a callback to the interrupt handler for a given interrupt.
/// Only a single callback is supported per interrupt.
/// TODO: Could be easy to set up multiple callbacks via linked list, but no
/// reason why I'd bother
pub(crate) fn set_callback(irq: IRQ, callback: fn()) -> Option<fn()> {
    unsafe {
        let old_cb = HANDLERS[irq as usize].take();
        HANDLERS[irq as usize] = Some(callback);
        irq::Mask::new().set_bits(1 << (irq as u16)).store();
        old_cb
    }
}

/// Remove a callback, if it is there, from the interrupt handler.
pub(crate) fn remove_callback(irq: IRQ) -> Option<fn()> {
    unsafe {
        let old_cb = HANDLERS[irq as usize].take();
        irq::Mask::new().clear_bits(1 << (irq as u16)).store();
        old_cb
    }
}

/// Reset to a new callback system
pub(crate) fn reset_callback() {
    if unsafe { IRQ_HANDLER_INSTALLED } {
        return;
    }

    unsafe {
        psx_enter_critical_section();
        (0..NUM_IRQ_CHANNELS).for_each(|i| {
            HANDLERS[i] = None;
        });

        // Why set bus common delay register?
        // TODO: Understand the point of editing this when lord nocash$ says
        // this is seldom touched by anything other than the BIOS
        MemRegister::<u32, 0x1F80_1020>::skip_load()
            .assign(0x0000_1325)
            .store();
        // psx__96_remove(); // Function doesn't work, so why?
        restart_callbacks(0);
    }
}

/// Restart the user interrupt callback loop
pub(crate) fn restart_callbacks(irq_mask: u16) {
    // Only call function when the IRQ handler has not
    // been installed into the PS1's IRQ handling system
    if unsafe { IRQ_HANDLER_INSTALLED } {
        return;
    }

    // Mark as entering critical section to avoid interrupt flag changes
    // during runtime of the function
    unsafe {
        psx_enter_critical_section();
    }

    // Empty status, set mask to saved value
    irq::Status::skip_load().assign(0).store();
    irq::Mask::skip_load().assign(irq_mask).store();

    // Hook up interrupt handler & clear auto acks
    unsafe {
        IRQ_JUMP_BUFFER = get_irq_handler_jmp_buf();
        psx_set_custom_exit_from_exception((&raw const IRQ_JUMP_BUFFER).cast());

        psx_change_clear_pad(0);
        (0..=3).for_each(|i| {
            psx_change_clear_rcnt(i, false);
        });
        IRQ_HANDLER_INSTALLED = true;

        // Make sure to exit critical section
        psx_exit_critical_section();
    }
}

/// Stop the user interrupt callback loop and save the options
/// for restart later on
pub(crate) fn stop_callback() {
    // Only run function when the handler is set up on the system
    if unsafe { !IRQ_HANDLER_INSTALLED } {
        return;
    }

    let mut irq_mask = irq::Mask::new();

    unsafe {
        psx_enter_critical_section();
        SAVED_IRQ_MASK = *irq_mask.load().as_ref();
        irq_mask.assign(0).store();
        irq::Status::new().assign(0).store();

        // Re-enable autoAck everywhere
        psx_change_clear_pad(1);
        (0..=3).for_each(|i| {
            psx_change_clear_rcnt(i, true);
        });

        IRQ_HANDLER_INSTALLED = false;
    }
}
