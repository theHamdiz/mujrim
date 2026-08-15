//! Shared HTTP Range-resume downloads for NNUE and Syzygy payloads.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub fn part_path(dest: &Path) -> PathBuf {
    dest.with_extension(format!(
        "{}.part",
        dest.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("download")
    ))
}

pub fn resume_len(part: &Path) -> u64 {
    part.metadata().map(|metadata| metadata.len()).unwrap_or(0)
}

pub fn validate_size(actual: u64, expected: Option<u64>) -> Result<(), String> {
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(format!("incomplete download: {actual} of {expected} bytes"));
    }
    Ok(())
}

pub fn download_resumable(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
) -> Result<(), String> {
    let part = part_path(dest);
    let resume_at = resume_len(&part);
    let mut request = client.get(url);
    if resume_at > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_at}-"));
    }
    let mut response = request
        .send()
        .map_err(|e| format!("Request failed for {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {url}", response.status()));
    }

    let resumed = resume_at > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let starting_size = if resumed { resume_at } else { 0 };
    let expected_from_response = response
        .content_length()
        .map(|remaining| starting_size + remaining);
    let expected = expected_size.or(expected_from_response);

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(|e| format!("Create partial file: {e}"))?;

    let mut buffer = vec![0u8; 65_536].into_boxed_slice();
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| format!("Read: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])
            .map_err(|e| format!("Write: {e}"))?;
    }

    file.flush().map_err(|e| format!("Flush: {e}"))?;
    drop(file);
    let actual_size = part.metadata().map_err(|e| format!("Metadata: {e}"))?.len();
    validate_size(actual_size, expected)?;
    fs::rename(&part, dest).map_err(|e| format!("Finalize file: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_keeps_the_original_extension() {
        assert_eq!(
            part_path(Path::new("ateed_default.bin")),
            PathBuf::from("ateed_default.bin.part")
        );
        assert_eq!(
            part_path(Path::new("sf_current.nnue")),
            PathBuf::from("sf_current.nnue.part")
        );
    }

    #[test]
    fn resume_len_is_zero_when_the_part_file_is_missing() {
        assert_eq!(
            resume_len(Path::new("/nonexistent/mujrim-ateed.bin.part")),
            0
        );
    }

    #[test]
    fn validate_size_rejects_truncated_payloads() {
        assert!(validate_size(17_327_452, Some(17_327_452)).is_ok());
        assert!(
            validate_size(100, Some(17_327_452))
                .unwrap_err()
                .contains("incomplete download")
        );
        assert!(validate_size(12, None).is_ok());
    }

    #[test]
    fn resume_len_reads_an_existing_part_file() {
        let path = std::env::temp_dir().join("mujrim-ateed-resume.bin.part");
        std::fs::write(&path, [0u8; 64]).unwrap();
        assert_eq!(resume_len(&path), 64);
        let _ = std::fs::remove_file(&path);
    }
}
