//! Embedded release binaries.
//!
//! Each binary is compiled into the installer at build time via `include_bytes!`.
//! The build script (`build.rs`) warns if any binary is missing.

/// Descriptor for an embedded binary payload.
pub struct EmbeddedBinary {
    /// Display name shown in the installer UI.
    pub name: &'static str,
    /// Filename to write on disk (without `.exe` — the installer appends it on Windows).
    pub filename: &'static str,
    /// Raw bytes of the release binary.
    pub data: &'static [u8],
    /// Whether this binary gets a desktop entry / .app bundle.
    pub create_shortcut: bool,
    /// Human-friendly description for the shortcut / .desktop comment.
    pub description: &'static str,
}

// We cannot conditionally include_bytes at compile time without a feature gate
// or a build-script-generated file. Instead, we use a cfg-gated approach:
// the binaries are included only when the `embed` feature is active.
// The justfile recipe enables this feature after building the workspace.

#[cfg(not(feature = "embed"))]
pub const BINARIES: &[EmbeddedBinary] = &[
    EmbeddedBinary {
        name: "Mujrim Elite Engine",
        filename: "mujrim",
        data: &[],
        create_shortcut: false,
        description: "Mujrim architecture-aware UCI engine",
    },
    EmbeddedBinary {
        name: "Mujrim v60 Engine",
        filename: "mujrim-v60",
        data: &[],
        create_shortcut: false,
        description: "Mujrim self-contained v60 fallback engine",
    },
    EmbeddedBinary {
        name: "Mujrim UI",
        filename: "mujrim-ui",
        data: &[],
        create_shortcut: true,
        description: "Mujrim Chess GUI",
    },
    EmbeddedBinary {
        name: "Mujrim Game",
        filename: "mujrim-game",
        data: &[],
        create_shortcut: true,
        description: "Mujrim 3D Chess Game",
    },
    EmbeddedBinary {
        name: "Mujrim Updater",
        filename: "mujrim-updater",
        data: &[],
        create_shortcut: false,
        description: "Mujrim update manager",
    },
];

#[cfg(feature = "embed")]
include!(concat!(env!("OUT_DIR"), "/embedded_payload.rs"));

/// Total embedded payload size in bytes.
pub fn total_size() -> u64 {
    BINARIES.iter().map(|b| b.data.len() as u64).sum()
}

/// Whether binaries are actually embedded (vs empty stubs for `cargo check`).
pub fn has_payload() -> bool {
    BINARIES.iter().any(|b| !b.data.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_list_is_populated() {
        assert_eq!(BINARIES.len(), 5);
    }

    #[test]
    fn filenames_are_unique() {
        let mut names: Vec<&str> = BINARIES.iter().map(|b| b.filename).collect();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            BINARIES.len(),
            "duplicate filenames in BINARIES"
        );
    }

    #[test]
    fn shortcut_binaries_have_descriptions() {
        for b in BINARIES {
            if b.create_shortcut {
                assert!(!b.description.is_empty(), "{} missing description", b.name);
            }
        }
    }
}
