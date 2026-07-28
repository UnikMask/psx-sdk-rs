use crate::gpu::colors::WHITE;
use crate::gpu::primitives::Sprt8;
use crate::gpu::{Clut, Color, DMAMode, Depth, DispEnv, DrawEnv, Packet, TexColor, TexCoord,
                 TexPage, Vertex, VertexError, VideoMode, GPU_BUFFER_SIZE};
use crate::hw::{gpu::{self, GP0Command, GP0, GP1},
                irq::IRQ,
                Register};
use crate::sys::irq_handler;
use crate::sys::kernel::{psx_enter_critical_section, psx_exit_critical_section};
use crate::{breakpoint, include_tim, println};
use crate::{dma, format::tim::TIM};
use core::fmt;
use core::mem::size_of;

fn draw_sync() {
    let mut gpu_stat = gpu::Status::new();
    while !gpu_stat.cmd_ready() || !gpu_stat.dma_ready() {
        gpu_stat.load();
    }
}

static mut VBLANK_COUNTER: usize = 0;

/// A double-buffered framebuffer configuration
///
/// Maintains the framebuffer's configuration and state. Also provides acess to
/// the GPU registers `GP0`, `GP1` and `GPU_STATUS`.
pub struct Framebuffer {
    /// The write-only GPU I/O port for GP0 commands and packets
    pub gp0: GP0,
    /// The write-only GPU I/O port for GP1 commands
    pub gp1: GP1,
    /// The read-only GPU status register
    pub gpu_status: gpu::Status,
    disp_envs: [DispEnv; 2],
    draw_envs: [Packet<DrawEnv>; 2],
    swapped: bool,
}

impl Default for Framebuffer {
    fn default() -> Self {
        // SAFETY: The framebuffer parameters are valid.
        unsafe { Self::new((0, 0), (0, 240), (320, 240), VideoMode::NTSC, None).unwrap_unchecked() }
    }
}

/// Callback for the vblank counter observed by the framebuffer.
fn framebuffer_vblank_callback() {
    unsafe {
        VBLANK_COUNTER = VBLANK_COUNTER.wrapping_add(1);
    }
}

impl Framebuffer {
    /// Creates a new framebuffer.
    ///
    /// Places one buffer at `buf0` and the other at `buf1` and uses the
    /// specified resolution and background color (or black if `bg_color` is
    /// `None`). Also resets the GPU, enables DMA to GP0 on the GPU-side and
    /// enables the display.
    pub fn new(
        buf0: (i16, i16), buf1: (i16, i16), res: (i16, i16), video_mode: VideoMode,
        bg_color: Option<Color>,
    ) -> Result<Self, VertexError> {
        let exit = unsafe { psx_enter_critical_section() };

        // Set up callback system if not enabled yet
        irq_handler::reset_callback();
        irq_handler::set_callback(IRQ::Vblank, framebuffer_vblank_callback);

        if exit {
            unsafe { psx_exit_critical_section() }
        };

        let mut fb = Framebuffer {
            // These registers are read-only
            gp0: GP0::skip_load(),
            gp1: GP1::skip_load(),
            gpu_status: gpu::Status::new(),
            disp_envs: [
                DispEnv::new(buf0, res, video_mode)?,
                DispEnv::new(buf1, res, video_mode)?,
            ],
            draw_envs: [
                Packet::new(DrawEnv::new(buf1, res, bg_color)?),
                Packet::new(DrawEnv::new(buf0, res, bg_color)?),
            ],
            swapped: false,
        };
        let interlace = matches!(res.1, 480 | 512);

        GP1::skip_load()
            .reset_gpu()
            .dma_mode(Some(DMAMode::GP0))
            .display_mode(res, video_mode, Depth::Bits15, interlace)?
            .enable_display(true);

        fb.draw_sync();
        fb.wait_vblank();
        fb.swap();
        Ok(fb)
    }

    /// Changes the framebuffer's background color.
    pub fn set_bg_color(&mut self, color: Color) {
        for packet_env in &mut self.draw_envs {
            packet_env.contents.bg_color = color;
        }
    }

    /// Swaps the framebuffers using only GPU I/O ports.
    pub fn swap(&mut self) {
        self.swapped = !self.swapped;
        let idx = self.swapped as usize;
        self.gp1.set_display_env(&self.disp_envs[idx]);
        self.gp0.send_command(&self.draw_envs[idx].contents);
    }

    /// Swaps the framebuffers using GPU I/O ports and the DMA channel
    pub fn dma_swap(&mut self, gpu_dma: &mut dma::GPU) {
        self.swapped = !self.swapped;
        let idx = self.swapped as usize;
        self.gp1.set_display_env(&self.disp_envs[idx]);
        gpu_dma.send_list(&self.draw_envs[idx]);
    }

    /// Loads a `TIM` file into VRAM.
    ///
    /// After loading a TIM into VRAM, the copy in memory isn't necessary so the
    /// lifetimes of the `TIM` and `LoadedTIM` are completely disconnected.
    pub fn load_tim<const N: usize, const M: usize>(&mut self, tim: TIM<N, M>) -> LoadedTIM {
        // Used to avoid implementing GP0Command for any &[u32]
        // TIM::new ensures that the bitmap data is a valid GP0 command
        struct CopyToVRAM<'a>(&'a [u32]);

        impl GP0Command for CopyToVRAM<'_> {
            fn data(&self) -> &[u32] {
                self.0
            }
        }

        self.draw_sync();
        self.gp0.send_command(&CopyToVRAM(&tim.bmp.data));
        let clut = if M != 0 {
            self.draw_sync();
            self.gp0.send_command(&CopyToVRAM(&tim.clut.data));
            Some(tim.clut.offset)
        } else {
            None
        };

        LoadedTIM {
            tex_page: tim.bmp.offset,
            clut,
        }
    }

    /// Loads the default font TIM into VRAM.
    ///
    /// This returns a `LoadedTIM` which can then be used to create `TextBox`s
    /// using `LoadedTIM::new_text_box`. Note that `LoadedTIM` does not track
    /// lifetimes so it's the user's responsibility to ensure that the font
    /// remains in VRAM while it's needed.
    pub fn load_default_font(&mut self) -> LoadedTIM {
        let font = include_tim!("../font.tim");
        self.load_tim(font)
    }

    /// Spins until the GPU is ready to draw.
    pub fn draw_sync(&mut self) {
        self.gpu_status.load();
        while !self.gpu_status.cmd_ready() || !self.gpu_status.dma_ready() {
            self.gpu_status.load();
        }
    }

    /// Spins until vblank.
    ///
    /// # Returns
    /// Whether the vblank wait was successful or not, according to arbitrary
    /// timeout. If `wait_vblank` returns false, it is recommended to log it.
    pub fn wait_vblank(&mut self) -> bool {
        const VSYNC_TIMEOUT: usize = 0x100000;
        unsafe {
            let vblank_target = VBLANK_COUNTER;
            breakpoint!(0x2);
            for _ in 0..VSYNC_TIMEOUT {
                if (&raw const VBLANK_COUNTER).read_volatile() != vblank_target {
                    breakpoint!(0x33);
                    return true;
                }
            }
            breakpoint!(0x34);
            false
        }
    }
}

impl<T: WriteMode> fmt::Write for TextBox<T> {
    fn write_str(&mut self, msg: &str) -> fmt::Result {
        // TODO: This may be unnecessary
        draw_sync();
        for c in msg.chars() {
            if c.is_ascii() {
                self.print_char(c as u8);
            } else {
                // Print '?' for non-ascii UTF-8
                self.print_char(b'?');
            }
        }
        Ok(())
    }
}

/// The properties of a TIM file that has been loaded into VRAM.
///
/// This does not track lifetimes, so it's the user's responsibility to ensure
/// that the TIM remains in VRAM while it's needed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoadedTIM {
    /// The loaded TIM's texture page attribute.
    pub tex_page: TexPage,
    /// The loaded TIM's color loookup table attribute.
    pub clut: Option<Clut>,
}

// Up to 5 `Sprt8`s fit in the GPU buffer at one time.
const TEXT_BOX_BUFFER: usize = GPU_BUFFER_SIZE / size_of::<Sprt8>();

/// A text box configuration and in-memory buffer.
pub struct TextBox<T: WriteMode> {
    color: TexColor,
    initial: Vertex,
    cursor: Vertex,
    // Dynamic vertex for overall text box size
    size: Vertex,
    data: T,
}

/// Trait describing a method with which a text box can write to be later
/// displayed.
pub trait WriteMode {
    /// Write a character to the structure's screen representation and increment
    /// it's internal index. Depending on the implementation, can either
    /// directly display the char on the screen or keep it for later.
    fn write_char(&mut self, sprt: Sprt8);
    /// Reset the internal buffer to rewrite anew
    fn reset(&mut self);
}

/// Text box configuration for direct mode - the text box calls GPU commands
/// to GP0 directly via I/O and for each character.
pub struct DirectMode {
    idx: usize,
    buffer: [Sprt8; TEXT_BOX_BUFFER],
}

impl WriteMode for DirectMode {
    fn write_char(&mut self, sprt: Sprt8) {
        self.buffer[self.idx] = sprt;
        GP0::skip_load().send_command(&self.buffer[self.idx]);
        self.idx = (self.idx + 1) % TEXT_BOX_BUFFER;
    }

    fn reset(&mut self) {
        self.idx = 0;
    }
}

/// Text box configuration for direct mode - the text box gets called via DMA
/// instead of GPU I/O, and has the call has to be done by the user who should
/// link the the text box to a packet in a linked list.
pub struct IndirectMode<const MEM_SIZE: usize> {
    /// Starting index of the textbox after each reset
    reset_index: usize,
    /// Index and cursor for text box at current point
    current_index: usize,
    /// Buffer is made twice its regular size to accomodate for frame swapping,
    /// and being able to write to the buffer as other parts are being printed.
    ///
    /// Buffer acts as a ring buffer - on each reset, the cursor for text
    /// positioning is reset, but the reset index is updated to be the next
    /// position the textbox should write to. This is to guarantee frameswapping
    /// resilience.
    buffer: [Packet<Sprt8>; MEM_SIZE],
}

impl<const MEM_SIZE: usize> WriteMode for IndirectMode<MEM_SIZE> {
    fn write_char(&mut self, sprt: Sprt8) {
        // Set char at current index, then if not starting letter, link letter
        // to previous letter in the linked list.
        self.buffer[self.current_index] = Packet::new(sprt);
        if self.current_index != self.reset_index {
            let [prev, cur] = &mut self.buffer[self.current_index - 1..=self.current_index] else {
                unreachable!("Prior check done for this!");
            };
            prev.insert_packet(cur);
        }

        // Increment index, such that the index loops back if it goes past
        // set width and height.
        self.current_index =
            ((self.current_index - self.reset_index + 1) % (MEM_SIZE / 2)) + self.reset_index;
    }

    fn reset(&mut self) {
        // Increment reset index by half what it was, so it always essentially
        // goes as 0 or WIDTH * HEIGHT.
        //
        // Then reset the first element of the buffer to break the previous linked list.
        self.reset_index = if self.reset_index == 0 {
            MEM_SIZE / 2
        } else {
            0
        };
        self.current_index = self.reset_index;
        self.buffer[self.current_index] = Packet::new(Sprt8::new());
    }
}

impl TextBox<DirectMode> {
    /// Create a text box from a TIM loaded into memory, with a given offset and
    /// size.
    pub fn from_loaded_tim(tim: &LoadedTIM, offset: (i16, i16), size: (i16, i16)) -> Self {
        let offset = Vertex::new(offset);
        let size = Vertex::new(size);
        let color = TexColor::from(WHITE);

        Self {
            color,
            initial: offset,
            cursor: offset,
            size,
            data: DirectMode::from(tim),
        }
    }
}

impl From<&LoadedTIM> for DirectMode {
    fn from(tim: &LoadedTIM) -> Self {
        let mut buffer = [Sprt8::new(); TEXT_BOX_BUFFER];
        let color = TexColor::from(WHITE);
        for letter in &mut buffer {
            if let Some(clut) = tim.clut {
                letter.set_clut(clut);
            }
            letter.set_color(color);
        }
        Self { idx: 0, buffer }
    }
}

impl<const MEM_SIZE: usize> From<&LoadedTIM> for IndirectMode<MEM_SIZE> {
    fn from(tim: &LoadedTIM) -> Self {
        breakpoint!(0x11); // Breakpoint from(tim)
        let mut buffer = [const { Packet::new(Sprt8::new()) }; MEM_SIZE];
        breakpoint!((&raw const buffer).addr() as u32); // Post-buffer breakpoint
        let color = TexColor::from(WHITE);
        for packet in &mut buffer {
            if let Some(clut) = tim.clut {
                packet.contents.set_clut(clut);
            }
            packet.contents.set_color(color);
        }
        Self {
            reset_index: 0,
            current_index: 0,
            buffer,
        }
    }
}

impl<const MEM_SIZE: usize> TextBox<IndirectMode<MEM_SIZE>> {
    /// Create a text box from a TIM loaded into memory, with a given offset and
    /// const-time deduced size.
    pub fn from_loaded_tim(tim: &LoadedTIM, offset: (i16, i16), size: (i16, i16)) -> Self {
        let offset = Vertex::new(offset);
        let color = TexColor::from(WHITE);

        Self {
            color,
            initial: offset,
            cursor: offset,
            clut: tim.clut,
            size: Vertex::new(size),
            data: IndirectMode::<MEM_SIZE>::from(tim),
        }
    }

    /// Get the first and last element of the current text box buffer
    /// to set them up for display.
    ///
    /// # Returns
    ///
    /// The first and last char's packet mutable reference if there is multiple
    /// chars, If there is only a single char, return only that packet.
    /// Return `None` if there is no element available.
    pub fn get_linked_list(&mut self) -> Option<(&mut Packet<Sprt8>, Option<&mut Packet<Sprt8>>)> {
        match self.data.current_index - self.data.reset_index {
            0 => None,
            1 => Some((&mut self.data.buffer[self.data.reset_index], None)),
            _ => {
                let (first, last) = self.data.buffer.split_at_mut(self.data.current_index - 1);
                Some((&mut first[self.data.reset_index], Some(&mut last[0])))
            },
        }
    }
}

const FONT_SIZE: u8 = 8;

impl<T: WriteMode> TextBox<T> {
    /// Moves the cursor to the beginning of the next line.
    pub fn newline(&mut self) {
        self.cursor = Vertex(self.initial.0, self.cursor.1 + FONT_SIZE as i16);
    }
    /// Moves the cursor to its initial position.
    pub fn reset(&mut self) {
        self.cursor = self.initial;
        self.data.reset();
    }
    /// Moves the cursor up n characters.
    pub fn move_up(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor.1 -= FONT_SIZE as i16;
        }
    }
    /// Moves the cursor down n characters.
    pub fn move_down(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor.1 += FONT_SIZE as i16;
        }
    }
    /// Moves the cursor left n characters.
    pub fn move_left(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor.0 -= FONT_SIZE as i16;
        }
    }
    /// Moves the cursor right n characters.
    pub fn move_right(&mut self, n: usize) {
        for _ in 0..n {
            self.cursor.0 += FONT_SIZE as i16;
        }
    }
    /// Change the font color.
    pub fn change_color(&mut self, color: Color) {
        let color = TexColor::from(color);
        if color != self.color {
            self.color = color;
        }
    }

    /// Prints a single character.
    pub fn print_char(&mut self, ascii: u8) {
        if ascii == b'\n' {
            self.newline();
            self.cursor.0 = self.initial.0;
        } else {
            let ascii_per_row = 128 / FONT_SIZE;
            // The default font omits the first 32 characters to save on VRAM. These
            // characters are printed as '?'
            let ascii = if ascii < (2 * ascii_per_row) {
                b'?'
            } else {
                ascii - (2 * ascii_per_row)
            };
            let x = (ascii % ascii_per_row) * FONT_SIZE;
            let y = (ascii / ascii_per_row) * FONT_SIZE;
            let mut sprt = Sprt8::new();
            sprt.set_offset(self.cursor)
                .set_tex_coord(TexCoord { x, y })
                .set_color(self.color);
            self.data.write_char(sprt);

            self.cursor.0 += FONT_SIZE as i16;
            if self.cursor.0 == self.initial.0 + self.size.0 {
                self.newline();
                self.cursor.0 = self.initial.0;
            }
            if self.cursor.1 == self.initial.1 + self.size.1 {
                self.cursor = self.initial;
            }
        }
    }
}

/// Print a rust-style format string and args using the `&mut TextBox` specified
/// by `$box`.
#[macro_export]
macro_rules! dprint {
    ($box:expr, $($args:tt)*) => {
        {
            use core::fmt::Write;
            $box.write_fmt(format_args!($($args)*)).ok()
        }
    };
}

/// Print a rust-style format string and args using the `&mut TextBox` specified
/// by `$box`.
#[macro_export]
macro_rules! dprintln {
    ($box:expr, $($args:tt)*) => {
        $crate::dprint!($box, $($args)*);
        $box.print_char(b'\n');
    };
}
