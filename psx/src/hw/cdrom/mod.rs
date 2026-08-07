//! Hardware CD-ROM controls
//! Suite of commands, registers, and interrupts
//! for interacting with the CD-ROM.
//!
//! For higher-level CD-ROM usage, check out `crate::cd`
#![allow(missing_docs)]

use crate::hw::{mmio::MemRegister, Register};

/// Control for switching/fetching the current bank
/// that the controller interface is switched to.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankNumber {
    /// Bank no. 0
    Zero,
    /// Bank no. 1
    One,
    /// Bank no. 2
    Two,
    /// Bank no. 3
    Three,
}

impl From<u8> for BankNumber {
    fn from(value: u8) -> Self {
        match value & 0x3 {
            0 => BankNumber::Zero,
            1 => BankNumber::One,
            2 => BankNumber::Two,
            _ => BankNumber::Three,
        }
    }
}

/// The status of an ADDRESS read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressStatus(u8);

impl AddressStatus {
    /// Get the current bank
    pub fn get_current_bank(self) -> BankNumber {
        BankNumber::from(self.0)
    }

    pub fn get_adpbusy(self) -> bool {
        self.0 & 4 != 0
    }

    pub fn get_prmempt(self) -> bool {
        self.0 & 8 != 0
    }

    pub fn get_prmwrdy(self) -> bool {
        self.0 & 16 != 0
    }

    pub fn get_rslrrdy(self) -> bool {
        self.0 & 32 != 0
    }

    pub fn get_drqsts(self) -> bool {
        self.0 & 64 != 0
    }

    pub fn get_busysts(self) -> bool {
        self.0 & 128 != 0
    }
}

/// Get the current bank the CD-ROM interface is on.
pub fn get_bank() -> BankNumber {
    HSTS_ADDRESS::skip_load().read().get_current_bank()
}

/// The memory register address for HSTS writes and ADDRESS reads
pub type HSTS_ADDRESS = MemRegister<u8, 0x1F80_1800>;
impl HSTS_ADDRESS {
    /// Switch host controller interface to a different bank.
    pub fn change_bank(&mut self, bank_no: BankNumber) {
        *self.as_mut() = bank_no as u8;
        self.store();
    }

    /// Read from address to fetch CD-ROM driver status
    ///
    /// # Returns
    /// The address status as a wrapper
    pub fn read(&mut self) -> AddressStatus {
        AddressStatus(*self.load().as_ref())
    }
}

/// Errors returned by the CD-ROM driver
pub enum CdError {
    ErrorGeneric,
}

/// Commands that can be sent to the CD-ROM
#[repr(u8)]
pub enum CdCommand {
    Unused = 0x0,
    /// ack INT3: status
    Nop = 0x1,
    /// Parameters: Minutes, Seconds, Frame
    /// ack INT3: Status
    Setloc = 0x2,
    /// Parameter: Option<Track>
    /// ack INT3: Status
    /// while reading - INT1: status, track, index, min, sec, frame, peakl,
    /// peakh
    Play = 0x3,
    /// ack INT3: Status
    /// while reading - INT1: status, track, index, min, sec, frame, peakl,
    /// peakh Errors if disk spun down
    Forward = 0x4,
    /// ack INT3: Status
    /// while reading - INT1: status, track, index, min, sec, frame, peakl,
    /// peakh Errors if disk spun down
    Backward = 0x5,
    /// ack INT3: Status
    ReadN = 0x6,
    /// ack INT3: Status
    /// completion INT2: Status
    Standby = 0x7,
    /// ack INT3: Status
    /// completion INT2: Status
    Stop = 0x8,
    /// ack INT3: Status
    /// completion INT2: Status
    Pause = 0x9,
    /// ack INT3: Status (late)
    /// completion INT2: Status
    Init = 0xa,
    /// ack INT3: Status
    Mute = 0xb,
    /// ack INT3: Status
    Demute = 0xc,
    /// Parameters: File, Channel
    /// ack INT3: Status
    Setfilter = 0xd,
    /// Parameters: Mode
    /// ack INT3: Status
    Setmode = 0xe,
    /// ack INT3: status, mode, 0x00, file, channel
    Getparam = 0xf,
    /// ack INT3: min, sec, frame, mode, file, channel, sm, ci
    /// Errors if disk spun down
    GetlocL = 0x10,
    /// ack INT3: track, index, rmin, rsec, rframe, min, sec, frame
    /// Errors if disk spun down
    GetlocP = 0x11,
    /// Parameters: Session
    /// ack INT3: status
    /// completion INT2: Status
    Setsession = 0x12,
    /// ack INT3: status, first, last
    GetTN = 0x13,
    /// ack INT3: status, min, sec
    GetTD = 0x14,
    /// ack INT3: status
    /// completion INT2: status
    SeekL = 0x15,
    /// ack INT3: status
    /// completion INT2: status
    SeekP = 0x16,
    /// Parameters: Sub, ...
    /// ack INT3: ...
    Test = 0x19,
    /// ack INT3: Status
    /// Completion INT2/INT5 Status, Flag, Type, Atip, "SCEx"
    GetID = 0x1a,
    /// ack INT3: status
    ReadS = 0x1b,
    /// ack INT3: status
    /// NOTE: Reboots HC05, requires delay after sending (nops?)
    Reset = 0x1c,
    /// Parameters: Adr, point
    /// ack INT3: status
    /// completion INT2: subq[10], peakl
    /// Errors if disk spun down
    /// NOTE: Console version >=`0xc1`
    GetQ = 0x1d,
    /// ack INT3: status (late)
    /// NOTE: Console version >=`0xc1`
    ReadTOC = 0x1e,
    /// NOTE: SCPH-5903 only
    VideoCD = 0x1f,
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock0 = 0x50,
    /// Parameters: "Licensed by"
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock1 = 0x51,
    /// Parameters: "Sony"
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock2 = 0x52,
    /// Parameters: "Computer"
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock3 = 0x53,
    /// Parameters: "Entertainment"
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock4 = 0x54,
    /// Parameters: "<region>"
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock5 = 0x55,
    /// ack INT5: 0x11, 0x40 (even on success)
    /// NOTE: Version >=`0xc1`, does nothing on japanese module
    Unlock6 = 0x56,
    Lock = 0x57,
}

/// Various interrupt kinds possible to listen to with the CD-ROM driver on
/// HINTSTS read.
#[repr(u8)]
pub enum CdIntFlag {
    /// No interrupt
    NoInterrupt = 0,
    /// Data ready
    DataReady = 1,
    /// Command finished processing
    Complete = 2,
    /// Acknowledged
    Acknowledged = 3,
    /// Data end
    DataEnd = 4,
    /// Disk error
    DiskError = 5,
}

impl From<u8> for CdIntFlag {
    fn from(value: u8) -> Self {
        match value & 0b111 {
            0 => Self::NoInterrupt,
            1 => Self::DataReady,
            2 => Self::Complete,
            3 => Self::Acknowledged,
            4 => Self::DataEnd,
            5 => Self::DiskError,
            _ => panic!("INT flag not supported by machine!"),
        }
    }
}

/// A read from a memory register for HINTSTS
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HintStsRead(u8);

impl HintStsRead {
    pub fn get_flag(self) -> CdIntFlag {
        self.0.into()
    }
}

/// Fetch the latest results from the CD-ROM driver.
pub fn fetch_cd_result(reg: MemRegister<u8, 0x1F80_1801>) -> Option<Result<(), CdError>> {
    None
}

/// Module for reads/writes on bank 0
mod cd_bank0 {
    use crate::hw::mmio::MemRegister;

    /// Memory register where command, wrdata, CI, and ATV2 writes are
    /// performed, and results are read.
    pub type COMMAND_RSLT = MemRegister<u8, 0x1F80_1801>;

    /// Register to write parameters to. Stores upto 16 bytes.
    /// Reads RDDATA in read-mode.
    pub type PARAMETER_RDDATA = MemRegister<u8, 0x1F80_1802>;

    /// Register to write for SMEN sound maps, and requesting read/write to
    /// buffer. Reads HINTMSK in read-mode.
    pub type HCHPCTL_HINTMSK = MemRegister<u8, 0x1F80_1803>;
}

/// Module for reads/writes on bank 1
mod cd_bank1 {
    use crate::hw::mmio::MemRegister;

    /// performed, and results are read.
    pub type WRDATA_RSLT = MemRegister<u8, 0x1F80_1801>;

    /// Reads RDDATA in read-mode.
    pub type HINTMSK_RDDATA = MemRegister<u8, 0x1F80_1802>;

    /// Reads HINTSTS in read-mode.
    pub type HCLRCTL_HINTSTS = MemRegister<u8, 0x1F80_1803>;
}

/// Module for reads/writes on bank 2
mod cd_bank2 {
    use crate::hw::mmio::MemRegister;

    /// performed, and results are read.
    pub type CI_RSLT = MemRegister<u8, 0x1F80_1801>;

    /// Reads RDDATA in read-mode.
    pub type ATV0_RDDATA = MemRegister<u8, 0x1F80_1802>;

    /// Reads HINTMSK in read-mode.
    pub type ATV1_HINTMSK = MemRegister<u8, 0x1F80_1803>;
}

/// Module for reads/writes on bank 3
mod cd_bank3 {
    use crate::hw::mmio::MemRegister;

    /// performed, and results are read.
    pub type ATV2_RSLT = MemRegister<u8, 0x1F80_1801>;

    /// Reads RDDATA in read-mode.
    pub type ATV3_RDDATA = MemRegister<u8, 0x1F80_1802>;

    /// Reads HINTSTS in read-mode.
    pub type ADPCTL_HINTSTS = MemRegister<u8, 0x1F80_1803>;
}
