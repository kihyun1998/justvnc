use justvnc_proto::Rect;

/// Rectangles of the framebuffer that changed and have not been sent, none overlapping.
#[derive(Debug, Default)]
pub(crate) struct Region(Vec<Rect>);

/// Past this many rectangles, the region becomes the one rectangle that holds them all.
const MAX_RECTANGLES: usize = 64;

impl Region {
    pub(crate) fn add(&mut self, rect: Rect) {
        let mut pieces = vec![rect];
        for old in &self.0 {
            pieces = pieces.into_iter().flat_map(|p| subtract(p, *old)).collect();
        }
        self.0.extend(pieces);
        if self.0.len() > MAX_RECTANGLES {
            let bounds = self.0.iter().copied().reduce(union).expect("not empty");
            self.0 = vec![bounds];
        }
    }

    /// Whether any of it lies inside `rect`.
    pub(crate) fn intersects(&self, rect: Rect) -> bool {
        self.0.iter().any(|r| intersect(*r, rect).is_some())
    }

    /// Removes and returns the parts inside `area`.
    pub(crate) fn take(&mut self, area: Rect) -> Vec<Rect> {
        let inside = self.0.iter().filter_map(|r| intersect(*r, area)).collect();
        self.0 = self.0.iter().flat_map(|r| subtract(*r, area)).collect();
        inside
    }
}

/// The part of `rect` inside a `width` x `height` framebuffer, if any.
pub(crate) fn clip(rect: Rect, (width, height): (u16, u16)) -> Option<Rect> {
    intersect(
        rect,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
}

fn end(rect: Rect) -> (u32, u32) {
    (
        u32::from(rect.x) + u32::from(rect.width),
        u32::from(rect.y) + u32::from(rect.height),
    )
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let ((ax2, ay2), (bx2, by2)) = (end(a), end(b));
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (x2, y2) = (ax2.min(bx2), ay2.min(by2));
    (u32::from(x) < x2 && u32::from(y) < y2).then(|| Rect {
        x,
        y,
        width: (x2 - u32::from(x)) as u16,
        height: (y2 - u32::from(y)) as u16,
    })
}

/// `a` without `b`: up to four rectangles, above, below, left and right of `b`.
fn subtract(a: Rect, b: Rect) -> Vec<Rect> {
    let Some(cut) = intersect(a, b) else {
        return vec![a];
    };
    let ((ax2, ay2), (cx2, cy2)) = (end(a), end(cut));
    let rect = |x: u32, y: u32, x2: u32, y2: u32| {
        (x < x2 && y < y2).then(|| Rect {
            x: x as u16,
            y: y as u16,
            width: (x2 - x) as u16,
            height: (y2 - y) as u16,
        })
    };
    let (ax, ay, cx, cy) = (
        u32::from(a.x),
        u32::from(a.y),
        u32::from(cut.x),
        u32::from(cut.y),
    );
    [
        rect(ax, ay, ax2, cy),
        rect(ax, cy2, ax2, ay2),
        rect(ax, cy, cx, cy2),
        rect(cx2, cy, ax2, cy2),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn union(a: Rect, b: Rect) -> Rect {
    let ((ax2, ay2), (bx2, by2)) = (end(a), end(b));
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect {
        x,
        y,
        width: (ax2.max(bx2) - u32::from(x)) as u16,
        height: (ay2.max(by2) - u32::from(y)) as u16,
    }
}
