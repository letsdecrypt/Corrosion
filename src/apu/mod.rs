mod components;
mod buffer;

use super::memory::MemSegment;
use audio::AudioOut;
use std::cmp;
use cpu::IrqInterrupt;
use std::cell::RefCell;
use std::rc::Rc;
use apu::components::*;
use apu::buffer::*;

pub type Sample = i16;

static NTSC_TICK_LENGTH_TABLE: [[u64; 6]; 2] = [[7459, 7456, 7458, 7458, 7458, 0000],
                                                [0001, 7458, 7456, 7458, 7458, 7452]];

static PULSE_DUTY_CYCLES: [[i16; 8]; 4] = [[0, 1, -1, 0, 0, 0, 0, 0],
                                           [0, 1, 0, -1, 0, 0, 0, 0],
                                           [0, 1, 0, 0, 0, -1, 0, 0],
                                           [0, -1, 0, 1, 0, 0, 0, 0]];

bitflags! {
    flags Frame : u8 {
        const MODE = 0b1000_0000, //0 = 4-step, 1 = 5-step
        const SUPPRESS_IRQ  = 0b0100_0000, //0 = disabled, 1 = enabled
    }
}

impl Frame {
    fn mode(&self) -> usize {
        if self.contains(MODE) {
            return 1;
        } else {
            return 0;
        }
    }
}

trait Writable {
    fn write(&mut self, idx: u16, val: u8);
}

struct Sweep {
    enable: bool,
    period: u8,
    negate: bool,
    shift: u8,

    is_pulse2: bool,
    divider: u8,
    reload: bool,
}

impl Sweep {
    fn new(is_pulse2: bool) -> Sweep {
        Sweep {
            enable: false,
            period: 0,
            negate: false,
            shift: 0,

            is_pulse2: is_pulse2,
            divider: 0,
            reload: false,
        }
    }

    fn write(&mut self, val: u8) {
        self.enable = (val & 0b1000_0000) != 0;
        self.period = (val & 0b0111_0000) >> 4;
        self.negate = (val & 0b0000_1000) != 0;
        self.shift = val & 0b0000_0111;
        self.reload = true;
    }

    fn tick(&mut self, timer: &mut Timer) {
        if !self.enable {
            return;
        }

        self.divider = self.divider.saturating_sub(1);
        if self.divider == 0 {
            self.divider = self.period;
            let period_shift = self.period_shift(timer);
            timer.add_period_shift(period_shift);
        }

        if self.reload {
            self.divider = self.period;
            self.reload = false;
        }
    }

    fn audible(&self) -> bool {
        // TODO
        true
    }

    fn period_shift(&self, timer: &Timer) -> i16 {
        let mut shift = timer.period() as i16;
        shift = shift >> self.shift;
        if self.negate {
            shift = -shift;
            if self.is_pulse2 {
                shift = shift + 1;
            }
        }
        shift
    }
}

struct Pulse {
    duty: usize,
    duty_index: usize,
    
    envelope: Envelope,
    sweep: Sweep,
    timer: Timer,
    length: Length,

	waveform: Waveform,
}

impl Pulse {
    fn new(is_pulse2: bool, buffer: Rc<RefCell<SampleBuffer>>) -> Pulse {
        Pulse {
            duty: 0,
            duty_index: 0,
            
            envelope: Envelope::new(),
            sweep: Sweep::new(is_pulse2),
            timer: Timer::new(2),
            length: Length::new(5),

            waveform: Waveform::new(buffer),
        }
    }

    fn length_tick(&mut self) {
        self.length.tick();
        let timer = &mut self.timer;
        self.sweep.tick(timer)
    }

    fn envelope_tick(&mut self) {
        self.envelope.tick();
    }

    fn play(&mut self, from_cyc: u32, to_cyc: u32) {
        if !self.sweep.audible() || !self.length.audible() {
            self.waveform.set_amplitude(0, from_cyc);
            return;
        }

        let volume = self.envelope.volume();

        let mut current_cyc = from_cyc;
        while let TimerClock::Clock = self.timer.run(&mut current_cyc, to_cyc) {
            self.duty_index = (self.duty_index + 1) % 8;
            match PULSE_DUTY_CYCLES[self.duty][self.duty_index] {
                -1 => self.waveform.set_amplitude(0, current_cyc),
                0 => (),
                1 => self.waveform.set_amplitude(volume, current_cyc),
                _ => (),
            };
        }
    }
}

impl Writable for Pulse {
    fn write(&mut self, idx: u16, val: u8) {
        match idx % 4 {
            0 => {
                self.duty = (val >> 6) as usize;
                self.length.write_halt(val);
                self.envelope.write(val);
            }
            1 => self.sweep.write(val),
            2 => self.timer.write_low(val),
            3 => {
                self.length.write_counter(val);
                self.timer.write_high(val);
            }
            _ => (),
        }
    }
}

#[allow(dead_code)] //TODO: Remove this
struct Triangle {
    counter: u8,
    timer: u8,
    length: Length,
}

#[allow(unused_variables)] //TODO: Remove this
impl Triangle {
    fn new() -> Triangle {
        Triangle {
            counter: 0,
            timer: 0,
            length: Length::new(7),
        }
    }

    fn length_tick(&mut self) {
        self.length.tick();
    }

    fn play(&mut self, from_cyc: u32, to_cyc: u32) {}
}

impl Writable for Triangle {
    fn write(&mut self, idx: u16, val: u8) {
        match idx % 4 {
            0 => self.length.write_halt(val),
            1 => (),
            2 => (),
            3 => self.length.write_counter(val),
            _ => (),
        }
    }
}

#[allow(dead_code)] //TODO: Remove this
struct Noise {
    envelope: Envelope,
    mode: u8,
    length: Length,
}

#[allow(unused_variables)] //TODO: Remove this
impl Noise {
    fn new() -> Noise {
        Noise {
            envelope: Envelope::new(),
            mode: 0,
            length: Length::new(5),
        }
    }

    fn length_tick(&mut self) {
        self.length.tick();
    }
    
    fn envelope_tick(&mut self) {
        self.envelope.tick();
    }

    fn play(&mut self, from_cyc: u32, to_cyc: u32) {}
}

impl Writable for Noise {
    fn write(&mut self, idx: u16, val: u8) {
        match idx % 4 {
            0 => {
                self.length.write_halt(val);
                self.envelope.write(val);
            }
            1 => (),
            2 => (),
            3 => self.length.write_counter(val),
            _ => (),
        }
    }
}

#[allow(dead_code)] //TODO: Remove this
struct DMC {
    freq: u8,
    direct: u8,
    sample_addr: u8,
    sample_length: u8,
}

#[allow(unused_variables)] //TODO: Remove this
impl DMC {
    fn new() -> DMC {
        DMC {
            freq: 0,
            direct: 0,
            sample_addr: 0,
            sample_length: 0,
        }
    }

    fn play(&mut self, from_cyc: u32, to_cyc: u32) {}
}

#[allow(unused_variables)] //TODO: Remove this
impl Writable for DMC {
    fn write(&mut self, idx: u16, val: u8) {}
}

enum Jitter {
    Delay(u64, u8),
    None,
}

pub struct APU {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: DMC,
    frame: Frame,
    
    square_buffer: Rc<RefCell<SampleBuffer>>,
    
    device: Box<AudioOut>,

    global_cyc: u64,
    tick: u8,
    next_tick_cyc: u64,
    next_transfer_cyc: u64,
    last_frame_cyc: u64,

    irq_requested: bool,

    jitter: Jitter,
}

impl APU {
    pub fn new(device: Box<AudioOut>) -> APU {
        let sample_rate = device.sample_rate();
        
        let square_buffer = Rc::new(RefCell::new(SampleBuffer::new(sample_rate)));
        let clocks_needed = square_buffer.borrow().clocks_needed() as u64;
        
        APU {
            pulse1: Pulse::new(false, square_buffer.clone()),
            pulse2: Pulse::new(true, square_buffer.clone()),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: DMC::new(),
            frame: Frame::empty(),

            square_buffer: square_buffer,

            device: device,

            global_cyc: 0,
            tick: 0,
            next_tick_cyc: NTSC_TICK_LENGTH_TABLE[0][0],
            next_transfer_cyc: clocks_needed,
            last_frame_cyc: 0,

            irq_requested: false,

            jitter: Jitter::None,
        }
    }

    pub fn run_to(&mut self, cpu_cycle: u64) -> IrqInterrupt {
        let mut interrupt = IrqInterrupt::None;

        while self.global_cyc < cpu_cycle {
            let current_cycle = self.global_cyc;

            let mut next_step = cmp::min(cpu_cycle, self.next_tick_cyc);
            next_step = cmp::min(next_step, self.next_transfer_cyc);

            if let Jitter::Delay(time, _) = self.jitter {
                next_step = cmp::min(next_step, time);
            }

            self.play(current_cycle, next_step);
            self.global_cyc = next_step;

            if let Jitter::Delay(time, val) = self.jitter {
                if self.global_cyc == time {
                    self.set_4017(val);
                    self.jitter = Jitter::None;
                }
            }
            if self.global_cyc == self.next_tick_cyc {
                interrupt = interrupt.or(self.tick());
            }
            if self.global_cyc == self.next_transfer_cyc {
                self.transfer();
            }
        }
        interrupt
    }

    /// Represents the 240Hz output of the frame sequencer's divider
    fn tick(&mut self) -> IrqInterrupt {
        self.tick += 1;
        let mode = self.frame.mode();
        self.next_tick_cyc = self.global_cyc + NTSC_TICK_LENGTH_TABLE[mode][self.tick as usize];

        match mode {
            0 => {
                match self.tick {
                    1 => {
                        self.envelope_tick();
                    }
                    2 => {
                        self.envelope_tick();
                        self.length_tick();
                    }
                    3 => {
                        self.envelope_tick();
                    }
                    4 => {
                        self.tick = 0;
                        self.envelope_tick();
                        self.length_tick();
                        return self.raise_irq();
                    }
                    _ => {
                        self.tick = 0;
                    }
                }
            }
            1 => {
                match self.tick {
                    1 => {
                        self.envelope_tick();
                        self.length_tick()
                    }
                    2 => {
                        self.envelope_tick();
                    }
                    3 => {
                        self.envelope_tick();
                        self.length_tick()
                    }
                    4 => {
                        self.envelope_tick();
                    }
                    5 => {
                        self.tick = 0;
                    } //4 is the actual last tick in the cycle.
                    _ => {
                        self.tick = 0;
                    }
                }
            }
            _ => (),
        }
        IrqInterrupt::None
    }

    fn envelope_tick(&mut self) {
        self.pulse1.envelope_tick();
        self.pulse2.envelope_tick();
        self.noise.envelope_tick();
    }

    fn length_tick(&mut self) {
        self.pulse1.length_tick();
        self.pulse2.length_tick();
        self.triangle.length_tick();
        self.noise.length_tick();
    }

    fn raise_irq(&mut self) -> IrqInterrupt {
        if !self.frame.contains(SUPPRESS_IRQ) {
            self.irq_requested = true;
            return IrqInterrupt::IRQ;
        }
        return IrqInterrupt::None;
    }

    fn play(&mut self, from_cyc: u64, to_cyc: u64) {
        let from = (from_cyc - self.last_frame_cyc) as u32;
        let to = (to_cyc - self.last_frame_cyc) as u32;
        self.pulse1.play(from, to);
        self.pulse2.play(from, to);
        self.triangle.play(from, to);
        self.noise.play(from, to);
        self.dmc.play(from, to);
    }

    fn transfer(&mut self) {
        let cpu_cyc = self.global_cyc;
        let cycles_since_last_frame = (cpu_cyc - self.last_frame_cyc) as u32;
        self.last_frame_cyc = cpu_cyc;
        
        let mut square_buf = self.square_buffer.borrow_mut(); 
        square_buf.end_frame(cycles_since_last_frame);
        let samples: Vec<Sample> = {
            let iter1 = square_buf.read().iter();
            iter1.cloned().collect()
        };
        self.next_transfer_cyc = cpu_cyc + square_buf.clocks_needed() as u64;
        self.device.play(&samples);
    }

    ///Returns the cycle number representing the next time the CPU should run the APU.
    ///Min of the next APU IRQ, the next DMC IRQ, and the next tick time. When the CPU cycle reaches
    ///this number, the CPU must run the APU.
    pub fn requested_run_cycle(&self) -> u64 {
        // In practice, the next tick time should cover the APU IRQ as well, since the
        // IRQ happens on tick boundaries. The DMC IRQ isn't implemented yet.
        // Using the tick time ensures that the APU will never get too far behind the
        // CPU.
        self.next_tick_cyc
    }

    fn set_4017(&mut self, val: u8) {
        self.frame = Frame::from_bits_truncate(val);
        if self.frame.contains(SUPPRESS_IRQ) {
            self.irq_requested = false;
        }

        self.tick = 0;
        self.next_tick_cyc = self.global_cyc + NTSC_TICK_LENGTH_TABLE[self.frame.mode()][0];
    }

    #[cfg_attr(rustfmt, rustfmt_skip)]
    pub fn read_status(&mut self, cycle: u64) -> (IrqInterrupt, u8) {
        let interrupt = self.run_to(cycle - 1);

        let mut status: u8 = 0;
        status = status | (self.pulse1.length.active() << 0);
        status = status | (self.pulse2.length.active() << 1);
        status = status | (self.triangle.length.active() << 2);
        status = status | (self.noise.length.active() << 3);
        status = status | if self.irq_requested { 1 << 6 } else { 0 };
    // TODO add DMC status
    // TODO add DMC interrupt flag
        self.irq_requested = false;

        (interrupt.or(self.run_to(cycle)), status)
    }

    pub fn write(&mut self, idx: u16, val: u8) {
        match idx % 0x20 {
            x @ 0x00...0x03 => self.pulse1.write(x, val),
            x @ 0x04...0x07 => self.pulse2.write(x, val),
            x @ 0x08...0x0B => self.triangle.write(x, val),
            x @ 0x0C...0x0F => self.noise.write(x, val),
            x @ 0x10...0x13 => self.dmc.write(x, val),
            0x0014 => (),
            0x0015 => {
                self.noise.length.set_enable(val & 0b0000_1000 != 0);
                self.triangle.length.set_enable(val & 0b0000_0100 != 0);
                self.pulse2.length.set_enable(val & 0b0000_0010 != 0);
                self.pulse1.length.set_enable(val & 0b0000_0001 != 0);
            }
            0x0016 => (),
            0x0017 => {
                if self.global_cyc % 2 == 0 {
                    self.set_4017(val);
                } else {
                    self.jitter = Jitter::Delay(self.global_cyc + 1, val);
                }
            }
            _ => (),
        }
    }
}