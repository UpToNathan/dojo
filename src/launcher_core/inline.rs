//! Inline rows: results an explicit trigger in the query produces (a calculation, a bang, a
//! `?` prompt). Engines add their provider here; ordinary text produces nothing.

use super::rows::Row;

/// Rows for `query`, in the order they should lead the list.
pub fn inline_rows(query: &str) -> Vec<Row> {
    let _ = query;
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_produces_no_inline_rows() {
        assert!(inline_rows("firefox").is_empty());
        assert!(inline_rows("").is_empty());
    }
}
