use std::fs::File;
use std::io::Read;
use std::io::Result;

#[derive(Debug, PartialEq, Eq)]
pub enum ElfEndianess {
    Little,
    Big,
    Unknown,
}

impl From<u8> for ElfEndianess {
    fn from(num: u8) -> Self {
        match num {
            0x01 => ElfEndianess::Little,
            0x02 => ElfEndianess::Big,
            _ => ElfEndianess::Unknown,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ElfType {
    None,
    Rel,
    Exec,
    Dyn,
    Core,
    Unknown,
}

impl From<u16> for ElfType {
    fn from(num: u16) -> Self {
        match num {
            0x00 => ElfType::None,
            0x01 => ElfType::Rel,
            0x02 => ElfType::Exec,
            0x03 => ElfType::Dyn,
            0x04 => ElfType::Core,
            _ => ElfType::Unknown,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileType {
    Unknown,

    // magic = "Microsoft C/C++ MSF 7.00\r\n\x1a\x44\x53"
    // https://github.com/Microsoft/microsoft-pdb/blob/082c5290e5aff028ae84e43affa8be717aa7af73/PDB/msf/msf.cpp#L962
    Pdb,

    // magic = "\x7FELF"
    // https://refspecs.linuxfoundation.org/elf/gabi4+/ch4.eheader.html#elfid
    Elf(ElfType),

    // DOS Header 'MZ'
    // https://docs.microsoft.com/en-us/windows/win32/debug/pe-format
    PE,

    // magic = 0xfeedface, 0xfeedfacf, 0xcefaedfe, 0xcffaedfe
    // in loader.h defines for mh_magic_64, mh_magic (plus endianness swapped)
    MachO,
}

pub fn file_type(file: &mut File) -> Result<FileType> {
    if file.metadata()?.len() < 32 {
        Ok(FileType::Unknown)
    } else {
        let mut buf: [u8; 32] = [0; 32];
        file.read_exact(&mut buf)?;
        match buf[0] {
            0x7F => {
                if &buf[1..4] == b"ELF" {
                    let endianness: ElfEndianess = buf[5].into();
                    let (lower_byte, upper_byte) = match endianness {
                        ElfEndianess::Little => (buf[16], buf[17]),
                        ElfEndianess::Big => (buf[17], buf[16]),
                        ElfEndianess::Unknown => return Ok(FileType::Unknown),
                    };
                    let elf_type: ElfType =
                        (((upper_byte as u16) << 8) | (lower_byte as u16)).into();

                    Ok(FileType::Elf(elf_type))
                } else {
                    Ok(FileType::Unknown)
                }
            }
            // 'M'
            0x4D => {
                match buf[1] {
                    // 'Z'
                    0x5A => Ok(FileType::PE),

                    // 'i'
                    0x69 => {
                        if &buf[2..29] == b"crosoft C/C++ MSF 7.00\r\n\x1a\x44\x53" {
                            Ok(FileType::Pdb)
                        } else {
                            Ok(FileType::Unknown)
                        }
                    }

                    _ => Ok(FileType::Unknown),
                }
            }
            0xFE => {
                if (buf[1..4] == [0xEDu8, 0xFAu8, 0xCEu8])
                    || (buf[1..4] == [0xEDu8, 0xFAu8, 0xCFu8])
                {
                    Ok(FileType::MachO)
                } else {
                    Ok(FileType::Unknown)
                }
            }
            0xCE | 0xCF => {
                if buf[1..4] == [0xFAu8, 0xEDu8, 0xFEu8] {
                    Ok(FileType::MachO)
                } else {
                    Ok(FileType::Unknown)
                }
            }
            _ => Ok(FileType::Unknown),
        }
    }
}
