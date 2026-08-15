//! Ateed training-pipeline helpers.

use std::io;
use std::path::Path;

/// Write a well-formed Ateed payload, creating parent directories as needed.
pub fn emit_network(path: &Path, network: &eval::nnue::AteedNetwork) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, network.to_bytes())
}

/// Write a well-formed zero Ateed payload so the train/eval loop can be tested
/// before a real checkpoint exists.
pub fn emit_zero_network(path: &Path) -> io::Result<()> {
    emit_network(path, &eval::nnue::AteedNetwork::zero())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eval::nnue::NnueNetworkSource;

    #[test]
    fn emit_zero_network_loads_as_ateed() {
        types::init();
        let path = std::env::temp_dir().join("mujrim-trainer-ateed-zero.bin");
        emit_zero_network(&path).expect("write zero Ateed net");
        let net = eval::nnue::load_network(&path).expect("load emitted Ateed net");
        let _ = std::fs::remove_file(&path);
        assert_eq!(net.search_profile(), eval::nnue::NnueSearchProfile::Ateed);
        assert_eq!(
            net.info().file_size,
            eval::nnue::ateed_format::FILE_SIZE as u64
        );
    }
}
