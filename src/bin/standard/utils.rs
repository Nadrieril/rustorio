use itertools::Itertools;
use std::{fmt::Display, sync::Arc};

/// Format a list of rows of equal length into a columned layout.
pub fn format_in_columns<const N: usize>(rows: &[[String; N]]) -> impl Display {
    let column_widths = Arc::new(
        (0..N)
            .map(|i| rows.iter().map(|r| r[i].len()).max().unwrap())
            .collect_vec(),
    );
    rows.iter()
        .map(move |r| {
            r.iter()
                .enumerate()
                .map({
                    let column_widths = column_widths.clone();
                    move |(i, cell)| format!("{cell:w$}", w = column_widths[i])
                })
                .format("")
        })
        .format("\n")
}
