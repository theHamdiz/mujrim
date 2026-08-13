//! Loader for Mujrim's native Akimbo-compatible raw network layout.

use std::path::Path;

use super::network::Network;

pub fn load(path: &Path) -> Result<Box<Network>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read NNUE file '{}': {error}", path.display()))?;
    let expected = std::mem::size_of::<Network>();
    if bytes.len() != expected {
        return Err(format!(
            "incompatible Akimbo NNUE size for '{}': expected {expected} bytes, found {}",
            path.display(),
            bytes.len()
        ));
    }

    let mut network = Box::<Network>::new_uninit();
    // SAFETY: Network contains only integer arrays, every bit pattern is valid, the destination
    // is correctly aligned, and the exact layout size was checked above.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            network.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
        Ok(network.assume_init())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_network_file() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/resources/ak_default.bin"
        ));
        let loaded = load(path).unwrap();
        assert_eq!(loaded.output_bias, super::super::network::net().output_bias);
    }

    #[test]
    fn rejects_incorrect_size() {
        let path = std::env::temp_dir().join("mujrim-invalid-akimbo.bin");
        std::fs::write(&path, b"too short").unwrap();
        let error = load(&path).err().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(error.contains("expected"));
    }
}
