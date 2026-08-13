//! Parsed SVG trees shared by board pieces, annotation badges, and chrome icons.

use std::cell::RefCell;
use std::collections::HashMap;

use floem::Renderer;
use floem::kurbo::Rect;

thread_local! {
    static TREES: RefCell<HashMap<usize, usvg::Tree>> = RefCell::new(HashMap::new());
}

pub fn draw(cx: &mut floem::context::PaintCx<'_>, svg: &str, rect: Rect, hash: &[u8]) {
    if svg.is_empty() {
        return;
    }
    let key = svg.as_ptr() as usize;
    TREES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&key)
            && let Ok(tree) = usvg::Tree::from_str(svg, &usvg::Options::default())
        {
            cache.insert(key, tree);
        }
        if let Some(tree) = cache.get(&key) {
            cx.draw_svg(
                floem::RendererSvg { tree, hash },
                rect,
                None::<&floem::peniko::Brush>,
            );
        }
    });
}

#[cfg(test)]
pub fn parse_ok(svg: &str) -> bool {
    if svg.is_empty() {
        return false;
    }
    let key = svg.as_ptr() as usize;
    TREES.with(|cache| {
        if cache.borrow().contains_key(&key) {
            return true;
        }
        match usvg::Tree::from_str(svg, &usvg::Options::default()) {
            Ok(tree) => {
                cache.borrow_mut().insert(key, tree);
                true
            }
            Err(_) => false,
        }
    })
}

#[cfg(test)]
pub fn cached(svg: &str) -> bool {
    let key = svg.as_ptr() as usize;
    TREES.with(|cache| cache.borrow().contains_key(&key))
}

#[cfg(test)]
mod tests {
    use super::super::icons;
    use super::*;

    #[test]
    fn chrome_icons_parse_into_the_shared_cache() {
        for icon in icons::ALL {
            assert!(parse_ok(icon), "unparseable lucide icon");
            assert!(cached(icon));
            assert!(parse_ok(icon));
        }
    }
}
