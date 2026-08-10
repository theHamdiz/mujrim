//! Hardware Detection — CPU, SIMD features, and GPU.
//!
//! Detects the hardware available to the running benchmark process.

/// Detected hardware information.
#[derive(Clone, Debug)]
pub struct HardwareInfo {
    pub cpu_arch: String,
    pub cpu_cores: usize,
    pub simd_features: Vec<String>,
    pub gpu: String,
    pub npu: String,
}

impl HardwareInfo {
    /// Detect hardware on the current system.
    pub fn detect() -> Self {
        let cpu_arch = std::env::consts::ARCH.to_string();
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let simd_features = detect_simd_features();
        let gpu = detect_gpu();

        Self {
            cpu_arch,
            cpu_cores,
            simd_features,
            gpu,
            npu: "N/A".into(),
        }
    }

    /// Number of benchmark threads (cores − 2, min 1).
    pub fn bench_threads(&self) -> usize {
        self.cpu_cores.saturating_sub(2).max(1)
    }

    /// Format as display lines for the benchmark header.
    pub fn display_lines(&self) -> Vec<String> {
        let simd_str = if self.simd_features.is_empty() {
            "(none detected)".to_string()
        } else {
            self.simd_features.join(", ")
        };

        vec![
            format!("    CPU arch:   {}", self.cpu_arch),
            format!(
                "    CPU cores:  {} (using {} for bench)",
                self.cpu_cores,
                self.bench_threads()
            ),
            format!("    SIMD:       {simd_str}"),
            format!("    GPU:        {}", self.gpu),
            format!("    NPU:        {}", self.npu),
        ]
    }
}

/// Detect SIMD features usable by this process.
fn detect_simd_features() -> Vec<String> {
    #[allow(unused_mut)] // mutability depends on the target architecture
    let mut features = Vec::new();

    #[cfg(target_arch = "aarch64")]
    {
        features.push("NEON".into());
        if cfg!(target_feature = "dotprod") || std::arch::is_aarch64_feature_detected!("dotprod") {
            features.push("DotProd".into());
        }
        if cfg!(target_feature = "fp16") || std::arch::is_aarch64_feature_detected!("fp16") {
            features.push("FP16".into());
        }
        if cfg!(target_feature = "crc") || std::arch::is_aarch64_feature_detected!("crc") {
            features.push("CRC32".into());
        }
        if cfg!(target_feature = "aes") || std::arch::is_aarch64_feature_detected!("aes") {
            features.push("AES".into());
        }
        if cfg!(target_feature = "sha2") || std::arch::is_aarch64_feature_detected!("sha2") {
            features.push("SHA2".into());
        }
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            features.push("AVX-512".into());
        }
        if std::arch::is_x86_feature_detected!("avx2") {
            features.push("AVX2".into());
        }
        if std::arch::is_x86_feature_detected!("avx") {
            features.push("AVX".into());
        }
        if std::arch::is_x86_feature_detected!("sse4.2") {
            features.push("SSE4.2".into());
        }
        if std::arch::is_x86_feature_detected!("sse4.1") {
            features.push("SSE4.1".into());
        }
        if std::arch::is_x86_feature_detected!("popcnt") {
            features.push("POPCNT".into());
        }
        if std::arch::is_x86_feature_detected!("bmi2") {
            features.push("BMI2".into());
        }
    }

    features
}

/// Detect GPU on the current platform.
fn detect_gpu() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("lspci").output() {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                let gpus: Vec<String> = stdout
                    .lines()
                    .filter(|l| {
                        let lower = l.to_lowercase();
                        lower.contains("vga") || lower.contains("3d") || lower.contains("display")
                    })
                    .filter_map(|l| l.split(": ").nth(1).map(|s| s.trim().to_string()))
                    .collect();
                if !gpus.is_empty() {
                    return gpus.join("; ");
                }
            }
        }
        // Fallback: sysfs
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("card") && !name.contains('-') {
                    let vendor_path = entry.path().join("device/vendor");
                    if let Ok(vendor) = std::fs::read_to_string(&vendor_path) {
                        let vendor = vendor.trim();
                        let vendor_name = match vendor {
                            "0x1002" => "AMD/ATI",
                            "0x10de" => "NVIDIA",
                            "0x8086" => "Intel",
                            _ => vendor,
                        };
                        return format!("{vendor_name} (via sysfs)");
                    }
                }
            }
        }
        "N/A (not detected)".into()
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("system_profiler")
            .args(["SPDisplaysDataType"])
            .output()
        {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                let gpu_model = stdout
                    .lines()
                    .find(|l| l.contains("Chipset Model"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                return gpu_model;
            }
        }
        "N/A".into()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "N/A (no detection for this platform)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_detect() {
        let hw = HardwareInfo::detect();
        assert!(hw.cpu_cores >= 1);
        assert!(!hw.cpu_arch.is_empty());
    }

    #[test]
    fn test_bench_threads() {
        let mut hw = HardwareInfo::detect();
        hw.cpu_cores = 4;
        assert_eq!(hw.bench_threads(), 2);
        hw.cpu_cores = 1;
        assert_eq!(hw.bench_threads(), 1);
    }

    #[test]
    fn detected_simd_matches_runtime_architecture() {
        let features = detect_simd_features();
        #[cfg(target_arch = "aarch64")]
        assert!(features.iter().any(|feature| feature == "NEON"));
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx2") {
            assert!(features.iter().any(|feature| feature == "AVX2"));
        }
    }
}
