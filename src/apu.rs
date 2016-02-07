use super::memory::MemSegment;
use audio::{AudioOut};

const NES_FPS: usize = 60;
const FRAMES_PER_BUFFER : usize = 6;
pub const BUFFERS_PER_SECOND : usize = NES_FPS / FRAMES_PER_BUFFER; //must always be a positive integer

const SAMPLE_RATE: usize = 44100;
const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / NES_FPS);
pub const BUFFER_SIZE: usize = SAMPLES_PER_FRAME * FRAMES_PER_BUFFER;

pub struct OutputBuffer {
    pub samples: [f32; BUFFER_SIZE as usize],
}

struct Pulse {
    flags: u8,
    sweep: u8,
    timer: u8,
    length: u8,
}

impl Pulse {
    fn new() -> Pulse {
        Pulse {
            flags: 0,
            sweep: 0,
            timer: 0,
            length: 0,
        }
    }
}

struct Triangle {
    counter: u8,
    timer: u8,
    length: u8,
}

impl Triangle {
    fn new() -> Triangle {
        Triangle {
            counter: 0,
            timer: 0,
            length: 0,
        }
    }
}

struct Noise {
    volume: u8,
    mode: u8,
    length: u8,
}

impl Noise {
    fn new() -> Noise {
        Noise {
            volume: 0,
            mode: 0,
            length: 0,
        }
    }
}

struct DMC {
    freq: u8,
    direct: u8,
    addr: u8,
    length: u8,
}

impl DMC {
    fn new() -> DMC {
        DMC {
            freq: 0,
            direct: 0,
            addr: 0,
            length: 0,
        }
    }
}

pub struct APU {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: DMC,
    frame: u8,
    control: u8,
    status: u8,
    
    frame_count: usize,
    
    square: SquareWave,
    device: Box<AudioOut>,
}

struct SquareWave {
    phase_inc: f32,
    phase: f32,
    volume: f32
}

impl APU {
    pub fn new( device: Box<AudioOut> ) -> APU {
        APU {
            pulse1: Pulse::new(),
            pulse2: Pulse::new(),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: DMC::new(),
            frame: 0,
            control: 0,
            status: 0,
            
            frame_count: 0,
            
            square: SquareWave {
                phase_inc: 612.0 / SAMPLE_RATE as f32,
                phase: 0.0,
                volume: 0.25
            },
            device: device,
        }
    }
    
    pub fn generate(&mut self) {
        self.frame_count += 1;
        if self.frame_count % FRAMES_PER_BUFFER != 0 {
            return;
        }
        
        let mut buffer = OutputBuffer {
            samples: [0f32; BUFFER_SIZE as usize],
        };
        
        for x in buffer.samples.iter_mut() {
            *x = match self.square.phase {
                0.0...0.5 => self.square.volume,
                _ => -self.square.volume
            };
            self.square.phase = (self.square.phase + self.square.phase_inc) % 1.0;
        }
        
        self.device.play(&buffer);
    }
}

impl MemSegment for APU {
    fn read(&mut self, idx: u16) -> u8 {
        match idx % 0x20 {
            0x0015 => self.status,
            _ => 0,
        }
    }

    fn write(&mut self, idx: u16, val: u8) {
        match idx % 0x20 {
            0x0000 => self.pulse1.flags = val,
            0x0001 => self.pulse1.sweep = val,
            0x0002 => self.pulse1.timer = val,
            0x0003 => self.pulse1.length = val,
            0x0004 => self.pulse2.flags = val,
            0x0005 => self.pulse2.sweep = val,
            0x0006 => self.pulse2.timer = val,
            0x0007 => self.pulse2.length = val,
            0x0008 => self.triangle.counter = val,
            0x0009 => (),
            0x000A => self.triangle.timer = val,
            0x000B => self.triangle.length = val,
            0x000C => self.noise.volume = val,
            0x000D => (),
            0x000E => self.noise.mode = val,
            0x000F => self.noise.length = val,
            0x0010 => self.dmc.freq = val,
            0x0011 => self.dmc.direct = val,
            0x0012 => self.dmc.addr = val,
            0x0013 => self.dmc.length = val,
            0x0014 => (),
            0x0015 => self.control = val,
            0x0016 => (),
            0x0017 => self.frame = val,
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::MemSegment;
    use audio::DummyAudioOut;

    fn assert_register_writable(idx: u16, getter: &Fn(&APU) -> u8) {
        let mut apu = APU::new(Box::new(DummyAudioOut));
        apu.write(idx, 12);
        assert_eq!(getter(&apu), 12);
        apu.write(idx, 125);
        assert_eq!(getter(&apu), 125);
    }

    fn assert_register_not_readable(idx: u16) {
        let mut apu = APU::new(Box::new(DummyAudioOut));
        apu.write(idx, 12);
        assert_eq!(apu.read(idx), 0);
        apu.write(idx, 125);
        assert_eq!(apu.read(idx), 0);
    }

    fn test_writable_register(idx: u16, getter: &Fn(&APU) -> u8) {
        assert_register_writable(idx, getter);
        assert_register_not_readable(idx);
    }

    #[test]
    fn test_writable_registers() {
        test_writable_register(0x4000, &|ref apu| apu.pulse1.flags);
        test_writable_register(0x4001, &|ref apu| apu.pulse1.sweep);
        test_writable_register(0x4002, &|ref apu| apu.pulse1.timer);
        test_writable_register(0x4003, &|ref apu| apu.pulse1.length);
        test_writable_register(0x4004, &|ref apu| apu.pulse2.flags);
        test_writable_register(0x4005, &|ref apu| apu.pulse2.sweep);
        test_writable_register(0x4006, &|ref apu| apu.pulse2.timer);
        test_writable_register(0x4007, &|ref apu| apu.pulse2.length);
        test_writable_register(0x4008, &|ref apu| apu.triangle.counter);
        test_writable_register(0x400A, &|ref apu| apu.triangle.timer);
        test_writable_register(0x400B, &|ref apu| apu.triangle.length);
        test_writable_register(0x400C, &|ref apu| apu.noise.volume);
        test_writable_register(0x400E, &|ref apu| apu.noise.mode);
        test_writable_register(0x400F, &|ref apu| apu.noise.length);
        test_writable_register(0x4010, &|ref apu| apu.dmc.freq);
        test_writable_register(0x4011, &|ref apu| apu.dmc.direct);
        test_writable_register(0x4012, &|ref apu| apu.dmc.addr);
        test_writable_register(0x4013, &|ref apu| apu.dmc.length);
        test_writable_register(0x4017, &|ref apu| apu.frame);
    }
}
