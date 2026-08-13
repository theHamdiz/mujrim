//! Host-native binary architecture detection for engine auto-discovery.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Coarse CPU architecture of an executable image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryArch {
    X86_64,
    Aarch64,
    X86,
    Other(u16),
}

impl BinaryArch {
    pub const fn host() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::Aarch64
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::Other(0)
        }
    }

    pub const fn matches_host(self) -> bool {
        matches!(
            (self, Self::host()),
            (Self::X86_64, Self::X86_64) | (Self::Aarch64, Self::Aarch64)
        )
    }
}

/// Read the machine type of a PE / ELF / Mach-O binary. Returns `None` if the
/// file is missing, unreadable, or not a recognized executable image.
pub fn detect_binary_arch(path: &Path) -> Option<BinaryArch> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    match magic {
        [0x4D, 0x5A, _, _] => detect_pe_arch(&mut file),
        [0x7F, b'E', b'L', b'F'] => detect_elf_arch(&mut file),
        [0xFE, 0xED, 0xFA, 0xCE]
        | [0xCE, 0xFA, 0xED, 0xFE]
        | [0xFE, 0xED, 0xFA, 0xCF]
        | [0xCF, 0xFA, 0xED, 0xFE]
        | [0xCA, 0xFE, 0xBA, 0xBE] => detect_macho_arch(&mut file, magic),
        _ => None,
    }
}

pub fn is_host_native_binary(path: &Path) -> bool {
    detect_binary_arch(path).is_some_and(BinaryArch::matches_host)
}

fn detect_pe_arch(file: &mut File) -> Option<BinaryArch> {
    file.seek(SeekFrom::Start(0x3C)).ok()?;
    let mut pe_offset_bytes = [0u8; 4];
    file.read_exact(&mut pe_offset_bytes).ok()?;
    let pe_offset = u32::from_le_bytes(pe_offset_bytes) as u64;
    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut pe_sig = [0u8; 4];
    file.read_exact(&mut pe_sig).ok()?;
    if &pe_sig != b"PE\0\0" {
        return None;
    }
    let mut machine = [0u8; 2];
    file.read_exact(&mut machine).ok()?;
    Some(match u16::from_le_bytes(machine) {
        0x8664 => BinaryArch::X86_64,
        0xAA64 => BinaryArch::Aarch64,
        0x014C => BinaryArch::X86,
        other => BinaryArch::Other(other),
    })
}

fn detect_elf_arch(file: &mut File) -> Option<BinaryArch> {
    // e_machine is at offset 18 in both ELF32 and ELF64.
    file.seek(SeekFrom::Start(18)).ok()?;
    let mut machine = [0u8; 2];
    file.read_exact(&mut machine).ok()?;
    // ELF may be little or big endian; chess engines on our targets are LE.
    Some(match u16::from_le_bytes(machine) {
        0x3E => BinaryArch::X86_64,
        0xB7 => BinaryArch::Aarch64,
        0x03 => BinaryArch::X86,
        other => BinaryArch::Other(other),
    })
}

fn detect_macho_arch(file: &mut File, magic: [u8; 4]) -> Option<BinaryArch> {
    // Fat binaries: reject unless a single host slice is trivial to prove; for
    // auto-detect we only accept thin Mach-O of the host CPU.
    if magic == [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    file.seek(SeekFrom::Start(4)).ok()?;
    let mut cpu = [0u8; 4];
    file.read_exact(&mut cpu).ok()?;
    let cpu_type = if magic == [0xFE, 0xED, 0xFA, 0xCE] || magic == [0xFE, 0xED, 0xFA, 0xCF] {
        u32::from_be_bytes(cpu)
    } else {
        u32::from_le_bytes(cpu)
    };
    // CPU_TYPE_X86_64 = 0x01000007, CPU_TYPE_ARM64 = 0x0100000C
    Some(match cpu_type {
        0x0100_0007 => BinaryArch::X86_64,
        0x0100_000C => BinaryArch::Aarch64,
        other => BinaryArch::Other((other & 0xFFFF) as u16),
    })
}

/// Build a minimal PE header for tests (x64 or arm64).
#[cfg(test)]
pub fn synthetic_pe_bytes(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x80];
    bytes[0] = 0x4D;
    bytes[1] = 0x5A;
    bytes[0x3C] = 0x40; // PE header at 0x40
    bytes[0x40] = b'P';
    bytes[0x41] = b'E';
    bytes[0x44] = (machine & 0xFF) as u8;
    bytes[0x45] = (machine >> 8) as u8;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detects_pe_x64_and_arm64() {
        let dir = tempfile_dir();
        let x64 = dir.join("x64.exe");
        let arm = dir.join("arm64.exe");
        std::fs::write(&x64, synthetic_pe_bytes(0x8664)).unwrap();
        std::fs::write(&arm, synthetic_pe_bytes(0xAA64)).unwrap();
        assert_eq!(detect_binary_arch(&x64), Some(BinaryArch::X86_64));
        assert_eq!(detect_binary_arch(&arm), Some(BinaryArch::Aarch64));
    }

    #[test]
    fn host_match_rejects_foreign_pe() {
        let dir = tempfile_dir();
        let foreign = dir.join("foreign.exe");
        let machine = if BinaryArch::host() == BinaryArch::Aarch64 {
            0x8664
        } else {
            0xAA64
        };
        std::fs::write(&foreign, synthetic_pe_bytes(machine)).unwrap();
        assert!(!is_host_native_binary(&foreign));
        let native = dir.join("native.exe");
        let host_machine = match BinaryArch::host() {
            BinaryArch::Aarch64 => 0xAA64,
            BinaryArch::X86_64 => 0x8664,
            _ => 0x8664,
        };
        std::fs::write(&native, synthetic_pe_bytes(host_machine)).unwrap();
        assert!(is_host_native_binary(&native));
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mujrim-binary-arch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn rejects_non_executable_magic() {
        let dir = tempfile_dir();
        let path = dir.join("notes.txt");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "hello").unwrap();
        assert_eq!(detect_binary_arch(&path), None);
    }
}
