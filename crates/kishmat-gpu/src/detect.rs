//! GPU/NPU/CPU auto-detection for training acceleration.
//!
//! Probes the system for available compute backends and returns
//! the best one for NNUE training workloads.

use std::fmt;
use std::process::Command;

/// Available GPU compute backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuBackend {
    /// Apple Metal (macOS, Apple Silicon or discrete AMD GPU)
    Metal {
        device_name: String,
        gpu_cores: u32,
        metal_version: String,
    },
    /// NVIDIA CUDA
    Cuda {
        device_name: String,
        compute_capability: String,
        vram_mb: u64,
    },
    /// AMD HIP/ROCm
    Hip {
        device_name: String,
    },
    /// CPU-only fallback (uses SIMD: AVX2/SSE/NEON)
    Cpu {
        cpu_name: String,
        cores: u32,
        simd_features: Vec<String>,
    },
}

impl fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuBackend::Metal { device_name, gpu_cores, metal_version } => {
                write!(f, "Metal ({device_name}, {gpu_cores} GPU cores, {metal_version})")
            }
            GpuBackend::Cuda { device_name, compute_capability, vram_mb } => {
                write!(f, "CUDA ({device_name}, SM {compute_capability}, {vram_mb}MB VRAM)")
            }
            GpuBackend::Hip { device_name } => {
                write!(f, "HIP/ROCm ({device_name})")
            }
            GpuBackend::Cpu { cpu_name, cores, simd_features } => {
                write!(f, "CPU ({cpu_name}, {cores} cores, SIMD: {})", simd_features.join(", "))
            }
        }
    }
}

/// Complete system information for training configuration.
#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub backend: GpuBackend,
    pub os: String,
    pub arch: String,
    pub total_memory_mb: u64,
}

impl fmt::Display for SystemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "KishMat GPU/NPU Detection")?;
        writeln!(f, "  OS:      {}", self.os)?;
        writeln!(f, "  Arch:    {}", self.arch)?;
        writeln!(f, "  Memory:  {} MB", self.total_memory_mb)?;
        write!(f, "  Backend: {}", self.backend)
    }
}

/// Detect the best available compute backend.
pub fn detect_best_backend() -> GpuBackend {
    // Try Metal first (macOS)
    #[cfg(target_os = "macos")]
    if let Some(metal) = detect_metal() {
        return metal;
    }

    // Try CUDA (Linux/Windows with NVIDIA GPU)
    if let Some(cuda) = detect_cuda() {
        return cuda;
    }

    // Try HIP/ROCm (Linux with AMD GPU)
    #[cfg(target_os = "linux")]
    if let Some(hip) = detect_hip() {
        return hip;
    }

    // Fallback: CPU
    detect_cpu()
}

/// Get full system information.
pub fn system_info() -> SystemInfo {
    let backend = detect_best_backend();

    let os = if cfg!(target_os = "macos") {
        "macOS".to_string()
    } else if cfg!(target_os = "linux") {
        "Linux".to_string()
    } else if cfg!(target_os = "windows") {
        "Windows".to_string()
    } else {
        "Unknown".to_string()
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64 (ARM64)".to_string()
    } else if cfg!(target_arch = "x86_64") {
        "x86_64".to_string()
    } else {
        std::env::consts::ARCH.to_string()
    };

    let total_memory_mb = get_total_memory_mb();

    SystemInfo {
        backend,
        os,
        arch,
        total_memory_mb,
    }
}

// ── Metal detection (macOS) ─────────────────────────────────────────

#[cfg(target_os = "macos")]
fn detect_metal() -> Option<GpuBackend> {
    // Use system_profiler to get GPU info
    let output = Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);

    // Parse chipset model
    let device_name = text.lines()
        .find(|l| l.contains("Chipset Model:"))
        .map(|l| l.split(':').nth(1).unwrap_or("Unknown").trim().to_string())
        .unwrap_or_else(|| "Apple GPU".to_string());

    // Parse GPU cores
    let gpu_cores = text.lines()
        .find(|l| l.contains("Total Number of Cores:"))
        .and_then(|l| l.split(':').nth(1)?.trim().parse::<u32>().ok())
        .unwrap_or(8);

    // Parse Metal version
    let metal_version = text.lines()
        .find(|l| l.contains("Metal Support:") || l.contains("Metal Family:"))
        .map(|l| l.split(':').nth(1).unwrap_or("Metal").trim().to_string())
        .unwrap_or_else(|| "Metal".to_string());

    Some(GpuBackend::Metal {
        device_name,
        gpu_cores,
        metal_version,
    })
}

#[cfg(not(target_os = "macos"))]
fn detect_metal() -> Option<GpuBackend> {
    None
}

// ── CUDA detection ──────────────────────────────────────────────────

fn detect_cuda() -> Option<GpuBackend> {
    // Check if nvidia-smi is available
    let output = Command::new("nvidia-smi")
        .arg("--query-gpu=name,compute_cap,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
        .ok()?;

    if !output.status.success() { return None; }

    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?;
    let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();

    if parts.len() >= 3 {
        Some(GpuBackend::Cuda {
            device_name: parts[0].to_string(),
            compute_capability: parts[1].to_string(),
            vram_mb: parts[2].parse().unwrap_or(0),
        })
    } else {
        None
    }
}

// ── HIP/ROCm detection ─────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn detect_hip() -> Option<GpuBackend> {
    let output = Command::new("rocm-smi")
        .arg("--showproductname")
        .output()
        .ok()?;

    if !output.status.success() { return None; }

    let text = String::from_utf8_lossy(&output.stdout);
    let device_name = text.lines()
        .find(|l| l.contains("Card series:") || l.contains("GPU"))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| "AMD GPU".to_string());

    Some(GpuBackend::Hip { device_name })
}

// ── CPU detection (fallback) ────────────────────────────────────────

fn detect_cpu() -> GpuBackend {
    let cpu_name = get_cpu_name();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let mut simd_features = Vec::new();

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") { simd_features.push("AVX2".to_string()); }
        else if is_x86_feature_detected!("sse4.2") { simd_features.push("SSE4.2".to_string()); }
        if is_x86_feature_detected!("avx512f") { simd_features.push("AVX-512".to_string()); }
        if is_x86_feature_detected!("bmi2") { simd_features.push("BMI2".to_string()); }
        if is_x86_feature_detected!("popcnt") { simd_features.push("POPCNT".to_string()); }
    }

    #[cfg(target_arch = "aarch64")]
    {
        simd_features.push("NEON".to_string());
        // Apple Silicon always has NEON + advanced SIMD
    }

    if simd_features.is_empty() {
        simd_features.push("None".to_string());
    }

    GpuBackend::Cpu {
        cpu_name,
        cores,
        simd_features,
    }
}

fn get_cpu_name() -> String {
    #[cfg(target_os = "macos")]
    {
        Command::new("sysctl")
            .arg("-n")
            .arg("machdep.cpu.brand_string")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }

    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name"))
                    .map(|l| l.split(':').nth(1).unwrap_or("Unknown").trim().to_string())
            })
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "Unknown CPU".to_string()
    }
}

fn get_total_memory_mb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|b| b / (1024 * 1024))
            .unwrap_or(0)
    }

    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| {
                        l.split_whitespace().nth(1)?.parse::<u64>().ok()
                    })
            })
            .map(|kb| kb / 1024)
            .unwrap_or(0)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_backend() {
        let backend = detect_best_backend();
        println!("Detected backend: {backend}");
        // Should detect *something* — at minimum CPU fallback
        match &backend {
            GpuBackend::Cpu { cores, .. } => assert!(*cores > 0),
            GpuBackend::Metal { gpu_cores, .. } => assert!(*gpu_cores > 0),
            GpuBackend::Cuda { vram_mb, .. } => assert!(*vram_mb > 0),
            GpuBackend::Hip { .. } => {}
        }
    }

    #[test]
    fn test_system_info() {
        let info = system_info();
        println!("{info}");
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
        assert!(info.total_memory_mb > 0);
    }

    #[test]
    fn test_backend_display() {
        let cpu = GpuBackend::Cpu {
            cpu_name: "Test CPU".to_string(),
            cores: 4,
            simd_features: vec!["AVX2".to_string()],
        };
        let display = format!("{cpu}");
        assert!(display.contains("Test CPU"));
        assert!(display.contains("AVX2"));
    }
}
