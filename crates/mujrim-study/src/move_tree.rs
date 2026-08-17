//! Branching study trees with comments, NAGs, and PGN variation I/O.

use crate::opening::START_FEN;
use types::{Board, Move};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoveNode {
    pub uci: String,
    pub san: String,
    pub comment: String,
    pub nag: Option<u8>,
    pub glyphs: String,
    pub children: Vec<MoveNode>,
    pub shapes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoveTree {
    pub start_fen: String,
    pub children: Vec<MoveNode>,
}

impl Default for MoveTree {
    fn default() -> Self {
        Self {
            start_fen: START_FEN.to_owned(),
            children: Vec::new(),
        }
    }
}

impl MoveTree {
    pub fn from_mainline(start_fen: impl Into<String>, moves: &[String]) -> Result<Self, String> {
        types::init();
        let start_fen = start_fen.into();
        let mut board = Board::from_fen(&start_fen)?;
        let mut children = Vec::new();
        let mut cursor = &mut children;
        for uci in moves {
            let mv = find_uci(&mut board, uci)?;
            let node = MoveNode {
                uci: mv.to_uci(),
                san: move_san(&board, mv),
                ..MoveNode::default()
            };
            cursor.push(node);
            board.make_move(mv);
            cursor = &mut cursor.last_mut().expect("just pushed").children;
        }
        Ok(Self {
            start_fen,
            children,
        })
    }

    pub fn mainline(&self) -> Vec<String> {
        let mut moves = Vec::new();
        let mut cursor = self.children.first();
        while let Some(node) = cursor {
            moves.push(node.uci.clone());
            cursor = node.children.first();
        }
        moves
    }

    pub fn node_at(&self, path: &[usize]) -> Option<&MoveNode> {
        let mut children = &self.children;
        let mut current = None;
        for &index in path {
            current = children.get(index);
            children = &current?.children;
        }
        current
    }

    pub fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut MoveNode> {
        fn walk<'a>(nodes: &'a mut [MoveNode], path: &[usize]) -> Option<&'a mut MoveNode> {
            let (first, rest) = path.split_first()?;
            let node = nodes.get_mut(*first)?;
            if rest.is_empty() {
                Some(node)
            } else {
                walk(&mut node.children, rest)
            }
        }
        walk(&mut self.children, path)
    }

    /// Play `uci` from `path`. Reuses an existing child or forks a sideline.
    pub fn play_uci(&mut self, path: &[usize], uci: &str) -> Result<Vec<usize>, String> {
        types::init();
        let mut board = Board::from_fen(&self.start_fen)?;
        let mut children = &self.children;
        for &index in path {
            let node = children
                .get(index)
                .ok_or_else(|| "study path is stale".to_owned())?;
            let mv = find_uci(&mut board, &node.uci)?;
            board.make_move(mv);
            children = &node.children;
        }
        let mv = find_uci(&mut board, uci)?;
        let san = move_san(&board, mv);
        let uci = mv.to_uci();
        if let Some(existing) = children.iter().position(|node| node.uci == uci) {
            let mut next = path.to_vec();
            next.push(existing);
            return Ok(next);
        }
        let parent = self.node_at_mut(path);
        let siblings = match parent {
            Some(node) => &mut node.children,
            None if path.is_empty() => &mut self.children,
            None => return Err("study path is stale".to_owned()),
        };
        siblings.push(MoveNode {
            uci,
            san,
            ..MoveNode::default()
        });
        let mut next = path.to_vec();
        next.push(siblings.len() - 1);
        Ok(next)
    }

    pub fn set_comment(&mut self, path: &[usize], comment: String) -> Result<(), String> {
        self.node_at_mut(path)
            .ok_or_else(|| "study path is stale".to_owned())?
            .comment = comment;
        Ok(())
    }

    pub fn set_glyphs(&mut self, path: &[usize], glyphs: String) -> Result<(), String> {
        self.node_at_mut(path)
            .ok_or_else(|| "study path is stale".to_owned())?
            .glyphs = glyphs;
        Ok(())
    }

    pub fn to_pgn(&self) -> String {
        let mut out = String::new();
        write_children(&self.children, &mut out, 0, true);
        out
    }

    pub fn from_pgn(start_fen: &str, movetext: &str) -> Result<Self, String> {
        types::init();
        let mut board = Board::from_fen(start_fen)?;
        let tokens = tokenize(movetext)?;
        let mut index = 0;
        let children = parse_siblings(&tokens, &mut index, &mut board)?;
        Ok(Self {
            start_fen: start_fen.to_owned(),
            children,
        })
    }

    pub fn encode(&self) -> Result<String, String> {
        serde_json::to_string(&flatten_tree(self))
            .map_err(|error| format!("failed to encode study tree: {error}"))
    }

    pub fn decode(text: &str) -> Result<Self, String> {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
            && value.get("nodes").is_some()
        {
            let flat = serde_json::from_value::<FlatTree>(value)
                .map_err(|error| format!("failed to decode study tree: {error}"))?;
            return unflatten_tree(flat);
        }
        serde_json::from_str(text).map_err(|error| format!("failed to decode study tree: {error}"))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FlatTree {
    start_fen: String,
    nodes: Vec<FlatNode>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FlatNode {
    parent: Option<u32>,
    uci: String,
    san: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    nag: Option<u8>,
    #[serde(default)]
    glyphs: String,
    #[serde(default)]
    shapes: Vec<String>,
}

fn flatten_tree(tree: &MoveTree) -> FlatTree {
    let mut nodes = Vec::new();
    fn walk(children: &[MoveNode], parent: Option<u32>, nodes: &mut Vec<FlatNode>) {
        for child in children {
            let index = nodes.len() as u32;
            nodes.push(FlatNode {
                parent,
                uci: child.uci.clone(),
                san: child.san.clone(),
                comment: child.comment.clone(),
                nag: child.nag,
                glyphs: child.glyphs.clone(),
                shapes: child.shapes.clone(),
            });
            walk(&child.children, Some(index), nodes);
        }
    }
    walk(&tree.children, None, &mut nodes);
    FlatTree {
        start_fen: tree.start_fen.clone(),
        nodes,
    }
}

fn unflatten_tree(flat: FlatTree) -> Result<MoveTree, String> {
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); flat.nodes.len()];
    let mut roots = Vec::new();
    for (index, node) in flat.nodes.iter().enumerate() {
        match node.parent {
            Some(parent) => {
                let parent = parent as usize;
                if parent >= flat.nodes.len() {
                    return Err("study tree parent is out of range".to_owned());
                }
                children_of[parent].push(index);
            }
            None => roots.push(index),
        }
    }
    fn build(index: usize, flat: &FlatTree, children_of: &[Vec<usize>]) -> MoveNode {
        let node = &flat.nodes[index];
        MoveNode {
            uci: node.uci.clone(),
            san: node.san.clone(),
            comment: node.comment.clone(),
            nag: node.nag,
            glyphs: node.glyphs.clone(),
            shapes: node.shapes.clone(),
            children: children_of[index]
                .iter()
                .map(|&child| build(child, flat, children_of))
                .collect(),
        }
    }
    let children = roots
        .into_iter()
        .map(|index| build(index, &flat, &children_of))
        .collect();
    Ok(MoveTree {
        start_fen: flat.start_fen,
        children,
    })
}

fn write_children(children: &[MoveNode], out: &mut String, ply: usize, first: bool) {
    if children.is_empty() {
        return;
    }
    let main = &children[0];
    if ply.is_multiple_of(2) {
        if !first && !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{}. {}", ply / 2 + 1, main.san));
    } else {
        if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&main.san);
    }
    append_annots(main, out);
    for side in &children[1..] {
        out.push_str(" (");
        if ply % 2 == 1 {
            out.push_str(&format!("{}... ", ply / 2 + 1));
        }
        out.push_str(&side.san);
        append_annots(side, out);
        write_children(&side.children, out, ply + 1, false);
        out.push(')');
    }
    write_children(&main.children, out, ply + 1, false);
}

fn append_annots(node: &MoveNode, out: &mut String) {
    if let Some(nag) = node.nag {
        out.push_str(&format!(" ${nag}"));
    }
    if !node.glyphs.is_empty() && node.nag.is_none() {
        out.push_str(&node.glyphs);
    }
    let mut comment = node.comment.clone();
    if !node.shapes.is_empty() {
        let shapes = format_shape_comment(&node.shapes);
        if !shapes.is_empty() {
            if !comment.is_empty() {
                comment.push(' ');
            }
            comment.push_str(&shapes);
        }
    }
    if !comment.is_empty() {
        out.push_str(&format!(" {{{}}}", comment.replace('}', "\\}")));
    }
}

pub fn extract_comment_shapes(comment: &str) -> (String, Vec<String>) {
    let mut prose = String::new();
    let mut shapes = Vec::new();
    let chars: Vec<char> = comment.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == '%' {
            let start = i;
            i += 2;
            while i < chars.len() && chars[i] != ']' {
                i += 1;
            }
            if i >= chars.len() {
                prose.push_str(&chars[start..].iter().collect::<String>());
                break;
            }
            let body: String = chars[start + 2..i].iter().collect();
            i += 1;
            if let Some(rest) = body.strip_prefix("cal ") {
                for token in rest.split(',') {
                    let token = token.trim();
                    if token.len() >= 5 {
                        shapes.push(format!("cal:{token}"));
                    }
                }
            } else if let Some(rest) = body.strip_prefix("csl ") {
                for token in rest.split(',') {
                    let token = token.trim();
                    if token.len() >= 3 {
                        shapes.push(format!("csl:{token}"));
                    }
                }
            } else {
                if !prose.is_empty() {
                    prose.push(' ');
                }
                prose.push('[');
                prose.push('%');
                prose.push_str(&body);
                prose.push(']');
            }
        } else {
            prose.push(chars[i]);
            i += 1;
        }
    }
    (
        prose.split_whitespace().collect::<Vec<_>>().join(" "),
        shapes,
    )
}

fn format_shape_comment(shapes: &[String]) -> String {
    let mut cals = Vec::new();
    let mut csls = Vec::new();
    for shape in shapes {
        if let Some(token) = shape.strip_prefix("cal:") {
            cals.push(token.to_owned());
        } else if let Some(token) = shape.strip_prefix("csl:") {
            csls.push(token.to_owned());
        }
    }
    let mut out = String::new();
    if !csls.is_empty() {
        out.push_str(&format!("[%csl {}]", csls.join(",")));
    }
    if !cals.is_empty() {
        out.push_str(&format!("[%cal {}]", cals.join(",")));
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Move(String),
    Comment(String),
    Nag(u8),
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        match ch {
            ';' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '{' => {
                i += 1;
                let mut comment = String::new();
                while i < chars.len() && chars[i] != '}' {
                    comment.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated PGN comment".to_owned());
                }
                i += 1;
                tokens.push(Token::Comment(comment.trim().to_owned()));
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '$' => {
                i += 1;
                let mut digits = String::new();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    digits.push(chars[i]);
                    i += 1;
                }
                let nag = digits
                    .parse::<u8>()
                    .map_err(|_| format!("invalid NAG '${digits}'"))?;
                tokens.push(Token::Nag(nag));
            }
            _ => {
                let mut raw = String::new();
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && !matches!(chars[i], '{' | '(' | ')' | ';')
                {
                    raw.push(chars[i]);
                    i += 1;
                }
                let token = strip_move_number(&raw);
                if token.is_empty() || is_result(token) {
                    continue;
                }
                tokens.push(Token::Move(token.to_owned()));
            }
        }
    }
    Ok(tokens)
}

fn parse_siblings(
    tokens: &[Token],
    index: &mut usize,
    board: &mut Board,
) -> Result<Vec<MoveNode>, String> {
    let mut siblings = Vec::new();
    let origin = board.clone();
    while *index < tokens.len() {
        match &tokens[*index] {
            Token::RParen => break,
            Token::LParen => {
                *index += 1;
                let mut fork = origin.clone();
                siblings.extend(parse_siblings(tokens, index, &mut fork)?);
                if *index >= tokens.len() || tokens[*index] != Token::RParen {
                    return Err("unterminated PGN variation".to_owned());
                }
                *index += 1;
            }
            Token::Comment(comment) => {
                attach_comment(siblings.last_mut(), comment);
                *index += 1;
            }
            Token::Nag(nag) => {
                if let Some(last) = siblings.last_mut() {
                    last.nag = Some(*nag);
                    if last.glyphs.is_empty() {
                        last.glyphs = nag_glyphs(*nag).to_owned();
                    }
                }
                *index += 1;
            }
            Token::Move(notation) => {
                let (notation, glyphs) = split_san_glyphs(notation);
                let mut next = origin.clone();
                let mv = resolve_notation(&mut next, notation)
                    .ok_or_else(|| format!("cannot resolve move '{notation}'"))?;
                let mut node = MoveNode {
                    uci: mv.to_uci(),
                    san: move_san(&next, mv),
                    glyphs: glyphs.to_owned(),
                    ..MoveNode::default()
                };
                next.make_move(mv);
                *index += 1;
                while *index < tokens.len() {
                    match &tokens[*index] {
                        Token::Comment(comment) => {
                            attach_comment(Some(&mut node), comment);
                            *index += 1;
                        }
                        Token::Nag(nag) => {
                            node.nag = Some(*nag);
                            if node.glyphs.is_empty() {
                                node.glyphs = nag_glyphs(*nag).to_owned();
                            }
                            *index += 1;
                        }
                        Token::LParen => {
                            *index += 1;
                            let mut fork = origin.clone();
                            siblings.extend(parse_siblings(tokens, index, &mut fork)?);
                            if *index >= tokens.len() || tokens[*index] != Token::RParen {
                                return Err("unterminated PGN variation".to_owned());
                            }
                            *index += 1;
                        }
                        _ => break,
                    }
                }
                node.children = parse_siblings(tokens, index, &mut next)?;
                siblings.insert(0, node);
                *board = next;
                break;
            }
        }
    }
    Ok(siblings)
}

fn attach_comment(node: Option<&mut MoveNode>, comment: &str) {
    let (prose, shapes) = extract_comment_shapes(comment);
    if let Some(node) = node {
        if !prose.is_empty() {
            if node.comment.is_empty() {
                node.comment = prose;
            } else {
                node.comment.push(' ');
                node.comment.push_str(&prose);
            }
        }
        for shape in shapes {
            if !node.shapes.contains(&shape) {
                node.shapes.push(shape);
            }
        }
    }
}

fn find_uci(board: &mut Board, uci: &str) -> Result<Move, String> {
    board
        .generate_legal_moves()
        .iter()
        .copied()
        .find(|mv| mv.to_uci() == uci)
        .ok_or_else(|| format!("illegal study move '{uci}'"))
}

fn resolve_notation(board: &mut Board, notation: &str) -> Option<Move> {
    crate::pgn::resolve_uci(board, notation)
}

fn move_san(board: &Board, mv: Move) -> String {
    crate::pgn::uci_to_san(board, &mv.to_uci())
}

fn strip_move_number(mut token: &str) -> &str {
    if let Some(index) = token.rfind('.')
        && token[..=index]
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    {
        token = &token[index + 1..];
    }
    token
}

fn is_result(token: &str) -> bool {
    matches!(token, "1-0" | "0-1" | "1/2-1/2" | "*")
}

fn split_san_glyphs(token: &str) -> (&str, &str) {
    let end = token.trim_end_matches(['!', '?']).len();
    (&token[..end], &token[end..])
}

fn nag_glyphs(nag: u8) -> &'static str {
    match nag {
        1 => "!",
        2 => "?",
        3 => "!!",
        4 => "??",
        5 => "!?",
        6 => "?!",
        10 => "=",
        13 => "∞",
        14 => "⩲",
        15 => "⩱",
        16 | 18 => "+−",
        17 | 19 => "−+",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainline_round_trips_and_forks_a_sideline() {
        let mut tree = MoveTree::from_mainline(START_FEN, &["e2e4".into(), "e7e5".into()]).unwrap();
        assert_eq!(tree.mainline(), vec!["e2e4", "e7e5"]);
        let path = tree.play_uci(&[0], "c7c5").unwrap();
        assert_eq!(path, vec![0, 1]);
        assert_eq!(tree.children[0].children[1].uci, "c7c5");
        assert_eq!(tree.mainline(), vec!["e2e4", "e7e5"]);
    }

    #[test]
    fn pgn_keeps_variations_and_comments() {
        let tree =
            MoveTree::from_pgn(START_FEN, "1. e4 e5 (1... c5) {Sicilian decline} 2. Nf3").unwrap();
        assert_eq!(tree.children[0].uci, "e2e4");
        assert_eq!(tree.children[0].children[0].uci, "e7e5");
        assert_eq!(tree.children[0].children[1].uci, "c7c5");
        let encoded = tree.to_pgn();
        assert!(encoded.contains("c5"));
        assert!(encoded.contains('('));
    }

    #[test]
    fn json_codec_round_trips_deep_variation_trees() {
        fn deep(depth: usize) -> MoveNode {
            MoveNode {
                uci: "e2e4".into(),
                san: "e4".into(),
                children: if depth == 0 {
                    Vec::new()
                } else {
                    vec![deep(depth - 1)]
                },
                ..MoveNode::default()
            }
        }
        let tree = MoveTree {
            start_fen: START_FEN.to_owned(),
            children: vec![deep(160)],
        };
        let encoded = tree.encode().unwrap();
        let decoded = MoveTree::decode(&encoded).unwrap();
        assert_eq!(decoded.node_at(&vec![0; 161]).unwrap().san, "e4");
    }

    #[test]
    fn json_codec_preserves_glyphs() {
        let mut tree = MoveTree::from_mainline(START_FEN, &["e2e4".into()]).unwrap();
        tree.set_glyphs(&[0], "!!".into()).unwrap();
        tree.set_comment(&[0], "Best by test".into()).unwrap();
        let decoded = MoveTree::decode(&tree.encode().unwrap()).unwrap();
        assert_eq!(decoded.children[0].glyphs, "!!");
        assert_eq!(decoded.children[0].comment, "Best by test");
    }

    #[test]
    fn pgn_keeps_lichess_shapes_and_san_glyphs() {
        let tree = MoveTree::from_pgn(
            START_FEN,
            "1. e4 e5 2. Nf3 Nc6 3. Bb5? { Natural } { [%csl Ge4][%cal Ge4e5] } a6",
        )
        .unwrap();
        let bb5 = &tree.children[0].children[0].children[0].children[0].children[0];
        assert_eq!(bb5.san, "Bb5");
        assert_eq!(bb5.glyphs, "?");
        assert!(bb5.comment.contains("Natural"));
        assert!(bb5.shapes.iter().any(|shape| shape == "csl:Ge4"));
        assert!(bb5.shapes.iter().any(|shape| shape == "cal:Ge4e5"));
        let encoded = tree.to_pgn();
        assert!(encoded.contains("[%csl Ge4]"));
        assert!(encoded.contains("[%cal Ge4e5]"));
    }

    #[test]
    fn pgn_accepts_over_disambiguated_knight_san() {
        let tree = MoveTree::from_pgn(
            START_FEN,
            "1. e4 e5 2. Nc3 Nc6 3. Bc4 Bc5 4. Qg4 g6 5. Qf3 Nf6 6. Nge2",
        )
        .unwrap();
        let last = tree
            .node_at(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            .expect("Nge2");
        assert_eq!(last.uci, "g1e2");
        assert!(last.san == "Nge2" || last.san == "Ne2");
    }
}
