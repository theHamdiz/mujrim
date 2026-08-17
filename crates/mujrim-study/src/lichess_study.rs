//! Lichess study PGN: multi-chapter documents with commentary and board shapes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::database::StudyDatabase;
use crate::move_tree::MoveTree;
use crate::opening::START_FEN;
use crate::study_doc::{self, Chapter, Study};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LichessImportReport {
    pub files: usize,
    pub studies: usize,
    pub chapters: usize,
    pub skipped: usize,
}

pub fn looks_like_lichess_study(text: &str) -> bool {
    text.contains("[StudyName ") || text.contains("[ChapterName ")
}

pub fn parse_studies(text: &str) -> Result<Vec<Study>, String> {
    types::init();
    let blocks = split_raw_games(text);
    if blocks.is_empty() {
        return Err("PGN does not contain a Lichess study chapter".to_owned());
    }
    let mut grouped: BTreeMap<String, Study> = BTreeMap::new();
    let mut last_error = None;
    for block in blocks {
        match parse_chapter_block(&block) {
            Ok((study_id, study_name, chapter)) => {
                let entry = grouped.entry(study_id.clone()).or_insert_with(|| Study {
                    id: study_id,
                    title: study_name,
                    chapters: Vec::new(),
                    source_key: String::new(),
                    annotator: chapter.annotator.clone(),
                });
                if entry.annotator.is_empty() {
                    entry.annotator = chapter.annotator.clone();
                }
                if !entry
                    .chapters
                    .iter()
                    .any(|existing| existing.id == chapter.id || existing.title == chapter.title)
                {
                    entry.chapters.push(chapter);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    if grouped.is_empty() {
        return Err(
            last_error.unwrap_or_else(|| "PGN does not contain a Lichess study chapter".to_owned())
        );
    }
    Ok(grouped.into_values().collect())
}

pub fn export_study(study: &Study) -> String {
    let mut out = String::new();
    for chapter in &study.chapters {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "[Event \"{}: {}\"]\n[Result \"*\"]\n[Variant \"Standard\"]\n[StudyName \"{}\"]\n[ChapterName \"{}\"]\n",
            escape_tag(&study.title),
            escape_tag(&chapter.title),
            escape_tag(&study.title),
            escape_tag(&chapter.title),
        ));
        if !chapter.eco.is_empty() {
            out.push_str(&format!("[ECO \"{}\"]\n", escape_tag(&chapter.eco)));
        }
        if !chapter.opening.is_empty() {
            out.push_str(&format!("[Opening \"{}\"]\n", escape_tag(&chapter.opening)));
        }
        if !chapter.annotator.is_empty() {
            out.push_str(&format!(
                "[Annotator \"{}\"]\n",
                escape_tag(&chapter.annotator)
            ));
        }
        if !chapter.chapter_url.is_empty() {
            out.push_str(&format!(
                "[ChapterURL \"{}\"]\n",
                escape_tag(&chapter.chapter_url)
            ));
        }
        if !chapter.orientation.is_empty() {
            out.push_str(&format!(
                "[Orientation \"{}\"]\n",
                escape_tag(&chapter.orientation)
            ));
        }
        if chapter.start_fen != START_FEN {
            out.push_str(&format!(
                "[FEN \"{}\"]\n[SetUp \"1\"]\n",
                escape_tag(&chapter.start_fen)
            ));
        }
        out.push('\n');
        if !chapter.chapter_notes.is_empty() {
            out.push_str(&format!(
                "{{ {} }} ",
                chapter.chapter_notes.replace('}', "\\}")
            ));
        }
        let moves = chapter.tree.to_pgn();
        if moves.is_empty() {
            out.push_str("*\n");
        } else {
            out.push_str(&moves);
            out.push_str(" *\n");
        }
    }
    out
}

pub fn import_studies_from_dir(
    database: &mut StudyDatabase,
    dir: impl AsRef<Path>,
) -> Result<LichessImportReport, String> {
    let mut report = LichessImportReport::default();
    let Ok(entries) = fs::read_dir(dir.as_ref()) else {
        return Ok(report);
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pgn"))
        })
        .collect();
    paths.sort();
    for path in paths {
        report.files += 1;
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if !looks_like_lichess_study(&text) {
            report.skipped += 1;
            continue;
        }
        let source_key = path.to_string_lossy().into_owned();
        match parse_studies(&text) {
            Ok(studies) => {
                for mut study in studies {
                    study.source_key = source_key.clone();
                    report.chapters += study.chapters.len();
                    database.save_study(&study)?;
                    report.studies += 1;
                }
            }
            Err(_) => report.skipped += 1,
        }
    }
    Ok(report)
}

pub fn shape_color(code: char) -> crate::board_marks::MarkColor {
    match code {
        'G' | 'g' => crate::board_marks::MarkColor::Green,
        'R' | 'r' => crate::board_marks::MarkColor::Red,
        'Y' | 'y' => crate::board_marks::MarkColor::Gold,
        'B' | 'b' => crate::board_marks::MarkColor::Blue,
        _ => crate::board_marks::MarkColor::Orange,
    }
}

pub fn shapes_to_marks(
    shapes: &[String],
) -> (Vec<crate::board_marks::BoardArrow>, Vec<types::Square>) {
    let mut arrows = Vec::new();
    let mut circles = Vec::new();
    for shape in shapes {
        if let Some(token) = shape.strip_prefix("cal:") {
            let bytes = token.as_bytes();
            if bytes.len() >= 5
                && let (Ok(from), Ok(to)) = (
                    token[1..3].parse::<types::Square>(),
                    token[3..5].parse::<types::Square>(),
                )
            {
                arrows.push(crate::board_marks::BoardArrow::new(
                    from,
                    to,
                    shape_color(token.chars().next().unwrap_or('G')),
                    crate::board_marks::ArrowRole::Coach,
                ));
            }
        } else if let Some(token) = shape.strip_prefix("csl:")
            && token.len() >= 3
            && let Ok(square) = token[1..3].parse::<types::Square>()
        {
            circles.push(square);
        }
    }
    (arrows, circles)
}

pub fn lichess_study_id(chapter_url: &str) -> Option<String> {
    let path = chapter_url.split("://").last().unwrap_or(chapter_url);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    while let Some(part) = parts.next() {
        if part.eq_ignore_ascii_case("study")
            && let Some(id) = parts.next()
            && id.len() >= 6
        {
            return Some(id.to_owned());
        }
    }
    None
}

fn parse_chapter_block(block: &str) -> Result<(String, String, Chapter), String> {
    let tags = collect_tags(block);
    let study_name = tag(&tags, "StudyName")
        .or_else(|| study_title_from_event(&tag(&tags, "Event")))
        .unwrap_or_else(|| "Imported study".to_owned());
    let chapter_name = tag(&tags, "ChapterName")
        .or_else(|| chapter_title_from_event(&tag(&tags, "Event"), &study_name))
        .unwrap_or_else(|| "Chapter".to_owned());
    let chapter_url = tag(&tags, "ChapterURL").unwrap_or_default();
    let study_id =
        lichess_study_id(&chapter_url).unwrap_or_else(|| study_doc::hash_id(&study_name));
    let start_fen = tag(&tags, "FEN").unwrap_or_else(|| START_FEN.to_owned());
    let movetext = movetext_of(block);
    let (preamble, tree) = parse_chapter_tree(&start_fen, &movetext)?;
    let mut chapter = Chapter::new(chapter_name, start_fen, tree);
    if !chapter_url.is_empty() {
        chapter.id = study_doc::hash_id(&chapter_url);
        chapter.chapter_url = chapter_url;
    }
    chapter.annotator = tag(&tags, "Annotator").unwrap_or_default();
    chapter.eco = tag(&tags, "ECO").unwrap_or_default();
    chapter.opening = tag(&tags, "Opening").unwrap_or_default();
    chapter.orientation = tag(&tags, "Orientation").unwrap_or_default();
    chapter.chapter_notes = preamble;
    Ok((study_id, study_name, chapter))
}

fn parse_chapter_tree(start_fen: &str, movetext: &str) -> Result<(String, MoveTree), String> {
    let (preamble, rest) = split_preamble(movetext);
    let rest = rest.trim();
    let tree = if rest.is_empty() || rest == "*" || is_result_only(rest) {
        MoveTree {
            start_fen: start_fen.to_owned(),
            children: Vec::new(),
        }
    } else {
        MoveTree::from_pgn(start_fen, rest)?
    };
    Ok((preamble, tree))
}

fn is_result_only(movetext: &str) -> bool {
    matches!(movetext, "*" | "1-0" | "0-1" | "1/2-1/2")
}

fn split_preamble(movetext: &str) -> (String, &str) {
    let trimmed = movetext.trim_start();
    if !trimmed.starts_with('{') {
        return (String::new(), movetext);
    }
    let mut depth = 0usize;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let body = trimmed[1..index].trim();
                    let rest = trimmed[index + 1..].trim_start();
                    return (body.to_owned(), rest);
                }
            }
            _ => {}
        }
    }
    (String::new(), movetext)
}

fn split_raw_games(input: &str) -> Vec<String> {
    let mut games = Vec::new();
    let mut current = String::new();
    let mut saw_movetext = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && saw_movetext {
            if !current.trim().is_empty() {
                games.push(current.trim().to_owned());
            }
            current.clear();
            saw_movetext = false;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('[') {
            saw_movetext = true;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        games.push(current.trim().to_owned());
    }
    games
}

fn collect_tags(source: &str) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        if let Some((name, value)) = parse_tag_line(trimmed) {
            tags.insert(name, value);
        }
    }
    tags
}

fn parse_tag_line(line: &str) -> Option<(String, String)> {
    let body = line.strip_prefix('[')?.strip_suffix(']')?;
    let separator = body.find(char::is_whitespace)?;
    let name = body[..separator].trim();
    let value = body[separator..].trim();
    if !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }
    Some((name.to_owned(), value[1..value.len() - 1].to_owned()))
}

fn tag(tags: &BTreeMap<String, String>, name: &str) -> Option<String> {
    tags.get(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn study_title_from_event(event: &Option<String>) -> Option<String> {
    event.as_ref().and_then(|event| {
        event
            .split_once(':')
            .map(|(title, _)| title.trim().to_owned())
            .filter(|title| !title.is_empty())
    })
}

fn chapter_title_from_event(event: &Option<String>, study_name: &str) -> Option<String> {
    event.as_ref().and_then(|event| {
        event
            .strip_prefix(&format!("{study_name}:"))
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn movetext_of(source: &str) -> String {
    let mut out = String::new();
    let mut tags_done = false;
    for line in source.lines() {
        if !tags_done && line.trim().starts_with('[') {
            continue;
        }
        tags_done = true;
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn escape_tag(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAFFORD: &str = r#"
[Event "Stafford Gambit Traps: Oh no my queen!"]
[Result "*"]
[StudyName "Stafford Gambit Traps"]
[ChapterName "Oh no my queen!"]
[ChapterURL "https://lichess.org/study/whCVdUeM/Ue5KaLXB"]
[Annotator "https://lichess.org/@/EricRosen"]
[ECO "C42"]
[Opening "Petrov's Defense: Stafford Gambit"]

1. e4 e5 2. Nf3 Nf6 3. Nxe5 Nc6 { The Stafford Gambit } 4. Nxc6 dxc6 5. d3 Bc5 6. Bg5? Nxe4!! { Oh no! My queen! } 7. Bxd8 (7. dxe4 Bxf2+! { [%cal Gd8d1,Re1d1] } 8. Kxf2 Qxd1 $19) 7... Bxf2+ *
"#;

    #[test]
    fn parse_groups_chapters_and_keeps_commentary() {
        types::init();
        let studies = parse_studies(STAFFORD).expect("parse");
        assert_eq!(studies.len(), 1);
        let study = &studies[0];
        assert_eq!(study.id, "whCVdUeM");
        assert_eq!(study.title, "Stafford Gambit Traps");
        assert_eq!(study.chapters.len(), 1);
        let chapter = &study.chapters[0];
        assert_eq!(chapter.title, "Oh no my queen!");
        assert_eq!(chapter.eco, "C42");
        assert!(chapter.annotator.contains("EricRosen"));
        let bg5 = chapter
            .tree
            .node_at(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .expect("Bg5");
        assert_eq!(bg5.san, "Bg5");
        assert_eq!(bg5.glyphs, "?");
        let nxe4 = &bg5.children[0];
        assert_eq!(nxe4.san, "Nxe4");
        assert_eq!(nxe4.glyphs, "!!");
        assert!(nxe4.comment.contains("Oh no"));
        let dxe4 = nxe4
            .children
            .iter()
            .find(|node| node.san == "dxe4")
            .expect("sideline");
        let bxf2 = dxe4
            .children
            .iter()
            .find(|node| node.san.starts_with("Bxf2"))
            .expect("Bxf2");
        assert!(bxf2.shapes.iter().any(|shape| shape.starts_with("cal:")));
    }

    #[test]
    fn export_round_trips_study_tags() {
        types::init();
        let studies = parse_studies(STAFFORD).expect("parse");
        let pgn = export_study(&studies[0]);
        assert!(pgn.contains("[StudyName \"Stafford Gambit Traps\"]"));
        assert!(pgn.contains("[ChapterName \"Oh no my queen!\"]"));
        assert!(pgn.contains("Nxe4"));
        let again = parse_studies(&pgn).expect("reparse");
        assert_eq!(again[0].chapters[0].title, "Oh no my queen!");
    }

    #[test]
    fn import_dir_indexes_lichess_pgns() {
        types::init();
        let root = std::env::temp_dir().join(format!(
            "mujrim-lichess-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("lichess_study_demo.pgn"), STAFFORD).unwrap();
        fs::write(root.join("plain.pgn"), "[Event \"X\"]\n\n1. e4 e5 1-0\n").unwrap();
        let mut db = StudyDatabase::open(root.join("library")).unwrap();
        let report = import_studies_from_dir(&mut db, &root).unwrap();
        assert_eq!(report.files, 2);
        assert_eq!(report.studies, 1);
        assert_eq!(report.chapters, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(db.list_studies().unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn study_id_parses_lichess_chapter_url() {
        assert_eq!(
            lichess_study_id("https://lichess.org/study/whCVdUeM/Ue5KaLXB").as_deref(),
            Some("whCVdUeM")
        );
    }

    #[test]
    fn comment_only_intro_chapter_keeps_notes() {
        types::init();
        let pgn = r#"
[Event "The Positional Library #1: Introduction"]
[StudyName "The Positional Library #1"]
[ChapterName "Introduction"]
[ChapterURL "https://lichess.org/study/2NjE96GW/WvmouVmi"]
[FEN "8/r1r1rrr1/r1r2r2/rrr2r2/r1r2r2/r1r2r2/r1r1rrr1/8 w - - 0 1"]
[SetUp "1"]

{ Hey everyone. Thanks for reading this first chapter. }

*
"#;
        let studies = parse_studies(pgn).expect("parse intro");
        assert_eq!(studies[0].chapters.len(), 1);
        let chapter = &studies[0].chapters[0];
        assert!(chapter.tree.children.is_empty());
        assert!(chapter.chapter_notes.contains("Hey everyone"));
        assert!(chapter.start_fen.starts_with("8/r1r1rrr1"));
    }

    #[test]
    fn indexes_lichess_downloads_when_present() {
        types::init();
        let dir = std::path::Path::new("/home/hamdiz/Downloads");
        if !dir.is_dir() {
            return;
        }
        let pgns = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("lichess_study_") && name.ends_with(".pgn")
                    })
            })
            .count();
        if pgns == 0 {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "mujrim-lichess-dl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&root).unwrap();
        let mut db = StudyDatabase::open(root.join("library")).unwrap();
        let mut failed = Vec::new();
        for path in fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|e| e.path()))
        {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lichess_study_") && name.ends_with(".pgn"))
            {
                let text = fs::read_to_string(&path).unwrap();
                if let Err(error) = parse_studies(&text) {
                    failed.push(format!("{}: {error}", path.display()));
                }
            }
        }
        let report = import_studies_from_dir(&mut db, dir).unwrap();
        assert!(
            failed.is_empty(),
            "Lichess Downloads PGNs failed to parse: {}",
            failed.join(" | ")
        );
        assert!(
            report.studies >= pgns,
            "expected at least {pgns} studies, got {}",
            report.studies
        );
        assert!(
            report.chapters >= 200,
            "Downloads studies should keep every chapter, got {}",
            report.chapters
        );
        assert_eq!(db.list_studies().unwrap().len(), report.studies);
        let _ = fs::remove_dir_all(root);
    }
}
