//! Reads the answer key of hotspot questions from the answer picture.
//!
//! ExamTopics marks the chosen option of each dropdown in the picture: a green fill,
//! or a red or black outline. When the question shows the same picture without the
//! marks, the difference between the two finds any kind of mark; otherwise coloured
//! marks are found by colour. The option is then identified by counting the lines of
//! text above the mark in the dropdown list, because OCR positions are too noisy.

use crate::model::DropdownGroup;

/// An RGBA picture (4 bytes per pixel, rows top to bottom).
pub struct Image<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl Image<'_> {
    fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    fn luma(&self, x: usize, y: usize) -> u32 {
        let [r, g, b] = self.px(x, y);
        (r as u32 * 30 + g as u32 * 59 + b as u32 * 11) / 100
    }

    fn valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.rgba.len() >= self.width * self.height * 4
    }
}

/// A box in pixels; `x1` / `y1` are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Mark {
    fn width(&self) -> usize {
        self.x1 - self.x0
    }

    fn height(&self) -> usize {
        self.y1 - self.y0
    }

    fn touches(&self, o: &Mark, gap: usize) -> bool {
        self.x0 <= o.x1 + gap
            && o.x0 <= self.x1 + gap
            && self.y0 <= o.y1 + gap
            && o.y0 <= self.y1 + gap
    }

    fn union(&self, o: &Mark) -> Mark {
        Mark {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

fn is_mark_colour([r, g, b]: [u8; 3]) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let green = g > r + 14 && g > b + 8 && g > 110;
    let red = r > g + 70 && r > b + 70;
    green || red
}

/// Marks in the answer picture, top to bottom. `question` is the unmarked picture,
/// when the question shows the same picture (at any scale).
pub fn find_marks(answer: &Image, question: Option<&Image>) -> Vec<Mark> {
    if !answer.valid() {
        return Vec::new();
    }
    let (w, h) = (answer.width, answer.height);
    // Coloured marks are unambiguous.
    let mut mask: Vec<bool> = (0..w * h)
        .map(|i| is_mark_colour(answer.px(i % w, i / w)))
        .collect();
    let mut marks = mark_boxes(&mask, w, h);
    if marks.is_empty()
        && let Some(q) = question.filter(|q| q.valid() && same_shape(answer, q))
    {
        // Anything else that was drawn on top: compare with the unmarked picture,
        // scaled to the same size. Resampling blurs edges, so only strong changes count
        // and thin ones are eroded away.
        for y in 0..h {
            let qy = (y * q.height / h).min(q.height - 1);
            for x in 0..w {
                let qx = (x * q.width / w).min(q.width - 1);
                let a = answer.px(x, y);
                let b = q.px(qx, qy);
                let d: i32 = (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).sum();
                mask[y * w + x] = d > 150;
            }
        }
        marks = mark_boxes(&erode(&mask, w, h), w, h);
    }
    if marks.is_empty() {
        marks = outlines(answer);
    }
    marks
}

/// Thick dark outlines drawn around options. Text strokes and one-pixel borders
/// disappear when the dark pixels are eroded; a filled bar (a dropdown's header) is
/// not hollow.
fn outlines(img: &Image) -> Vec<Mark> {
    let (w, h) = (img.width, img.height);
    let dark: Vec<bool> = (0..w * h).map(|i| img.luma(i % w, i / w) < 70).collect();
    let thick = erode(&dark, w, h);
    mark_boxes(&thick, w, h)
        .into_iter()
        .filter(|m| {
            let inner = Mark {
                x0: m.x0 + m.width() / 4,
                x1: m.x1 - m.width() / 4,
                y0: m.y0 + m.height() / 3,
                y1: m.y1 - m.height() / 3,
            };
            let filled = (inner.y0..inner.y1)
                .flat_map(|y| (inner.x0..inner.x1).map(move |x| (x, y)))
                .filter(|&(x, y)| thick[y * w + x])
                .count();
            filled * 4 < inner.width() * inner.height()
        })
        .collect()
}

/// Whether two pictures have the same aspect ratio (the same picture at another scale).
fn same_shape(a: &Image, b: &Image) -> bool {
    let ra = a.width as f32 / a.height as f32;
    let rb = b.width as f32 / b.height as f32;
    (ra - rb).abs() / ra < 0.03
}

/// Keeps pixels whose 3×3 neighbourhood is fully set: removes one-pixel edges and specks.
fn erode(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            out[y * w + x] = (y - 1..=y + 1).all(|yy| (x - 1..=x + 1).all(|xx| mask[yy * w + xx]));
        }
    }
    out
}

/// Row-shaped boxes around the set pixels.
fn mark_boxes(mask: &[bool], w: usize, h: usize) -> Vec<Mark> {
    let mut parts = components(mask, w, h);
    // An outline is one component; fills with text inside can break into a few.
    let mut merged: Vec<Mark> = Vec::new();
    parts.sort_by_key(|m| (m.y0, m.x0));
    for m in parts {
        match merged.iter_mut().find(|o| o.touches(&m, 4)) {
            Some(o) => *o = o.union(&m),
            None => merged.push(m),
        }
    }
    let mut out: Vec<Mark> = merged
        .into_iter()
        // Erosion shrank everything by a pixel.
        .map(|m| Mark {
            x0: m.x0.saturating_sub(1),
            y0: m.y0.saturating_sub(1),
            x1: (m.x1 + 1).min(w),
            y1: (m.y1 + 1).min(h),
        })
        // Options can be as short as "3", but never taller than wide.
        .filter(|m| m.height() >= 10 && m.height() <= 90 && m.width() >= m.height())
        .collect();
    out.sort_by_key(|m| (m.y0, m.x0));
    out
}

/// Bounding boxes of 4-connected regions of at least a few pixels.
fn components(mask: &[bool], w: usize, h: usize) -> Vec<Mark> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut m = Mark {
            x0: usize::MAX,
            y0: usize::MAX,
            x1: 0,
            y1: 0,
        };
        let mut count = 0;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            count += 1;
            m.x0 = m.x0.min(x);
            m.y0 = m.y0.min(y);
            m.x1 = m.x1.max(x + 1);
            m.y1 = m.y1.max(y + 1);
            let mut visit = |j: usize| {
                if mask[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if count >= 12 {
            out.push(m);
        }
    }
    out
}

/// Where the marked option sits in its dropdown list: (index, number of options).
///
/// The list is the block of text lines around the mark, bounded by the empty
/// dropdown box above it and the space below it. Borders, separators and the mark
/// itself span the whole width and are not text.
pub fn list_position(img: &Image, mark: Mark) -> Option<(usize, usize)> {
    let x0 = (mark.x0 + 4).min(img.width);
    let x1 = (mark.x0 + mark.width() * 3 / 5).min(img.width);
    if x1 <= x0 + 4 {
        return None;
    }
    let band = x1 - x0;
    let ink = |y: usize| {
        let dark = (x0..x1).filter(|&x| img.luma(x, y) < 150).count();
        dark >= 2 && dark * 2 < band
    };
    let stop_gap = (mark.height() * 7 / 10).max(5);
    let centre = (mark.y0 + mark.y1) / 2;
    let reach = mark.height() * 14;
    let edge = |ys: &mut dyn Iterator<Item = usize>| {
        let mut last = centre;
        let mut blank = 0;
        for y in ys {
            if ink(y) {
                blank = 0;
                last = y;
            } else {
                blank += 1;
                if blank >= stop_gap {
                    break;
                }
            }
        }
        last
    };
    let top = edge(&mut (centre.saturating_sub(reach)..centre).rev());
    let bottom = edge(&mut (centre..(centre + reach).min(img.height)));

    // Lines of text: runs of ink rows. Rows with little ink (the tops of capitals)
    // don't end a line, and a line is taller than a row or two of JPEG noise.
    let line_gap = (mark.height() / 6).max(2);
    let min_text = (mark.height() / 5).max(3);
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for y in top..=bottom {
        match (&mut run, ink(y)) {
            (Some((_, end)), true) => *end = y,
            (Some((start, end)), false) if y - *end >= line_gap => {
                if *end + 1 - *start >= min_text {
                    lines.push((*start, *end));
                }
                run = None;
            }
            (None, true) => run = Some((y, y)),
            _ => {}
        }
    }
    if let Some((start, end)) = run
        && end + 1 - start >= min_text
    {
        lines.push((start, end));
    }
    let index = lines
        .iter()
        .position(|&(a, b)| a <= centre && centre <= b + line_gap)?;
    Some((index, lines.len()))
}

/// Sets the keys of `groups` from the marks in the answer picture. Returns `false`
/// (leaving `groups` unchanged) unless there is exactly one usable mark per group.
pub fn key_dropdowns(
    groups: &mut [DropdownGroup],
    answer: &Image,
    question: Option<&Image>,
) -> bool {
    let marks = find_marks(answer, question);
    if marks.len() != groups.len() {
        return false;
    }
    // The list must have as many rows as OCR found options, or an option is missing
    // and the index would point at the wrong one.
    let keys: Option<Vec<usize>> = groups
        .iter()
        .zip(&marks)
        .map(|(g, m)| match list_position(answer, *m) {
            Some((k, n)) if n == g.options.len() => Some(k),
            _ => None,
        })
        .collect();
    let Some(keys) = keys else {
        return false;
    };
    for (g, k) in groups.iter_mut().zip(keys) {
        g.correct = Some(k);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canvas {
        w: usize,
        h: usize,
        px: Vec<u8>,
    }

    impl Canvas {
        fn new(w: usize, h: usize) -> Self {
            Canvas {
                w,
                h,
                px: vec![255; w * h * 4],
            }
        }

        fn fill(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, c: [u8; 3]) {
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * self.w + x) * 4;
                    self.px[i..i + 3].copy_from_slice(&c);
                }
            }
        }

        /// A fake line of text: short dark strokes.
        fn text(&mut self, x: usize, y: usize) {
            for k in 0..12 {
                self.fill(x + k * 6, y, x + k * 6 + 3, y + 9, [20, 20, 20]);
            }
        }

        fn image(&self) -> Image<'_> {
            Image {
                width: self.w,
                height: self.h,
                rgba: &self.px,
            }
        }
    }

    /// A dropdown: an empty box with an arrow, then a bordered list of rows 22 px high.
    fn dropdown(c: &mut Canvas, top: usize, options: usize) {
        c.fill(100, top, 300, top + 1, [90, 90, 90]);
        c.fill(100, top + 22, 300, top + 23, [90, 90, 90]);
        c.fill(280, top + 8, 290, top + 14, [10, 10, 10]);
        for i in 0..options {
            let y = top + 24 + i * 22;
            c.fill(100, y + 21, 300, y + 22, [210, 210, 210]);
            c.text(104, y + 6);
        }
    }

    #[test]
    fn finds_green_marks_and_counts_rows() {
        let mut c = Canvas::new(400, 300);
        dropdown(&mut c, 10, 4);
        dropdown(&mut c, 150, 4);
        let unmarked = Canvas {
            px: c.px.clone(),
            ..c
        };
        // Mark option 0 of the first list and option 2 of the second.
        for (top, opt) in [(10, 0), (150, 2)] {
            let y = top + 24 + opt * 22;
            c.fill(101, y, 299, y + 20, [198, 239, 206]);
            c.text(104, y + 6);
        }
        let marks = find_marks(&c.image(), None);
        assert_eq!(marks.len(), 2);
        assert_eq!(list_position(&c.image(), marks[0]), Some((0, 4)));
        assert_eq!(list_position(&c.image(), marks[1]), Some((2, 4)));

        let mut groups = vec![
            DropdownGroup {
                label: "a".into(),
                options: vec!["1".into(), "2".into(), "3".into(), "4".into()],
                correct: None,
            };
            2
        ];
        assert!(key_dropdowns(
            &mut groups,
            &c.image(),
            Some(&unmarked.image())
        ));
        assert_eq!(groups[0].correct, Some(0));
        assert_eq!(groups[1].correct, Some(2));
    }

    #[test]
    fn finds_black_outlines_by_difference() {
        let mut c = Canvas::new(400, 200);
        dropdown(&mut c, 10, 3);
        let unmarked = Canvas {
            px: c.px.clone(),
            ..c
        };
        let y = 10 + 24 + 22;
        // A 4 px black ring around option 1.
        c.fill(96, y - 4, 304, y, [0, 0, 0]);
        c.fill(96, y + 20, 304, y + 24, [0, 0, 0]);
        c.fill(96, y, 100, y + 20, [0, 0, 0]);
        c.fill(300, y, 304, y + 20, [0, 0, 0]);
        let marks = find_marks(&c.image(), Some(&unmarked.image()));
        assert_eq!(marks.len(), 1);
        assert_eq!(list_position(&c.image(), marks[0]), Some((1, 3)));
        // Without the unmarked picture, the outline is found by its shape.
        let marks = find_marks(&c.image(), None);
        assert_eq!(marks.len(), 1);
        assert_eq!(list_position(&c.image(), marks[0]), Some((1, 3)));
    }
}
