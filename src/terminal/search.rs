//! Read-only paged search of interpreted output, using stable history rows.
use super::*;
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Hit {
    pub row: u64,
    pub text: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Page {
    pub hits: Vec<Hit>,
    pub next: u64,
    pub end: u64,
}
impl Terminal {
    pub fn search_output(&self, from: u64, query: &str) -> Page {
        let history_len = if self.primary.is_none() {
            self.history.len()
        } else {
            0
        };
        let base = self.history_base;
        let end = base.saturating_add((history_len + self.grid.rows) as u64);
        let start = from.clamp(base, end);
        let next = start.saturating_add(512).min(end);
        let query = query.to_lowercase();
        let mut hits = Vec::new();
        for row in start..next {
            let index = (row - base) as usize;
            let text = if index < history_len {
                self.history.get(index).unwrap().text()
            } else {
                self.grid.line(index - history_len)
            };
            if !query.is_empty() && text.to_lowercase().contains(&query) {
                hits.push(Hit {
                    row,
                    text: crate::wire::passive(&text),
                });
            }
        }
        Page { hits, next, end }
    }
}
