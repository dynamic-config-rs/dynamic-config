//! Why a value is what it is — every layer's answer, not just the winner's.
//!
//! [`source_of`](crate::source_of) answers *which layer won*. The question a
//! production incident actually asks is *why*: what did every layer have to
//! say, and who beat whom. An [`Explanation`] is that table.
//!
//! Unlike every other diagnostic in this crate, an explanation **contains
//! values** — that is its point; you asked. Fields marked
//! `#[config(secret)]` stay `***` in the generated `explain()`, and
//! [`Explanation::redacted`] blanks all of them for a path the caller knows
//! to be sensitive.

use std::fmt;

use figment::value::Value;

use crate::error::{Error, Origin};
use crate::source::LoadSpec;

/// One layer's answer for one path.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Contribution {
    /// The layer, by its name in the precedence order.
    pub layer: &'static str,
    /// Where within the layer the value came from.
    ///
    /// `None` when the layer supplies nothing at this path.
    pub origin: Option<Origin>,
    /// The value, rendered short — `None` when the layer supplies nothing,
    /// `***` when redacted. Tables and lists render as their shape
    /// (`a table (3 keys)`), not their contents.
    pub value: Option<String>,
}

/// Every configured layer's answer for one path, lowest precedence first.
///
/// Produced by [`explain`](crate::explain) or a generated `explain()`.
/// `Display` renders the table; the rows are public for anything that wants
/// to format its own.
#[derive(Debug, Clone)]
pub struct Explanation {
    path: String,
    rows: Vec<Contribution>,
}

impl Explanation {
    pub(crate) fn new(path: String, rows: Vec<Contribution>) -> Self {
        Self { path, rows }
    }

    /// The path this explains.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Every configured layer's row, lowest precedence first.
    #[must_use]
    pub fn rows(&self) -> &[Contribution] {
        &self.rows
    }

    /// The winning contribution — the highest-precedence layer that supplies
    /// anything — or `None` when nothing does.
    #[must_use]
    pub fn winner(&self) -> Option<&Contribution> {
        self.rows.iter().rev().find(|row| row.value.is_some())
    }

    /// This explanation with every value replaced by `***`.
    ///
    /// The origins stay: *where* a secret comes from is the useful half, and
    /// it is exactly the half that is safe to show.
    #[must_use]
    pub fn redacted(mut self) -> Self {
        for row in &mut self.rows {
            if row.value.is_some() {
                row.value = Some("***".to_owned());
            }
        }

        self
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.winner() {
            Some(winner) => writeln!(
                f,
                "{} = {}",
                self.path,
                winner.value.as_deref().unwrap_or("***")
            )?,
            None => writeln!(f, "{}: nothing supplies it", self.path)?,
        }
        writeln!(f)?;

        let sources: Vec<String> = self
            .rows
            .iter()
            .map(|row| {
                row.origin
                    .as_ref()
                    .map_or_else(|| "—".to_owned(), Origin::to_string)
            })
            .collect();

        let layer_width = self
            .rows
            .iter()
            .map(|row| row.layer.len())
            .chain(["layer".len()])
            .max()
            .unwrap_or(0);
        let source_width = sources
            .iter()
            .map(String::len)
            .chain(["source".len()])
            .max()
            .unwrap_or(0);

        writeln!(
            f,
            "{:layer_width$}  {:source_width$}  value",
            "layer", "source"
        )?;

        let winner_at = self.rows.iter().rposition(|row| row.value.is_some());

        for (index, (row, source)) in self.rows.iter().zip(&sources).enumerate() {
            let value = row.value.as_deref().unwrap_or("absent");
            let marker = if Some(index) == winner_at {
                "   ← winner"
            } else {
                ""
            };

            writeln!(
                f,
                "{:layer_width$}  {source:source_width$}  {value}{marker}",
                row.layer
            )?;
        }

        Ok(())
    }
}

/// Explains `path`: every configured layer's answer, lowest precedence first.
///
/// Reads every source once per layer, the same way a load would. A layer the
/// spec does not configure gets no row — a table of nine `absent`s would
/// bury the answer.
pub(crate) fn explain(spec: &LoadSpec<'_>, path: &str) -> Result<Explanation, Error> {
    let mut rows = Vec::new();

    for (name, figment) in crate::loader::layer_figments(spec)? {
        let value = figment.find_value(path).ok();
        let origin = value
            .is_some()
            .then(|| crate::loader::origin_in(&figment, path));

        rows.push(Contribution {
            layer: name,
            origin,
            value: value.map(|value| render(&value)),
        });
    }

    // Aliases are a gap-fill, not a layer: a value that only an alias
    // supplies shows up in no per-layer probe. When that happens, the
    // composed load is asked directly and the answer gets a row of its own.
    if rows.iter().all(|row| row.value.is_none()) {
        let merged = crate::loader::merged(spec)?;

        if let Ok(value) = merged.find_value(path) {
            let origin = crate::loader::origin_in(&merged, path);

            rows.push(Contribution {
                layer: "alias",
                origin: Some(origin),
                value: Some(render(&value)),
            });
        }
    }

    Ok(Explanation::new(path.to_owned(), rows))
}

/// A short, single-line rendering — the value for scalars, the shape for
/// containers.
fn render(value: &Value) -> String {
    match value {
        Value::String(_, string) => string.clone(),
        Value::Char(_, character) => character.to_string(),
        Value::Bool(_, boolean) => boolean.to_string(),
        Value::Num(_, number) => number
            .to_i128()
            .map(|whole| whole.to_string())
            .or_else(|| number.to_u128().map(|whole| whole.to_string()))
            .or_else(|| number.to_f64().map(|float| float.to_string()))
            .unwrap_or_else(|| "a number".to_owned()),
        Value::Empty(..) => "null".to_owned(),
        Value::Dict(_, table) => format!("a table ({} keys)", table.len()),
        Value::Array(_, items) => format!("a list ({} items)", items.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(layer: &'static str, value: Option<&str>) -> Contribution {
        Contribution {
            layer,
            origin: value.map(|_| Origin::Runtime("default")),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn the_winner_is_the_highest_layer_that_supplies_anything() {
        let explanation = Explanation::new(
            "port".to_owned(),
            vec![
                row("default", Some("3000")),
                row("file", Some("8080")),
                row("environment", None),
            ],
        );

        assert_eq!(explanation.winner().unwrap().layer, "file");
    }

    #[test]
    fn redaction_blanks_values_and_keeps_origins() {
        let explanation = Explanation::new(
            "token".to_owned(),
            vec![row("file", Some("hunter2")), row("environment", None)],
        )
        .redacted();

        let rendered = explanation.to_string();

        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("***"), "{rendered}");
        assert!(explanation.rows()[0].origin.is_some());
        assert_eq!(explanation.rows()[1].value, None, "absent stays absent");
    }

    #[test]
    fn the_table_marks_the_winner() {
        let explanation = Explanation::new(
            "port".to_owned(),
            vec![row("default", Some("3000")), row("file", Some("8080"))],
        );

        let rendered = explanation.to_string();
        let winner_line = rendered
            .lines()
            .find(|line| line.contains("← winner"))
            .expect("one row is marked");

        assert!(winner_line.starts_with("file"), "{rendered}");
        assert!(rendered.starts_with("port = 8080"), "{rendered}");
    }
}
