//! Shared two-pane geometry. Groups own tab identities; they never own processes.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SplitAxis {
    Right,
    Below,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneGroup {
    pub tabs: Vec<u64>,
    pub selected: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneLayout {
    pub axis: SplitAxis,
    pub ratio: u16,
    pub zoomed: bool,
    pub focused: u8,
    pub revision: u64,
    pub groups: [PaneGroup; 2],
}
impl PaneLayout {
    pub fn validate(&self, tabs: &[u64]) -> std::io::Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        if !(200..=800).contains(&self.ratio)
            || self.focused > 1
            || self.revision == 0
            || self.groups.iter().any(|g| {
                g.tabs.is_empty()
                    || g.tabs.len() > 512
                    || !g.tabs.contains(&g.selected)
                    || g.tabs
                        .iter()
                        .any(|id| *id == 0 || !tabs.contains(id) || !seen.insert(*id))
            })
            || seen.len() != tabs.len()
        {
            return Err(crate::wire::invalid("invalid pane group layout"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneRect {
    pub x: usize,
    pub y: usize,
    pub cols: usize,
    pub rows: usize,
}
/// Coordinates are relative to the original terminal rectangle. A below split
/// reserves a divider plus the second group's tab bar and separator.
pub fn geometry(
    cols: usize,
    rows: usize,
    axis: SplitAxis,
    ratio: u16,
    zoomed: bool,
    focused: u8,
) -> [Option<PaneRect>; 2] {
    let full = PaneRect {
        x: 0,
        y: 0,
        cols,
        rows,
    };
    let available = match axis {
        SplitAxis::Right => cols.saturating_sub(1),
        SplitAxis::Below => rows.saturating_sub(3),
    };
    let minimum = match axis {
        SplitAxis::Right => 18,
        SplitAxis::Below => 4,
    };
    if zoomed || available < minimum * 2 {
        let mut panes = [None, None];
        panes[usize::from(focused.min(1))] = Some(full);
        return panes;
    }
    let first =
        (available * usize::from(ratio.clamp(200, 800)) / 1000).clamp(minimum, available - minimum);
    match axis {
        SplitAxis::Right => [
            Some(PaneRect {
                cols: first,
                ..full
            }),
            Some(PaneRect {
                x: first + 1,
                cols: available - first,
                ..full
            }),
        ],
        SplitAxis::Below => [
            Some(PaneRect {
                rows: first,
                ..full
            }),
            Some(PaneRect {
                y: first + 3,
                rows: available - first,
                ..full
            }),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_geometry_tiles_the_viewport_and_keeps_useful_minimums() {
        for cols in 2..=240 {
            for ratio in [200, 300, 500, 700, 800] {
                let panes = geometry(cols, 31, SplitAxis::Right, ratio, false, 1);
                if cols < 37 {
                    assert!(panes[0].is_none());
                    assert_eq!(
                        panes[1].unwrap(),
                        PaneRect {
                            x: 0,
                            y: 0,
                            cols,
                            rows: 31
                        }
                    );
                } else {
                    let (a, b) = (panes[0].unwrap(), panes[1].unwrap());
                    assert!(a.cols >= 18 && b.cols >= 18);
                    assert_eq!(a.cols + 1 + b.cols, cols);
                    assert_eq!(b.x, a.cols + 1);
                    assert_eq!((a.rows, b.rows), (31, 31));
                }
            }
        }
        for rows in 2..=100 {
            let panes = geometry(101, rows, SplitAxis::Below, 300, false, 0);
            if rows < 11 {
                assert!(panes[1].is_none());
                assert_eq!(panes[0].unwrap().rows, rows);
            } else {
                let (a, b) = (panes[0].unwrap(), panes[1].unwrap());
                assert!(a.rows >= 4 && b.rows >= 4);
                assert_eq!(a.rows + 3 + b.rows, rows);
                assert_eq!(b.y, a.rows + 3);
                assert_eq!((a.cols, b.cols), (101, 101));
            }
        }
        for focused in 0..=1 {
            let panes = geometry(101, 31, SplitAxis::Right, 300, true, focused);
            assert_eq!(
                panes[usize::from(focused)].unwrap(),
                PaneRect {
                    x: 0,
                    y: 0,
                    cols: 101,
                    rows: 31
                }
            );
            assert!(panes[usize::from(1 - focused)].is_none());
        }
    }
}
