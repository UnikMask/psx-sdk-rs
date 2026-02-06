//! Structure to pass to BIOS for setjmp syscall.
//! The setjmp syscall is used to set up callbacks

/// State for jumps, readable by the BIOS for hook entrypoint.
#[repr(C)]
pub(crate) struct JumpBuffer {
    pub ra: u32,
    pub sp: u32,
    pub fp: u32,
    pub s0: u32,
    pub s1: u32,
    pub s2: u32,
    pub s3: u32,
    pub s4: u32,
    pub s5: u32,
    pub s6: u32,
    pub s7: u32,
    pub gp: u32,
}

impl JumpBuffer {
    pub(crate) const fn zeroed() -> Self {
        Self {
            ra: 0,
            sp: 0,
            fp: 0,
            s0: 0,
            s1: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            gp: 0,
        }
    }
}
