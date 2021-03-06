use super::{Mapper, MapperParams, RomAddress};
use super::bank::*;

struct Mapper000 {
    prg_rom: MappingTable,
    chr_rom: Box<[u8]>,
    chr_ram: Box<[u8]>,
    prg_ram: Box<[u8]>,

    mode: &'static [u16; 4],
}

pub fn new(params: MapperParams) -> Box<Mapper> {
    let chr_ram = if params.chr_rom.is_empty() {
        vec![0u8; 0x2000].into_boxed_slice()
    } else {
        vec![0u8; 0].into_boxed_slice()
    };

    let mut prg_rom_table = MappingTable::new(params.prg_rom, 8);
    let bank_count = prg_rom_table.bank_count();
    for page in 0..8 {
        prg_rom_table.map_page(page, page % bank_count);
    }

    Box::new(Mapper000 {
        prg_rom: prg_rom_table,
        chr_rom: params.chr_rom.into_boxed_slice(),
        chr_ram: chr_ram,
        prg_ram: vec![0u8; params.prg_ram_size].into_boxed_slice(),
        mode: super::standard_mapping_tables(params.mirroring_mode),
    })
}

impl Mapper for Mapper000 {
    fn prg_rom_read(&mut self, idx: u16) -> &RomBank {
        self.prg_rom.get_bank(idx)
    }

    fn prg_rom_write(&mut self, idx: u16, _: u8) -> &mut RomBank {
        self.prg_rom.get_bank_mut(idx)
    }

    fn prg_rom_address(&self, idx: u16) -> RomAddress {
        self.prg_rom.get_rom_address(idx)
    }

    fn prg_ram_read(&mut self, idx: u16) -> u8 {
        self.prg_ram[((idx - 0x6000) as usize % self.prg_ram.len())]
    }

    fn prg_ram_write(&mut self, idx: u16, val: u8) {
        let idx = (idx - 0x6000) as usize % self.prg_ram.len();
        self.prg_ram[idx] = val;
    }

    fn chr_read(&mut self, idx: u16) -> u8 {
        if self.chr_rom.len() == 0 {
            self.chr_ram[idx as usize % self.chr_ram.len()]
        } else {
            self.chr_rom[idx as usize % self.chr_rom.len()]
        }
    }

    fn chr_write(&mut self, idx: u16, val: u8) {
        if self.chr_rom.len() == 0 {
            let len = self.chr_ram.len();
            self.chr_ram[idx as usize % len] = val;
        }
    }

    fn get_mirroring_table(&self) -> &[u16; 4] {
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mappers::{Mapper, MapperParams};

    #[test]
    fn test_can_create_mapper_0() {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        new(MapperParams::simple(path, vec![0u8; 0x1000], vec![]));
    }

    fn create_test_mapper() -> Box<Mapper> {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        new(MapperParams::simple(
            path,
            vec![0u8; 0x4000],
            vec![0u8; 0x4000],
        ))
    }

    #[test]
    fn test_prg_ram_read_write() {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        let mut params = MapperParams::simple(path, vec![0u8; 0x4000], vec![0u8; 0x4000]);
        params.prg_ram_size = 0x1000;
        let mut nrom = new(params);
        nrom.prg_ram_write(0x6111, 15);
        assert_eq!(nrom.prg_ram_read(0x6111), 15);

        nrom.prg_ram_write(0x6112, 16);
        assert_eq!(nrom.prg_ram_read(0x7112), 16);
    }

    #[test]
    fn test_prg_rom_read() {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        let prg_rom: Vec<_> = (0..0x4000).map(|val| (val % 0xFF) as u8).collect();
        let mut mapper = new(MapperParams::simple(path, prg_rom, vec![0u8; 0x4000]));

        assert_eq!(
            mapper.prg_rom_read(0x8111).read(0x8111),
            mapper.prg_rom_read(0xC111).read(0xC111)
        );
    }

    #[test]
    fn test_prg_rom_mirroring() {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        let mut prg_rom: Vec<_> = vec![0u8; 0x4000];
        prg_rom[0x2612] = 0x15;
        let mut mapper = new(MapperParams::simple(path, prg_rom, vec![0u8; 0x1000]));
        assert_eq!(mapper.prg_rom_read(0xA612).read(0xA612), 0x15);
    }

    #[test]
    fn test_prg_rom_write() {
        let mut mapper = create_test_mapper();

        mapper.prg_rom_write(0x8612, 15).write(0x8612, 15);
        assert_eq!(mapper.prg_rom_read(0x8612).read(0x8612), 0);
    }

    #[test]
    fn test_chr_rom_read() {
        let path_buf = ::std::path::PathBuf::new();
        let path = path_buf.as_path();
        let chr_rom: Vec<_> = (0..0x2000).map(|val| (val % 0xFF) as u8).collect();
        let mut mapper = new(MapperParams::simple(path, vec![0u8; 0x4000], chr_rom));

        assert_eq!(mapper.chr_read(0x8111), mapper.chr_read(0xC111));
    }

    #[test]
    fn test_chr_rom_write() {
        let mut mapper = create_test_mapper();

        mapper.chr_write(0x1612, 15);
        assert_eq!(mapper.chr_read(0x1612), 0);
    }
}
