//! Local Lichess-style studies: titled documents with chapter trees.

use crate::move_tree::MoveTree;
use crate::opening::START_FEN;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Study {
    pub id: String,
    pub title: String,
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub source_key: String,
    #[serde(default)]
    pub annotator: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub start_fen: String,
    pub tree: MoveTree,
    pub chapter_notes: String,
    #[serde(default)]
    pub annotator: String,
    #[serde(default)]
    pub eco: String,
    #[serde(default)]
    pub opening: String,
    #[serde(default)]
    pub chapter_url: String,
    #[serde(default)]
    pub orientation: String,
}

impl Study {
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        let id = hash_id(&title);
        Self {
            id,
            title,
            chapters: vec![Chapter::new("Chapter 1", START_FEN, MoveTree::default())],
            source_key: String::new(),
            annotator: String::new(),
        }
    }

    pub fn from_mainline(
        title: impl Into<String>,
        chapter_title: impl Into<String>,
        start_fen: &str,
        moves: &[String],
    ) -> Result<Self, String> {
        let title = title.into();
        let tree = MoveTree::from_mainline(start_fen, moves)?;
        Ok(Self {
            id: hash_id(&title),
            title,
            chapters: vec![Chapter::new(chapter_title, start_fen, tree)],
            source_key: String::new(),
            annotator: String::new(),
        })
    }
}

impl Chapter {
    pub fn new(title: impl Into<String>, start_fen: impl Into<String>, tree: MoveTree) -> Self {
        let title = title.into();
        let start_fen = start_fen.into();
        Self {
            id: hash_id(&format!("{title}\0{start_fen}")),
            title,
            start_fen,
            tree,
            chapter_notes: String::new(),
            annotator: String::new(),
            eco: String::new(),
            opening: String::new(),
            chapter_url: String::new(),
            orientation: String::new(),
        }
    }

    pub fn from_pgn(
        title: impl Into<String>,
        start_fen: &str,
        movetext: &str,
    ) -> Result<Self, String> {
        let tree = MoveTree::from_pgn(start_fen, movetext)?;
        Ok(Self::new(title, start_fen, tree))
    }
}

pub(crate) fn hash_id(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_study_starts_with_one_empty_chapter() {
        let study = Study::new("Italian");
        assert_eq!(study.title, "Italian");
        assert_eq!(study.chapters.len(), 1);
        assert_eq!(study.chapters[0].start_fen, START_FEN);
        assert!(study.chapters[0].tree.children.is_empty());
    }

    #[test]
    fn chapter_from_mainline_keeps_moves() {
        let study = Study::from_mainline("Prep", "Main", START_FEN, &["e2e4".into()]).unwrap();
        assert_eq!(study.chapters[0].tree.mainline(), vec!["e2e4"]);
    }
}
