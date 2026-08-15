//! Select an official Lc0 compute backend for NVIDIA, AMD, or CPU.
//!
//! Mujrim does not ship Lc0's GPL search. This module only chooses a
//! `--backend` flag and a sibling binary name so the upstream engine can use
//! CUDA, ROCm/OpenCL/ONNX, or a CPU BLAS/Eigen build.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lc0DeviceKind {
    Nvidia,
    Amd,
    Cpu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lc0Launch {
    pub binary: PathBuf,
    pub backend: &'static str,
    pub extra_args: Vec<String>,
}

impl Lc0Launch {
    pub fn argv(&self) -> Vec<String> {
        let mut args = vec!["--backend".to_string(), self.backend.to_string()];
        args.extend(self.extra_args.iter().cloned());
        args
    }
}

/// Detect the preferred device from the environment and common sysfs/CLI probes.
pub fn detect_device_kind() -> Lc0DeviceKind {
    if let Ok(forced) = std::env::var("MUJRIM_LC0_DEVICE") {
        return match forced.to_ascii_lowercase().as_str() {
            "nvidia" | "cuda" | "cudnn" => Lc0DeviceKind::Nvidia,
            "amd" | "rocm" | "opencl" | "hip" => Lc0DeviceKind::Amd,
            _ => Lc0DeviceKind::Cpu,
        };
    }
    if nvidia_present() {
        return Lc0DeviceKind::Nvidia;
    }
    if amd_present() {
        return Lc0DeviceKind::Amd;
    }
    Lc0DeviceKind::Cpu
}

pub fn nvidia_present() -> bool {
    Path::new("/dev/nvidia0").exists()
        || Path::new("/dev/nvidiactl").exists()
        || command_succeeds("nvidia-smi", &["-L"])
}

pub fn amd_present() -> bool {
    Path::new("/dev/kfd").exists()
        || Path::new("/dev/dri/renderD128").exists() && rocm_hint()
        || command_succeeds("rocminfo", &[])
        || std::env::var_os("ROCM_PATH").is_some()
}

fn rocm_hint() -> bool {
    Path::new("/opt/rocm").is_dir() || Path::new("/sys/module/amdgpu").exists()
}

fn command_succeeds(bin: &str, args: &[&str]) -> bool {
    std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Prefer a device-specific sibling (`lc0-cuda`, `lc0-rocm`) when present,
/// otherwise the discovered `lc0` binary with the matching `--backend`.
pub fn plan_launch(discovered: &Path, device: Lc0DeviceKind) -> Lc0Launch {
    let dir = discovered.parent().unwrap_or(discovered);
    let (preferred_names, backend) = match device {
        Lc0DeviceKind::Nvidia => (
            &["lc0-cuda", "lc0-cudnn", "lc0-onnx-cuda"][..],
            first_supported_backend(discovered, &["cuda", "cudnn", "onnx-cuda", "eigen"]),
        ),
        Lc0DeviceKind::Amd => (
            &["lc0-rocm", "lc0-opencl", "lc0-onnx-rocm", "lc0-hip"][..],
            first_supported_backend(discovered, &["onnx-rocm", "opencl", "sycl", "hip", "eigen"]),
        ),
        Lc0DeviceKind::Cpu => (
            &["lc0-cpu", "lc0-eigen", "lc0-blas"][..],
            first_supported_backend(discovered, &["eigen", "blas", "onnx-cpu", "dnnl"]),
        ),
    };

    for name in preferred_names {
        let candidate = with_exe(dir.join(name));
        if candidate.is_file() {
            let extra_args = weights_args(&candidate);
            return Lc0Launch {
                binary: candidate,
                backend,
                extra_args,
            };
        }
    }

    Lc0Launch {
        binary: discovered.to_path_buf(),
        backend,
        extra_args: weights_args(discovered),
    }
}

fn with_exe(path: PathBuf) -> PathBuf {
    if cfg!(windows) && path.extension().is_none() {
        path.with_extension("exe")
    } else {
        path
    }
}

/// Bundled official Lc0 transformer (BT4-it332 / TCEC+CCC).
pub const LC0_BUNDLED_WEIGHTS_NAME: &str = "lc0_bt4.pb.gz";

/// Filenames searched for official lc0 `--weights`, strongest bundled net first.
pub const LC0_WEIGHT_NAMES: &[&str] = &[
    LC0_BUNDLED_WEIGHTS_NAME,
    "weights.pb.gz",
    "lc0_t1_512.pb.gz",
    "lc0_default.pb.gz",
    "192x15-2024.pb.gz",
    "lc0.pb.gz",
];

fn is_usable_lc0_weights(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .is_ok_and(|metadata| metadata.len() > 1_000_000)
}

fn weight_search_dirs(binary: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(binary) = binary
        && let Some(dir) = binary.parent()
    {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("nnue"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("nnue"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("nnue"));
        dirs.push(cwd.join("dist").join("nnue"));
        dirs.push(
            cwd.join("dist")
                .join(format!(
                    "{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ))
                .join("nnue"),
        );
        dirs.push(cwd.join("crates").join("mujrim-eval").join("resources"));
    }
    dirs
}

/// Locate bundled or downloaded official Lc0 `.pb.gz` weights.
pub fn discover_lc0_weights(binary: Option<&Path>) -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("MUJRIM_LC0_WEIGHTS") {
        let path = PathBuf::from(explicit);
        if is_usable_lc0_weights(&path) {
            return Some(path);
        }
    }
    for dir in weight_search_dirs(binary) {
        for name in LC0_WEIGHT_NAMES {
            let weights = dir.join(name);
            if is_usable_lc0_weights(&weights) {
                return Some(weights);
            }
        }
    }
    None
}

fn weights_args(binary: &Path) -> Vec<String> {
    discover_lc0_weights(Some(binary))
        .map(|weights| {
            vec![
                "--weights".to_string(),
                weights.to_string_lossy().into_owned(),
            ]
        })
        .unwrap_or_default()
}

fn first_supported_backend(binary: &Path, wanted: &[&'static str]) -> &'static str {
    let listed = advertised_backends(binary);
    wanted
        .iter()
        .copied()
        .find(|name| listed.iter().any(|listed| listed == name))
        .unwrap_or(wanted.last().copied().unwrap_or("eigen"))
}

fn advertised_backends(binary: &Path) -> Vec<String> {
    let Ok(output) = std::process::Command::new(binary).arg("--help").output() else {
        return Vec::new();
    };
    parse_advertised_backends(&String::from_utf8_lossy(&output.stdout))
}

fn parse_advertised_backends(help: &str) -> Vec<String> {
    let section = help
        .split("--backend=CHOICE")
        .nth(1)
        .or_else(|| help.split("UCI: Backend").nth(1))
        .unwrap_or("");
    let Some(values) = section.split("VALUES:").nth(1) else {
        return Vec::new();
    };
    values
        .split(']')
        .next()
        .unwrap_or_default()
        .split(',')
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_selects_nvidia() {
        let previous = std::env::var_os("MUJRIM_LC0_DEVICE");
        unsafe { std::env::set_var("MUJRIM_LC0_DEVICE", "cuda") };
        assert_eq!(detect_device_kind(), Lc0DeviceKind::Nvidia);
        unsafe { std::env::set_var("MUJRIM_LC0_DEVICE", "amd") };
        assert_eq!(detect_device_kind(), Lc0DeviceKind::Amd);
        unsafe { std::env::set_var("MUJRIM_LC0_DEVICE", "cpu") };
        assert_eq!(detect_device_kind(), Lc0DeviceKind::Cpu);
        match previous {
            Some(value) => unsafe { std::env::set_var("MUJRIM_LC0_DEVICE", value) },
            None => unsafe { std::env::remove_var("MUJRIM_LC0_DEVICE") },
        }
    }

    #[test]
    fn launch_falls_back_to_discovered_binary() {
        let path = PathBuf::from("/tmp/missing-lc0");
        let launch = plan_launch(&path, Lc0DeviceKind::Cpu);
        assert_eq!(launch.binary, path);
        assert_eq!(launch.backend, "dnnl");
        assert!(
            launch
                .argv()
                .starts_with(&["--backend".to_string(), "dnnl".to_string()])
        );
    }

    #[test]
    fn parse_backend_values_skips_earlier_choice_lists() {
        let help = "\
[UCI: FpuStrategy  DEFAULT: reduction  VALUES: reduction,absolute]
  -b,  --backend=CHOICE
               Neural network computational backend to use.
               [UCI: Backend  DEFAULT: eigen  VALUES: eigen,cuda,onnx-dml,dnnl]
";
        assert_eq!(
            parse_advertised_backends(help),
            vec!["eigen", "cuda", "onnx-dml", "dnnl"]
        );
    }

    fn write_usable_weights(path: &Path) {
        std::fs::write(path, vec![0u8; 1_000_001]).unwrap();
    }

    #[test]
    fn launch_picks_up_sibling_weights() {
        let dir = std::env::temp_dir().join(format!("mujrim-lc0-weights-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let weights = dir.join("weights.pb.gz");
        let binary = dir.join("lc0");
        write_usable_weights(&weights);
        std::fs::write(&binary, b"").unwrap();
        let launch = plan_launch(&binary, Lc0DeviceKind::Cpu);
        assert_eq!(
            launch.extra_args,
            vec![
                "--weights".to_string(),
                weights.to_string_lossy().into_owned()
            ]
        );
        let _ = std::fs::remove_file(&weights);
        let _ = std::fs::remove_file(&binary);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn launch_prefers_bundled_bt4_over_smaller_fallback() {
        let dir = std::env::temp_dir().join(format!("mujrim-lc0-bt4-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let bt4 = dir.join(LC0_BUNDLED_WEIGHTS_NAME);
        let fallback = dir.join("lc0_default.pb.gz");
        let binary = dir.join("lc0");
        write_usable_weights(&bt4);
        write_usable_weights(&fallback);
        std::fs::write(&binary, b"").unwrap();
        let launch = plan_launch(&binary, Lc0DeviceKind::Cpu);
        assert_eq!(
            launch.extra_args,
            vec!["--weights".to_string(), bt4.to_string_lossy().into_owned()]
        );
        let _ = std::fs::remove_file(&bt4);
        let _ = std::fs::remove_file(&fallback);
        let _ = std::fs::remove_file(&binary);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn argv_includes_backend_flag() {
        let launch = Lc0Launch {
            binary: PathBuf::from("lc0"),
            backend: "cuda",
            extra_args: vec!["--weights".into(), "net.pb.gz".into()],
        };
        assert_eq!(
            launch.argv(),
            vec![
                "--backend".to_string(),
                "cuda".to_string(),
                "--weights".to_string(),
                "net.pb.gz".to_string()
            ]
        );
    }
}
