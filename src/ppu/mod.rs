use super::memory::MemSegment;
use cart::Cart;
use std::rc::Rc;
use std::cell::RefCell;
use screen::Screen;

#[cfg(test)]
mod tests;

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 240;
pub const SCREEN_BUFFER_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

const NAMETABLE_WIDTH: usize = 32;


#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(C)]
pub struct Color(u8);
impl Color {
    fn from_bits_truncate(val: u8) -> Color {
        Color(val & 0b0011_1111)
    }

    pub fn bits(&self) -> u8 {
        self.0
    }
}

///Represents the PPU's memory map.
struct PPUMemory {
    cart: Rc<RefCell<Cart>>,
    vram: [u8; 0x0800],
    palette: [Color; 0x20],
}

impl PPUMemory {
    fn new(cart: Rc<RefCell<Cart>>) -> PPUMemory {
        PPUMemory {
            cart: cart,
            vram: [0u8; 0x0800],
            palette: [Color::from_bits_truncate(0); 0x20],
        }
    }
}

impl MemSegment for PPUMemory {
    fn read(&mut self, idx: u16) -> u8 {
        match idx {
            0x0000...0x1FFF => {
                let cart = self.cart.borrow_mut();
                cart.chr_read(idx)
            }
            0x2000...0x3EFF => self.vram[(idx % 0x800) as usize],
            0x3F00...0x3FFF => {
                match (idx & 0x001F) as usize {
                    0x10 => self.palette[0x00],
                    0x14 => self.palette[0x04],
                    0x18 => self.palette[0x08],
                    0x1C => self.palette[0x0C],
                    x => self.palette[x],
                }
                .bits()
            }
            x => invalid_address!(x),
        }
    }

    fn write(&mut self, idx: u16, val: u8) {
        match idx {
            0x0000...0x1FFF => {
                let mut cart = self.cart.borrow_mut();
                cart.chr_write(idx, val)
            }
            0x2000...0x3EFF => {
                let idx = ((idx - 0x2000) % 0x800) as usize;
                self.vram[idx] = val;
            }
            0x3F00...0x3FFF => {
                let val = Color::from_bits_truncate(val);
                match (idx - 0x3F00) as usize {
                    0x10 => self.palette[0x00] = val,
                    0x14 => self.palette[0x04] = val,
                    0x18 => self.palette[0x08] = val,
                    0x1C => self.palette[0x0C] = val,
                    x => self.palette[x] = val,
                }
            }
            x => invalid_address!(x),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum AddrByte {
    High,
    Low,
}

struct PPUCtrl {
    bits: u8,
}
impl PPUCtrl {
    fn empty() -> PPUCtrl {
        PPUCtrl { bits: 0 }
    }

    fn new(bits: u8) -> PPUCtrl {
        PPUCtrl { bits: bits }
    }

    fn nametable_addr(&self) -> u16 {
        (self.bits & 0b0000_0011) as u16 * 0x0400 | 0x2000
    }

    fn vram_addr_step(&self) -> u16 {
        if self.bits & 0b0000_0100 != 0 {
            32
        } else {
            1
        }
    }

    fn background_table(&self) -> u16 {
        if self.bits & 0b0001_0000 != 0 {
            0x1000
        } else {
            0x0000
        }
    }

    fn generate_vblank_nmi(&self) -> bool {
        self.bits & 0b1000_0000 != 0
    }
}

bitflags! {
    flags PPUMask : u8 {
        const GREY =    0b0000_0001, //Greyscale
        const S_BCK_L = 0b0000_0010, //Show background in the leftmost 8 pixels
        const S_SPR_L = 0b0000_0100, //Show sprites in the leftmost 8 pixels
        const S_BCK =   0b0000_1000, //Show background
        const S_SPR =   0b0001_0000, //Show sprites
        const EM_R =    0b0010_0000, //Emphasize Red
        const EM_G =    0b0100_0000, //Emphasize Green
        const EM_B =    0b1000_0000, //Emphasize Blue
    }
}

bitflags! {
    flags PPUStat : u8 {
        const VBLANK =          0b1000_0000, //Currently in the vertical blank interval
        const SPRITE_0 =        0b0100_0000, //Sprite 0 hit
        const SPRITE_OVERFLOW = 0b0010_0000, //Greater than 8 sprites on current scanline
    }
}

struct PPUReg {
    ppuctrl: PPUCtrl,
    ppumask: PPUMask,
    ppustat: PPUStat,
    oamaddr: u8,
    ppuscroll: u16,
    ppuaddr: u16,

    ///A fake dynamic latch representing the capacitance of the wires in the
    ///PPU that we have to emulate.
    dyn_latch: u8,

    ///The address registers are two bytes but we can only write one at a time.
    address_latch: AddrByte,
}

impl PPUReg {
    fn scroll_x(&self) -> u8 {
        ((self.ppuscroll & 0xFF00) > 8) as u8
    }

    fn scroll_y(&self) -> u8 {
        ((self.ppuscroll & 0x00FF) > 0) as u8
    }
}

bitflags! {
    flags OAMAttr : u8 {
        const FLIP_VERT = 0b1000_0000,
        const FLIP_HORZ = 0b0100_0000,
        const BEHIND    = 0b0010_0000,
        const PALETTE1  = 0b0000_0010,
        const PALETTE2  = 0b0000_0001,
    }
}

#[derive(Debug, Copy, Clone)]
struct OAMEntry {
    y: u8,
    tile: u8,
    attr: OAMAttr,
    x: u8,
}

impl OAMEntry {
    fn zero() -> OAMEntry {
        OAMEntry::new(0, 0, 0, 0)
    }

    fn new(y: u8, tile: u8, attr: u8, x: u8) -> OAMEntry {
        OAMEntry {
            y: y,
            tile: tile,
            attr: OAMAttr::from_bits_truncate(attr),
            x: x,
        }
    }
}

impl MemSegment for OAMEntry {
    fn read(&mut self, idx: u16) -> u8 {
        match idx % 4 {
            0 => self.y,
            1 => self.tile,
            2 => self.attr.bits(),
            3 => self.x,
            _ => panic!("Math is broken!"),
        }
    }

    fn write(&mut self, idx: u16, val: u8) {
        match idx % 4 {
            0 => self.y = val,
            1 => self.tile = val,
            2 => self.attr = OAMAttr::from_bits_truncate(val),
            3 => self.x = val,
            _ => panic!("Math is broken!"),
        }
    }
}

pub struct PPU {
    reg: PPUReg,
    oam: [OAMEntry; 64],
    ppu_mem: PPUMemory,

    screen: Box<Screen>,
    screen_buffer: [Color; SCREEN_BUFFER_SIZE],

    global_cyc: u64,
    cyc: u16,
    sl: i16,
    frame: u32,
}

#[derive(Copy, Debug, PartialEq, Clone)]
pub enum StepResult {
    NMI,
    Continue,
}

impl PPU {
    pub fn new(cart: Rc<RefCell<Cart>>, screen: Box<Screen>) -> PPU {
        PPU {
            reg: PPUReg {
                ppuctrl: PPUCtrl::empty(),
                ppumask: PPUMask::empty(),
                ppustat: PPUStat::empty(),
                oamaddr: 0,
                ppuscroll: 0,
                ppuaddr: 0,
                dyn_latch: 0,
                address_latch: AddrByte::High,
            },
            oam: [OAMEntry::zero(); 64],
            ppu_mem: PPUMemory::new(cart),
            screen_buffer: [Color::from_bits_truncate(0x00); SCREEN_BUFFER_SIZE],
            screen: screen,

            global_cyc: 0,
            cyc: 0,
            sl: 241,
            frame: 0,
        }
    }

    fn incr_ppuaddr(&mut self) {
        let incr_size = self.reg.ppuctrl.vram_addr_step();
        self.reg.ppuaddr = self.reg.ppuaddr.wrapping_add(incr_size);
    }

    pub fn run_to(&mut self, cpu_cycle: u64) -> StepResult {
        let mut hit_nmi = false;
        while self.global_cyc < (cpu_cycle * 3) {
            self.tick_cycle();
            hit_nmi |= self.run_cycle();
        }

        if hit_nmi {
            StepResult::NMI
        } else {
            StepResult::Continue
        }
    }

    fn tick_cycle(&mut self) {
        self.global_cyc += 1;
        self.cyc += 1;
        if self.cyc == 341 {
            self.cyc = 0;
            self.sl += 1;
            if self.sl == 261 {
                self.sl = -1;
                self.frame += 1;
            }
        }
    }

    fn run_cycle(&mut self) -> bool {
        match (self.cyc, self.sl) {
            (c, -1) => self.prerender_scanline(c),
            (c, sl @ 0...239) => self.visible_scanline(c, sl),
            (_, 240) => (), //Post-render idle scanline
            (1, 241) => return self.start_vblank(),
            (_, 241...260) => (), //VBlank lines
            _ => (),
        }
        false
    }

    fn prerender_scanline(&mut self, _: u16) {
        // Nothing here yet
    }

    fn visible_scanline(&mut self, pixel: u16, scanline: i16) {
        // Nothing here yet
        if pixel >= 256 {
            return;
        }
        let x = pixel as usize;
        let y = scanline as usize;
        self.screen_buffer[y * SCREEN_WIDTH + x] = self.get_pixel(x as u16, y as u16);
    }

    fn get_pixel(&mut self, x: u16, y: u16) -> Color {
        self.get_background_pixel(x, y)
    }

    fn get_background_pixel(&mut self, screen_x: u16, screen_y: u16) -> Color {
        let x = screen_x + self.reg.scroll_x() as u16;
        let y = screen_y + self.reg.scroll_y() as u16;

        let color_id = self.get_color_id(x, y);
        let palette_id = self.get_palette_id(x, y);

        self.read_palette(palette_id, color_id)
    }

    fn get_color_id(&mut self, x: u16, y: u16) -> u8 {
        let nametable_addr = self.get_nametable_addr(x, y);
        let tile_idx = self.ppu_mem.read(nametable_addr);

        let tile_table = self.reg.ppuctrl.background_table();
        let pattern = self.read_tile_pattern(tile_idx, y & 0x07, tile_table);

        self.get_color_in_pattern(pattern, x as u32 & 0x07)
    }

    fn get_nametable_addr(&self, px_x: u16, px_y: u16) -> u16 {
        let x = px_x / 8;
        let y = px_y / 8;
        let result = self.reg.ppuctrl.nametable_addr() + y * NAMETABLE_WIDTH as u16 + x;
        result
    }

    fn read_tile_pattern(&mut self, tile_id: u8, fine_y_scroll: u16, tile_table: u16) -> (u8, u8) {
        let lo_addr = self.get_tile_addr(tile_id, 0, fine_y_scroll, tile_table);
        let hi_addr = self.get_tile_addr(tile_id, 8, fine_y_scroll, tile_table);
        (self.ppu_mem.read(lo_addr), self.ppu_mem.read(hi_addr))
    }

    fn get_tile_addr(&self, tile_id: u8, plane: u8, fine_y_scroll: u16, tile_table: u16) -> u16 {
        let mut tile_addr = 0u16;
        tile_addr |= fine_y_scroll;
        tile_addr |= plane as u16; //Plane must be 0 for low or 8 for high
        tile_addr |= (tile_id as u16) << 4;
        tile_addr |= tile_table; //Table must be 0x0000 or 0x1000
        tile_addr
    }

    fn get_color_in_pattern(&self, pattern: (u8, u8), fine_x: u32) -> u8 {
        let (lo, hi) = pattern;
        let shift = 0x07 - fine_x;
        let color_id_lo = lo.wrapping_shr(shift) & 0x01;
        let color_id_hi = (hi.wrapping_shr(shift) & 0x01) << 1;
        color_id_lo | color_id_hi
    }

    fn get_palette_id(&mut self, x: u16, y: u16) -> u8 {
        let attribute_addr = self.get_attribute_addr(x, y);
        let attribute_byte = self.ppu_mem.read(attribute_addr);
        self.get_palette_from_attribute(attribute_byte, x, y)
    }

    fn get_attribute_addr(&self, x: u16, y: u16) -> u16 {
        let x = x / 32;
        let y = y / 32;
        let attr_table = self.reg.ppuctrl.nametable_addr() + 0x03C0;
        attr_table + (y * 8) + x
    }

    fn get_palette_from_attribute(&self, attr: u8, x: u16, y: u16) -> u8 {
        let mut at = attr;
        if y & 0x10 != 0 {
            at >>= 4
        }
        if x & 0x10 != 0 {
            at >>= 2
        }
        at & 0x03
    }

    fn read_palette(&mut self, palette_id: u8, color_id: u8) -> Color {
        let offset = (palette_id << 2) | color_id;
        let bits = self.ppu_mem.read(0x3F00 + offset as u16);
        Color::from_bits_truncate(bits)
    }

    fn start_vblank(&mut self) -> bool {
        let buf = &self.screen_buffer;
        self.screen.draw(buf);
        if self.frame > 0 {
            self.reg.ppustat.insert(VBLANK);
            self.reg.ppuctrl.generate_vblank_nmi()
        } else {
            false
        }
    }

    #[cfg(feature="cputrace")]
    pub fn cycle(&self) -> u16 {
        self.cyc
    }

    #[cfg(feature="cputrace")]
    pub fn scanline(&self) -> i16 {
        self.sl
    }
}

fn write_addr_byte(latch: &mut AddrByte, target: &mut u16, val: u8) {
    match *latch {
        AddrByte::High => {
            *target = (*target & 0x00FF) | ((val as u16) << 8);
            *latch = AddrByte::Low;
        }
        AddrByte::Low => {
            *target = (*target & 0xFF00) | ((val as u16) << 0);
            *latch = AddrByte::High;
        }
    }
}

impl MemSegment for PPU {
    fn read(&mut self, idx: u16) -> u8 {
        match idx % 8 {
            0x0000 => self.reg.dyn_latch,
            0x0001 => self.reg.dyn_latch,
            0x0002 => {
                self.reg.address_latch = AddrByte::High;
                let res = self.reg.ppustat.bits | (self.reg.dyn_latch & 0b0001_1111);
                self.reg.ppustat.remove(VBLANK);
                res
            }
            0x0003 => self.reg.dyn_latch,
            0x0004 => {
                let res = self.oam[self.reg.oamaddr as usize / 4].read(self.reg.oamaddr as u16);
                self.reg.oamaddr = self.reg.oamaddr.wrapping_add(1);
                res
            }
            0x0005 => self.reg.dyn_latch,
            0x0006 => self.reg.dyn_latch,
            0x0007 => {
                let res = self.ppu_mem.read(self.reg.ppuaddr);
                self.incr_ppuaddr();
                res
            }
            x => invalid_address!(x),
        }
    }

    fn write(&mut self, idx: u16, val: u8) {
        self.reg.dyn_latch = val;
        match idx % 8 {
            0x0000 => self.reg.ppuctrl = PPUCtrl::new(val),
            0x0001 => self.reg.ppumask = PPUMask::from_bits_truncate(val),
            0x0002 => (),
            0x0003 => self.reg.oamaddr = val,
            0x0004 => {
                self.oam[self.reg.oamaddr as usize / 4].write(self.reg.oamaddr as u16, val);
                self.reg.oamaddr = self.reg.oamaddr.wrapping_add(1);
            }
            0x0005 => write_addr_byte(&mut self.reg.address_latch, &mut self.reg.ppuscroll, val),
            0x0006 => write_addr_byte(&mut self.reg.address_latch, &mut self.reg.ppuaddr, val),
            0x0007 => {
                self.ppu_mem.write(self.reg.ppuaddr, val);
                self.incr_ppuaddr();
            }
            x => invalid_address!(x),
        }
    }
}
