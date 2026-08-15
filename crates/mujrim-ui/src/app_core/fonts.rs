//! Bundled and user-imported UI fonts.

use std::fs;
use std::path::{Path, PathBuf};

use super::settings::{DEFAULT_MONO_FONT, DEFAULT_UI_FONT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontChoice {
    pub family: String,
}

impl std::fmt::Display for FontChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.family)
    }
}

pub fn bundled_ui_fonts() -> Vec<FontChoice> {
    vec![
        FontChoice {
            family: DEFAULT_UI_FONT.to_owned(),
        },
        FontChoice {
            family: "sans-serif".to_owned(),
        },
    ]
}

pub fn bundled_mono_fonts() -> Vec<FontChoice> {
    vec![
        FontChoice {
            family: DEFAULT_MONO_FONT.to_owned(),
        },
        FontChoice {
            family: "monospace".to_owned(),
        },
    ]
}

pub fn user_font_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("mujrim");
    path.push("fonts");
    path
}

pub fn is_font_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "ttf" | "otf" | "ttc")
    )
}

pub fn import_font_file(source: &Path) -> Result<(String, PathBuf), String> {
    if source
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("font file name is not allowed".to_owned());
    }
    if !is_font_file(source) {
        return Err("Choose a .ttf, .otf, or .ttc font file.".to_owned());
    }
    let name = source
        .file_name()
        .ok_or_else(|| "invalid font path".to_owned())?
        .to_string_lossy()
        .into_owned();
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("font file name is not allowed".to_owned());
    }
    let dest_dir = user_font_dir();
    fs::create_dir_all(&dest_dir)
        .map_err(|error| format!("failed to create font folder: {error}"))?;
    let dest = dest_dir.join(&name);
    fs::copy(source, &dest).map_err(|error| format!("failed to copy font: {error}"))?;
    let family = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().replace('-', " "))
        .unwrap_or_else(|| name.clone());
    Ok((family, dest))
}

pub fn register_user_fonts(paths: &[String], register: impl Fn(&[u8])) {
    for path in paths {
        if let Ok(bytes) = fs::read(path) {
            register(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_defaults_are_inter_and_jetbrains_mono() {
        assert_eq!(bundled_ui_fonts()[0].family, DEFAULT_UI_FONT);
        assert_eq!(bundled_mono_fonts()[0].family, DEFAULT_MONO_FONT);
    }

    #[test]
    fn font_file_filter_accepts_open_type() {
        assert!(is_font_file(Path::new("Inter-Regular.ttf")));
        assert!(is_font_file(Path::new("Custom.OTF")));
        assert!(!is_font_file(Path::new("notes.txt")));
        assert!(!is_font_file(Path::new("escape.bin")));
    }

    #[test]
    fn import_rejects_path_traversal_names() {
        let err = import_font_file(Path::new("../evil.ttf")).unwrap_err();
        assert!(err.contains("allowed") || err.contains("Choose"));
    }

    #[test]
    fn import_copies_bundled_inter_into_user_font_dir() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/Inter-Regular.ttf");
        let (family, dest) = import_font_file(&source).expect("copy");
        assert!(dest.exists());
        assert!(family.to_ascii_lowercase().contains("inter"));
        let _ = fs::remove_file(dest);
    }
}
