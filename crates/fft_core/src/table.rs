//! Data tables (csv, tsv, parquet, json, ...) as a signal source.
//!
//! Files are read into a small [`TableData`] model: numeric columns keep their
//! values (with missing entries marked) and everything else is text. The reader
//! is feature-gated so a text-only build does not pull in the binary formats:
//!
//! * `tables-text` (default): delimited text, JSON, and JSON Lines;
//! * `tables-parquet`: Parquet;
//! * `tables-arrow`: Arrow IPC / Feather.
//!
//! Picking the signal column, filling gaps, deriving the sample rate, and
//! removing the mean all happen in [`TableData::into_signal`].

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::DataSettings;
use crate::traits::{Error, Result};

/// File name extensions that are read as data tables (the file dialog's second filter).
pub const TABLE_EXTENSIONS: &[&str] = &[
    "csv", "tsv", "tab", "txt", "dat", "parquet", "pq", "json", "ndjson", "jsonl", "arrow", "ipc",
    "feather",
];

/// Column names that are taken for the time axis when none is chosen.
const TIME_NAMES: &[&str] = &["time", "time_s", "t", "seconds", "sec", "timestamp"];

/// How a table file is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableFormat {
    /// Delimited text with a header row.
    ///
    /// `separator` is used for non-standard extensions. `.tsv`/`.tab` always
    /// force a tab, while `.csv`/`.txt`/`.dat` sniff `,`, tab, `;`, `|`.
    Csv {
        separator: u8,
    },
    Parquet,
    /// One JSON array of row objects.
    Json,
    /// One JSON object per line.
    JsonLines,
    /// Arrow IPC / Feather.
    Ipc,
}

impl TableFormat {
    /// The table format a file name stands for, `None` for anything else
    /// (audio, or unknown: those go to the audio decoder).
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "csv" | "txt" | "dat" => TableFormat::Csv { separator: b',' },
            "tsv" | "tab" => TableFormat::Csv { separator: b'\t' },
            "parquet" | "pq" => TableFormat::Parquet,
            "json" => TableFormat::Json,
            "ndjson" | "jsonl" => TableFormat::JsonLines,
            "arrow" | "ipc" | "feather" => TableFormat::Ipc,
            _ => return None,
        })
    }

    /// Stable capability name for this format family.
    ///
    /// Delimited text is reported as `csv` regardless of the separator bytes;
    /// this name is for API/capability transport, not lossless round-tripping
    /// of a parsed file format. Use [`TableFormat::from_path`] when the actual
    /// separator matters.
    pub fn kind_name(self) -> &'static str {
        match self {
            TableFormat::Csv { .. } => "csv",
            TableFormat::Parquet => "parquet",
            TableFormat::Json => "json",
            TableFormat::JsonLines => "jsonl",
            TableFormat::Ipc => "arrow",
        }
    }

    /// Inverse of [`TableFormat::kind_name`] for capability payloads.
    ///
    /// `csv` maps back to comma-separated text; the capability name does not
    /// carry the original separator.
    pub fn from_kind_name(name: &str) -> Option<Self> {
        Some(match name {
            "csv" => TableFormat::Csv { separator: b',' },
            "parquet" => TableFormat::Parquet,
            "json" => TableFormat::Json,
            "jsonl" => TableFormat::JsonLines,
            "arrow" => TableFormat::Ipc,
            _ => return None,
        })
    }
}

impl Serialize for TableFormat {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.kind_name())
    }
}

impl<'de> Deserialize<'de> for TableFormat {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        TableFormat::from_kind_name(&name).ok_or_else(|| {
            serde::de::Error::unknown_variant(&name, &["csv", "parquet", "json", "jsonl", "arrow"])
        })
    }
}

/// Extensions available in this build, for the desktop file dialog and
/// capability reporting.
///
/// Order is the same as [`TABLE_EXTENSIONS`] so the UI does not jump around
/// between builds.
pub fn available_table_extensions() -> Vec<&'static str> {
    TABLE_EXTENSIONS
        .iter()
        .copied()
        .filter(|ext| {
            let name = format!("data.{ext}");
            TableFormat::from_path(Path::new(&name)).is_some_and(format_available)
        })
        .collect()
}

fn format_available(format: TableFormat) -> bool {
    match format {
        TableFormat::Csv { .. } | TableFormat::Json | TableFormat::JsonLines => {
            cfg!(feature = "tables-text")
        }
        TableFormat::Parquet => cfg!(feature = "tables-parquet"),
        TableFormat::Ipc => cfg!(feature = "tables-arrow"),
    }
}

/// What was found in a table and what was made of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub rows: usize,
    /// Every numeric column (the candidates for signal and time).
    pub columns: Vec<String>,
    /// The column that was analysed.
    pub signal: String,
    /// The column the sample rate came from, if any.
    pub time: Option<String>,
    /// Things worth knowing: assumed sample rate, repaired samples, uneven time steps.
    pub notes: Vec<String>,
}

/// A table column ready for the analyzer.
pub struct Signal {
    pub samples: Vec<f32>,
    pub sample_rate: f64,
    pub info: TableInfo,
}

// ---------------------------------------------------------------- table data ---

/// A whole table, independent of the file format it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct TableData {
    /// Number of data rows (the header row is not counted).
    pub rows: usize,
    pub columns: Vec<TableColumn>,
}

/// One column: numeric values (missing entries are `None`) or text.
#[derive(Clone, Debug, PartialEq)]
pub struct TableColumn {
    pub name: String,
    pub kind: ColumnKind,
    /// Number of missing cells. For numeric columns this is the `None` count.
    pub missing: usize,
}

/// Numeric columns can become the signal/time column; text columns cannot.
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnKind {
    /// One entry per data row; `None` is a missing/empty cell.
    Numeric(Vec<Option<f64>>),
    /// Strings, booleans, dates, decimals, arrays, objects, or any other
    /// non-Int/UInt/Float Arrow type.
    Text,
}

/// How table columns map onto plotted lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotLayout {
    /// First column = shared x; every remaining numeric column is one line.
    Columns,
    /// Columns as x/y pairs: `(0, 1)`, `(2, 3)`, ...; each pair is one line.
    Pairs,
}

impl PlotLayout {
    pub const fn as_str(self) -> &'static str {
        match self {
            PlotLayout::Columns => "columns",
            PlotLayout::Pairs => "pairs",
        }
    }
}

/// One plotted point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlotPoint {
    pub x: f64,
    pub y: f64,
}

/// One plotted line and the column label it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotSeries {
    pub label: String,
    pub points: Vec<PlotPoint>,
}

/// A table mapped onto x/y curves for the generic Plot display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePlot {
    /// Label of the shared x column (or the first pair's x column).
    pub x_label: String,
    pub series: Vec<PlotSeries>,
    /// Data rows in the source table.
    pub rows: usize,
    /// Non-fatal notes: ignored columns, skipped points, uneven pairs.
    pub notes: Vec<String>,
}

fn numeric_values(column: &TableColumn) -> Option<&[Option<f64>]> {
    match &column.kind {
        ColumnKind::Numeric(values) => Some(values),
        ColumnKind::Text => None,
    }
}

fn plot_points(
    x_values: &[Option<f64>],
    y_values: &[Option<f64>],
    label: &str,
    notes: &mut Vec<String>,
) -> PlotSeries {
    let mut points = Vec::with_capacity(x_values.len().min(y_values.len()));
    let mut skipped = 0usize;
    for (x, y) in x_values.iter().zip(y_values.iter()) {
        match (x, y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => {
                points.push(PlotPoint { x: *x, y: *y });
            }
            _ => skipped += 1,
        }
    }
    if skipped > 0 {
        notes.push(format!(
            "{skipped} row(s) skipped in '{label}': missing or non-finite values"
        ));
    }
    PlotSeries {
        label: label.to_string(),
        points,
    }
}

impl TableData {
    /// Choose the signal column and the sample rate (see [`DataSettings`]).
    pub fn into_signal(&self, d: &DataSettings) -> Result<Signal> {
        let numeric: Vec<String> = self
            .columns
            .iter()
            .filter(|c| matches!(c.kind, ColumnKind::Numeric(_)))
            .map(|c| c.name.clone())
            .collect();

        let time_name = d.time_column.clone().or_else(|| {
            numeric
                .iter()
                .find(|n| TIME_NAMES.contains(&n.to_ascii_lowercase().as_str()))
                .cloned()
        });
        let signal_name = d
            .column
            .clone()
            .or_else(|| {
                numeric
                    .iter()
                    .find(|n| Some(*n) != time_name.as_ref())
                    .cloned()
            })
            .ok_or("the table has no numeric column to analyse")?;
        if Some(&signal_name) == time_name.as_ref() {
            return Err(
                format!("'{signal_name}' cannot be both the signal and the time column").into(),
            );
        }

        let signal_index = self.numeric_index(&signal_name, &numeric)?;
        let signal_column = &self.columns[signal_index];
        let missing = signal_column.missing;
        let mut notes = Vec::new();
        if missing > 0 {
            notes.push(format!(
                "{missing} missing samples repeat the previous value"
            ));
        }

        let mut samples = match &signal_column.kind {
            ColumnKind::Numeric(values) => fill_missing(values),
            ColumnKind::Text => return Err(format!("column '{signal_name}' is not numeric").into()),
        };

        // Sample rate: fixed > derived from the time column > 1 Hz.
        let mut time_used = None;
        let sample_rate = match (d.sample_rate, &time_name) {
            (Some(sr), _) => sr,
            (None, Some(name)) => {
                let index = self.numeric_index(name, &numeric)?;
                let time = match &self.columns[index].kind {
                    ColumnKind::Numeric(values) => fill_missing(values),
                    ColumnKind::Text => {
                        return Err(format!("column '{name}' is not numeric").into())
                    }
                };
                let steps: Vec<f64> = time.windows(2).map(|w| w[1] - w[0]).collect();
                let span = time.last().zip(time.first()).map_or(0.0, |(l, f)| l - f);
                if steps.is_empty() || span <= 0.0 {
                    return Err(format!("time column '{name}' does not increase").into());
                }
                let mean_step = span / steps.len() as f64;
                if steps
                    .iter()
                    .any(|s| (s - mean_step).abs() > 0.01 * mean_step)
                {
                    notes.push(format!(
                        "'{name}' is not evenly spaced (> 1 %): the mean step is used"
                    ));
                }
                time_used = Some(name.clone());
                1.0 / mean_step
            }
            (None, None) => {
                notes.push("no time column or sample rate: assuming 1 Hz".into());
                1.0
            }
        };

        if d.remove_mean && !samples.is_empty() {
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            samples.iter_mut().for_each(|s| *s -= mean);
        }

        Ok(Signal {
            samples: samples
                .iter()
                .map(|&s| if s.is_finite() { s as f32 } else { 0.0 })
                .collect(),
            sample_rate,
            info: TableInfo {
                rows: self.rows,
                columns: numeric,
                signal: signal_name,
                time: time_used,
                notes,
            },
        })
    }

    /// Map this table onto x/y curves for a generic plot.
    ///
    /// [`PlotLayout::Columns`] treats the first column as a shared x axis and
    /// every remaining numeric column as its own line; text columns are
    /// ignored with a note. [`PlotLayout::Pairs`] reads `(x, y)` column pairs,
    /// which suits files written as `x1,y1,x2,y2,...`.
    pub fn into_plot(&self, layout: PlotLayout) -> Result<TablePlot> {
        if self.columns.is_empty() {
            return Err("the table has no columns".into());
        }
        let mut notes = Vec::new();
        let mut series = Vec::new();
        let x_label = match layout {
            PlotLayout::Columns => {
                let x_column = &self.columns[0];
                let Some(x_values) = numeric_values(x_column) else {
                    return Err(format!(
                        "first column '{}' is not numeric; use the x/y pair layout",
                        x_column.name
                    )
                    .into());
                };
                for column in &self.columns[1..] {
                    match numeric_values(column) {
                        Some(y_values) => {
                            let line = plot_points(x_values, y_values, &column.name, &mut notes);
                            if line.points.is_empty() {
                                notes.push(format!(
                                    "column '{}' has no plottable points",
                                    column.name
                                ));
                            } else {
                                series.push(line);
                            }
                        }
                        None => notes.push(format!("text column '{}' ignored", column.name)),
                    }
                }
                if series.is_empty() {
                    return Err("the table has no numeric y column to plot".into());
                }
                x_column.name.clone()
            }
            PlotLayout::Pairs => {
                let mut index = 0;
                let mut x_label: Option<String> = None;
                while index + 1 < self.columns.len() {
                    let (x_column, y_column) = (&self.columns[index], &self.columns[index + 1]);
                    let Some(x_values) = numeric_values(x_column) else {
                        return Err(format!(
                            "x column '{}' of the pair starting at column {} is not numeric",
                            x_column.name,
                            index + 1
                        )
                        .into());
                    };
                    let Some(y_values) = numeric_values(y_column) else {
                        return Err(format!(
                            "y column '{}' of the pair starting at column {} is not numeric",
                            y_column.name,
                            index + 1
                        )
                        .into());
                    };
                    let line = plot_points(x_values, y_values, &y_column.name, &mut notes);
                    if line.points.is_empty() {
                        notes.push(format!(
                            "pair '{} / {}' has no plottable points",
                            x_column.name, y_column.name
                        ));
                    } else {
                        series.push(line);
                    }
                    if x_label.is_none() {
                        x_label = Some(x_column.name.clone());
                    }
                    index += 2;
                }
                if index < self.columns.len() {
                    notes.push(format!(
                        "trailing column '{}' ignored: it has no y partner",
                        self.columns[index].name
                    ));
                }
                if series.is_empty() {
                    return Err("the table has no complete x/y column pair to plot".into());
                }
                x_label.unwrap_or_else(|| "x".to_string())
            }
        };
        Ok(TablePlot {
            x_label,
            series,
            rows: self.rows,
            notes,
        })
    }

    /// Index of a numeric column called `name`, or an error listing the usable ones.
    fn numeric_index(&self, name: &str, numeric: &[String]) -> Result<usize> {
        let index = self
            .columns
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| {
                Error::from(format!(
                    "no column '{name}'; numeric columns: {}",
                    numeric.join(", ")
                ))
            })?;
        if !matches!(self.columns[index].kind, ColumnKind::Numeric(_)) {
            return Err(format!("column '{name}' is not numeric").into());
        }
        Ok(index)
    }
}

/// Forward-fill missing values (zero before the first value).
fn fill_missing(values: &[Option<f64>]) -> Vec<f64> {
    let mut last = 0.0;
    values
        .iter()
        .map(|value| match value {
            Some(v) => {
                last = *v;
                *v
            }
            None => last,
        })
        .collect()
}

// ------------------------------------------------------------------- reading ---

/// Read a whole table. The format comes from [`TableFormat::from_path`] in the
/// pipeline, so a disabled format produces a clear error here instead of being
/// mistaken for audio.
pub fn read_table(path: &Path, format: TableFormat) -> Result<TableData> {
    match format {
        TableFormat::Csv { .. } => {
            #[cfg(feature = "tables-text")]
            {
                text_table::read_delimited(path, format)
            }
            #[cfg(not(feature = "tables-text"))]
            {
                let _ = path;
                Err("built without text table support (enable the 'tables-text' feature)".into())
            }
        }
        TableFormat::Json | TableFormat::JsonLines => {
            #[cfg(feature = "tables-text")]
            {
                text_table::read_json(path, format)
            }
            #[cfg(not(feature = "tables-text"))]
            {
                let _ = (path, format);
                Err("built without text table support (enable the 'tables-text' feature)".into())
            }
        }
        TableFormat::Parquet => {
            #[cfg(feature = "tables-parquet")]
            {
                binary_table::read_parquet(path)
            }
            #[cfg(not(feature = "tables-parquet"))]
            {
                let _ = path;
                Err(
                    "built without parquet table support (enable the 'tables-parquet' feature)"
                        .into(),
                )
            }
        }
        TableFormat::Ipc => {
            #[cfg(feature = "tables-arrow")]
            {
                binary_table::read_arrow(path)
            }
            #[cfg(not(feature = "tables-arrow"))]
            {
                let _ = path;
                Err("built without arrow table support (enable the 'tables-arrow' feature)".into())
            }
        }
    }
}

/// Read `path` and turn the chosen column into a signal.
pub fn load(path: &Path, format: TableFormat, data: &DataSettings) -> Result<Signal> {
    read_table(path, format)?.into_signal(data)
}

/// Read `path` as a table and map it onto generic x/y plot lines.
pub fn load_plot(path: &Path, layout: PlotLayout) -> Result<TablePlot> {
    let format = TableFormat::from_path(path).ok_or_else(|| {
        Error::from(format!(
            "'{}' is not a readable table (expected one of: {})",
            path.display(),
            available_table_extensions().join(", ")
        ))
    })?;
    read_table(path, format)?.into_plot(layout)
}

/// Read a text table from an in-memory buffer.
///
/// `extension` is the original file name extension, used for the same
/// delimiter/sniffing rules as [`read_table`]. Binary table formats are
/// intentionally not supported here; the lean WASM build only enables
/// `tables-text`.
pub fn read_table_bytes(
    bytes: &[u8],
    format: TableFormat,
    extension: Option<&str>,
) -> Result<TableData> {
    match format {
        TableFormat::Csv { .. } => {
            #[cfg(feature = "tables-text")]
            {
                text_table::read_delimited_bytes(bytes.to_vec(), extension, format)
            }
            #[cfg(not(feature = "tables-text"))]
            {
                let _ = (bytes, extension, format);
                Err("built without text table support (enable the 'tables-text' feature)".into())
            }
        }
        TableFormat::Json | TableFormat::JsonLines => {
            #[cfg(feature = "tables-text")]
            {
                text_table::read_json_bytes(bytes.to_vec(), format, "<memory>")
            }
            #[cfg(not(feature = "tables-text"))]
            {
                let _ = (bytes, format);
                Err("built without text table support (enable the 'tables-text' feature)".into())
            }
        }
        TableFormat::Parquet | TableFormat::Ipc => Err(format!(
            "in-memory {} tables are not supported (use a file path)",
            format.kind_name()
        )
        .into()),
    }
}

/// Read an in-memory text table and turn the chosen column into a signal.
pub fn load_bytes(
    bytes: &[u8],
    format: TableFormat,
    data: &DataSettings,
    extension: Option<&str>,
) -> Result<Signal> {
    read_table_bytes(bytes, format, extension)?.into_signal(data)
}

// --------------------------------------------------------------- delimited text ---

#[cfg(feature = "tables-text")]
mod text_table {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use serde_json::Value;

    use super::{ColumnKind, TableColumn, TableData, TableFormat};
    use crate::traits::Result;

    use csv::{ReaderBuilder, StringRecord, Trim};

    const SNIFF_CANDIDATES: [u8; 4] = [b',', b'\t', b';', b'|'];
    const SNIFF_ROWS: usize = 20;

    pub(super) fn read_delimited(path: &Path, format: TableFormat) -> Result<TableData> {
        read_delimited_bytes(
            std::fs::read(path)?,
            path.extension().and_then(|e| e.to_str()),
            format,
        )
    }

    pub(super) fn read_delimited_bytes(
        mut bytes: Vec<u8>,
        extension: Option<&str>,
        format: TableFormat,
    ) -> Result<TableData> {
        // Excel writes a UTF-8 BOM; it is not part of the first header name.
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bytes.drain(..3);
        }
        let extension = extension.map(|value| value.to_ascii_lowercase());
        let separator = match extension.as_deref() {
            Some("tsv" | "tab") => b'\t',
            Some("csv" | "txt" | "dat") => sniff_delimiter(&bytes),
            _ => match format {
                TableFormat::Csv { separator } => separator,
                _ => b',',
            },
        };
        parse_delimited(&bytes, separator)
    }

    /// Deterministic delimiter sniff: the candidate that reads the most
    /// non-empty records with a consistent field count wins. Ties prefer comma,
    /// then tab, `;`, and `|`; no candidate means comma.
    fn sniff_delimiter(bytes: &[u8]) -> u8 {
        let mut best = (0usize, b',');
        for &candidate in &SNIFF_CANDIDATES {
            if let Some(score) = sniff_candidate(bytes, candidate) {
                if score > best.0 {
                    best = (score, candidate);
                }
            }
        }
        best.1
    }

    fn sniff_candidate(bytes: &[u8], delimiter: u8) -> Option<usize> {
        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .trim(Trim::All)
            .from_reader(bytes);
        let mut fields = None;
        let mut sampled = 0usize;
        for record in reader.byte_records() {
            let record = record.ok()?;
            if is_blank(&record) {
                continue;
            }
            let count = record.len();
            if count < 2 {
                return None;
            }
            match fields {
                None => fields = Some(count),
                Some(expected) if expected == count => {}
                Some(_) => return None,
            }
            sampled += 1;
            if sampled >= SNIFF_ROWS {
                break;
            }
        }
        (sampled > 0).then_some(sampled)
    }

    fn is_blank(record: &csv::ByteRecord) -> bool {
        record
            .iter()
            .all(|field| field.iter().all(u8::is_ascii_whitespace))
    }

    fn parse_delimited(bytes: &[u8], delimiter: u8) -> Result<TableData> {
        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(true)
            .flexible(true)
            .trim(Trim::All)
            .from_reader(bytes);

        let headers = reader
            .headers()
            .map_err(|e| format!("could not read the table header: {e}"))?
            .clone();
        if headers.is_empty() {
            return Err("the table has no header row".into());
        }
        let names = unique_headers(&headers);

        let mut values: Vec<Vec<Option<f64>>> = vec![Vec::new(); names.len()];
        let mut text = vec![false; names.len()];
        let mut missing = vec![0usize; names.len()];
        let mut rows = 0usize;

        for record in reader.records() {
            let record = record.map_err(|e| format!("could not read row {}: {e}", rows + 1))?;
            rows += 1;
            if record.len() > names.len() {
                return Err(format!(
                    "row {rows} has {} fields; expected {}",
                    record.len(),
                    names.len()
                )
                .into());
            }
            for column in 0..names.len() {
                let raw = record.get(column).unwrap_or("");
                if raw.is_empty() {
                    missing[column] += 1;
                    if !text[column] {
                        values[column].push(None);
                    }
                    continue;
                }
                if text[column] {
                    continue;
                }
                match raw.parse::<f64>() {
                    Ok(value) => values[column].push(Some(value)),
                    Err(_) => {
                        text[column] = true;
                        values[column].clear();
                    }
                }
            }
        }

        let columns = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let kind = if text[index] {
                    ColumnKind::Text
                } else {
                    ColumnKind::Numeric(std::mem::take(&mut values[index]))
                };
                TableColumn {
                    name,
                    kind,
                    missing: missing[index],
                }
            })
            .collect();
        Ok(TableData { rows, columns })
    }

    /// Empty headers become `column_{index}`; duplicates get `_2`, `_3`, ...
    /// while keeping every final name unique.
    fn unique_headers(headers: &StringRecord) -> Vec<String> {
        let mut used = HashSet::new();
        headers
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let base = if raw.is_empty() {
                    format!("column_{index}")
                } else {
                    raw.to_string()
                };
                let mut name = base.clone();
                let mut suffix = 2usize;
                while used.contains(&name) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                used.insert(name.clone());
                name
            })
            .collect()
    }

    pub(super) fn read_json(path: &Path, format: TableFormat) -> Result<TableData> {
        read_json_bytes(std::fs::read(path)?, format, &path.display().to_string())
    }

    pub(super) fn read_json_bytes(
        bytes: Vec<u8>,
        format: TableFormat,
        label: &str,
    ) -> Result<TableData> {
        let text =
            String::from_utf8(bytes).map_err(|e| format!("{label} is not valid UTF-8: {e}"))?;
        let rows: Vec<serde_json::Map<String, Value>> = match format {
            TableFormat::JsonLines => text
                .lines()
                .enumerate()
                .filter(|(_, line)| !line.trim().is_empty())
                .map(
                    |(index, line)| match serde_json::from_str::<Value>(line.trim()) {
                        Ok(Value::Object(object)) => Ok(object),
                        Ok(_) => Err(format!("line {} is not a JSON object", index + 1)),
                        Err(e) => Err(format!("line {} is not valid JSON: {e}", index + 1)),
                    },
                )
                .collect::<std::result::Result<_, String>>()
                .map_err(|e| -> crate::traits::Error { e.into() })?,
            _ => {
                let value: Value = serde_json::from_str(&text)
                    .map_err(|e| format!("could not read JSON table '{label}': {e}"))?;
                let Value::Array(rows) = value else {
                    return Err("JSON table must be an array of objects".into());
                };
                rows.into_iter()
                    .enumerate()
                    .map(|(index, row)| match row {
                        Value::Object(object) => Ok(object),
                        _ => Err(format!("row {} is not a JSON object", index + 1)),
                    })
                    .collect::<std::result::Result<_, String>>()
                    .map_err(|e| -> crate::traits::Error { e.into() })?
            }
        };

        // Keys in first-seen order across all rows. `serde_json` is built with
        // `preserve_order`, so each object keeps the order it was parsed in.
        let mut names = Vec::new();
        let mut positions = HashMap::new();
        for row in &rows {
            for key in row.keys() {
                if !positions.contains_key(key) {
                    positions.insert(key.clone(), names.len());
                    names.push(key.clone());
                }
            }
        }

        let mut values: Vec<Vec<Option<f64>>> = vec![Vec::new(); names.len()];
        let mut text = vec![false; names.len()];
        let mut missing = vec![0usize; names.len()];
        for row in &rows {
            for (column, name) in names.iter().enumerate() {
                match row.get(name) {
                    None | Some(Value::Null) => {
                        missing[column] += 1;
                        if !text[column] {
                            values[column].push(None);
                        }
                    }
                    Some(Value::Number(number)) if !text[column] => match number.as_f64() {
                        Some(value) => values[column].push(Some(value)),
                        None => {
                            text[column] = true;
                            values[column].clear();
                        }
                    },
                    Some(_) => {
                        text[column] = true;
                        values[column].clear();
                    }
                }
            }
        }

        let columns = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let kind = if text[index] {
                    ColumnKind::Text
                } else {
                    ColumnKind::Numeric(std::mem::take(&mut values[index]))
                };
                TableColumn {
                    name,
                    kind,
                    missing: missing[index],
                }
            })
            .collect();
        Ok(TableData {
            rows: rows.len(),
            columns,
        })
    }
}

// ------------------------------------------------------------- parquet / arrow ---

#[cfg(any(feature = "tables-parquet", feature = "tables-arrow"))]
mod binary_table {
    use std::fs::File;
    use std::path::Path;

    use arrow::array::{
        Array, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array, Int8Array,
        UInt16Array, UInt32Array, UInt64Array, UInt8Array,
    };
    use arrow::datatypes::{DataType, Schema};
    use arrow::record_batch::RecordBatch;

    use super::{ColumnKind, TableColumn, TableData};
    use crate::traits::{Error, Result};

    #[cfg_attr(not(feature = "tables-parquet"), allow(dead_code))]
    pub(super) fn read_parquet(path: &Path) -> Result<TableData> {
        #[cfg(feature = "tables-parquet")]
        {
            use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

            let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)
                .map_err(|e| format!("could not read parquet '{}': {e}", path.display()))?;
            let mut table = TableBuilder::new(builder.schema());
            let reader = builder
                .build()
                .map_err(|e| format!("could not read parquet '{}': {e}", path.display()))?;
            for batch in reader {
                let batch = batch
                    .map_err(|e| format!("could not read parquet '{}': {e}", path.display()))?;
                table.push(&batch)?;
            }
            Ok(table.finish())
        }
        #[cfg(not(feature = "tables-parquet"))]
        {
            let _ = path;
            Err("built without parquet table support (enable the 'tables-parquet' feature)".into())
        }
    }

    #[cfg_attr(not(feature = "tables-arrow"), allow(dead_code))]
    pub(super) fn read_arrow(path: &Path) -> Result<TableData> {
        #[cfg(feature = "tables-arrow")]
        {
            match read_arrow_file(path) {
                Ok(table) => Ok(table),
                Err(file_error) => match read_arrow_stream(path) {
                    Ok(table) => Ok(table),
                    Err(stream_error) => Err(format!(
                        "could not read Arrow IPC '{}': file: {file_error}; stream: {stream_error}",
                        path.display()
                    )
                    .into()),
                },
            }
        }
        #[cfg(not(feature = "tables-arrow"))]
        {
            let _ = path;
            Err("built without arrow table support (enable the 'tables-arrow' feature)".into())
        }
    }

    #[cfg(feature = "tables-arrow")]
    fn read_arrow_file(path: &Path) -> Result<TableData> {
        use arrow::ipc::reader::FileReader;

        let reader = FileReader::try_new(File::open(path)?, None)
            .map_err(|e| format!("could not read Arrow file: {e}"))?;
        let mut table = TableBuilder::new(&reader.schema());
        for batch in reader {
            table.push(&batch.map_err(|e| format!("could not read Arrow file: {e}"))?)?;
        }
        Ok(table.finish())
    }

    #[cfg(feature = "tables-arrow")]
    fn read_arrow_stream(path: &Path) -> Result<TableData> {
        use arrow::ipc::reader::StreamReader;

        let reader = StreamReader::try_new(File::open(path)?, None)
            .map_err(|e| format!("could not read Arrow stream: {e}"))?;
        let mut table = TableBuilder::new(&reader.schema());
        for batch in reader {
            table.push(&batch.map_err(|e| format!("could not read Arrow stream: {e}"))?)?;
        }
        Ok(table.finish())
    }

    enum BuildKind {
        Numeric(Vec<Option<f64>>),
        Text,
    }

    struct BuildColumn {
        name: String,
        kind: BuildKind,
        missing: usize,
    }

    /// Accumulates Arrow record batches into the format-neutral table model.
    struct TableBuilder {
        rows: usize,
        columns: Vec<BuildColumn>,
    }

    impl TableBuilder {
        fn new(schema: &Schema) -> Self {
            let columns = schema
                .fields()
                .iter()
                .map(|field| BuildColumn {
                    name: field.name().clone(),
                    kind: if is_numeric(field.data_type()) {
                        BuildKind::Numeric(Vec::new())
                    } else {
                        BuildKind::Text
                    },
                    missing: 0,
                })
                .collect();
            Self { rows: 0, columns }
        }

        fn push(&mut self, batch: &RecordBatch) -> Result<()> {
            if batch.num_columns() != self.columns.len() {
                return Err(format!(
                    "table batch has {} columns; expected {}",
                    batch.num_columns(),
                    self.columns.len()
                )
                .into());
            }
            self.rows += batch.num_rows();
            for (index, column) in self.columns.iter_mut().enumerate() {
                let array = batch.column(index);
                match &mut column.kind {
                    BuildKind::Numeric(values) => {
                        values.reserve(array.len());
                        for row in 0..array.len() {
                            if array.is_null(row) {
                                column.missing += 1;
                                values.push(None);
                            } else {
                                values.push(Some(numeric_value(array, row)?));
                            }
                        }
                    }
                    BuildKind::Text => column.missing += array.null_count(),
                }
            }
            Ok(())
        }

        fn finish(self) -> TableData {
            let columns = self
                .columns
                .into_iter()
                .map(|column| TableColumn {
                    name: column.name,
                    kind: match column.kind {
                        BuildKind::Numeric(values) => ColumnKind::Numeric(values),
                        BuildKind::Text => ColumnKind::Text,
                    },
                    missing: column.missing,
                })
                .collect();
            TableData {
                rows: self.rows,
                columns,
            }
        }
    }

    fn is_numeric(data_type: &DataType) -> bool {
        matches!(
            data_type,
            DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
                | DataType::Float32
                | DataType::Float64
        )
    }

    fn numeric_value(array: &dyn Array, row: usize) -> Result<f64> {
        macro_rules! value {
            ($array:ty) => {
                array
                    .as_any()
                    .downcast_ref::<$array>()
                    .ok_or_else(|| Error::from("Arrow column type did not match its schema"))?
                    .value(row) as f64
            };
        }
        Ok(match array.data_type() {
            DataType::Int8 => value!(Int8Array),
            DataType::Int16 => value!(Int16Array),
            DataType::Int32 => value!(Int32Array),
            DataType::Int64 => value!(Int64Array),
            DataType::UInt8 => value!(UInt8Array),
            DataType::UInt16 => value!(UInt16Array),
            DataType::UInt32 => value!(UInt32Array),
            DataType::UInt64 => value!(UInt64Array),
            DataType::Float32 => value!(Float32Array),
            DataType::Float64 => value!(Float64Array),
            other => {
                return Err(format!("unsupported Arrow numeric type {other:?}").into());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    use std::fs;
    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn table_format_serializes_as_a_stable_capability_name() {
        let formats = [
            (TableFormat::Csv { separator: b',' }, "csv"),
            (TableFormat::Csv { separator: b'\t' }, "csv"),
            (TableFormat::Parquet, "parquet"),
            (TableFormat::Json, "json"),
            (TableFormat::JsonLines, "jsonl"),
            (TableFormat::Ipc, "arrow"),
        ];
        for (format, name) in formats {
            let encoded = serde_json::to_string(&format).unwrap();
            assert_eq!(encoded, format!("\"{name}\""));
            assert_eq!(format.kind_name(), name);
            assert_eq!(
                TableFormat::from_kind_name(name).map(TableFormat::kind_name),
                Some(name)
            );
            assert_eq!(
                serde_json::from_str::<TableFormat>(&encoded)
                    .unwrap()
                    .kind_name(),
                name
            );
            // Separator bytes must never leak into the capability payload.
            assert!(!encoded.contains('\\'));
        }
        assert!(TableFormat::from_kind_name("tsv").is_none());
        assert!(serde_json::from_str::<TableFormat>("\"tsv\"").is_err());
    }

    // ------------------------------------------------------------ fixtures ---

    /// Unique directory per test, removed on drop.
    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    struct TempDir {
        path: PathBuf,
    }

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    impl TempDir {
        fn new(tag: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fft_audio_table_{}_{}_{}_{}",
                std::process::id(),
                stamp,
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
                tag
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self { path }
        }

        fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, contents).expect("write test fixture");
            path
        }
    }

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(feature = "tables-text")]
    fn load_file(path: &Path, data: &DataSettings) -> Result<Signal> {
        load(
            path,
            TableFormat::from_path(path).expect("table extension"),
            data,
        )
    }

    // --------------------------------------------------------- format rules ---

    #[test]
    fn formats_follow_the_extension() {
        let f = |p: &str| TableFormat::from_path(Path::new(p));
        assert_eq!(f("a.CSV"), Some(TableFormat::Csv { separator: b',' }));
        assert_eq!(f("a.tsv"), Some(TableFormat::Csv { separator: b'\t' }));
        assert_eq!(f("a.tab"), Some(TableFormat::Csv { separator: b'\t' }));
        assert_eq!(f("a.txt"), Some(TableFormat::Csv { separator: b',' }));
        assert_eq!(f("a.dat"), Some(TableFormat::Csv { separator: b',' }));
        assert_eq!(f("a.parquet"), Some(TableFormat::Parquet));
        assert_eq!(f("a.pq"), Some(TableFormat::Parquet));
        assert_eq!(f("a.json"), Some(TableFormat::Json));
        assert_eq!(f("a.jsonl"), Some(TableFormat::JsonLines));
        assert_eq!(f("a.ndjson"), Some(TableFormat::JsonLines));
        assert_eq!(f("a.arrow"), Some(TableFormat::Ipc));
        assert_eq!(f("a.ipc"), Some(TableFormat::Ipc));
        assert_eq!(f("a.feather"), Some(TableFormat::Ipc));
        assert_eq!(f("a.wav"), None);
        assert_eq!(f("noextension"), None);
    }

    #[test]
    fn every_listed_extension_is_read_as_a_table() {
        for ext in TABLE_EXTENSIONS {
            let path = PathBuf::from(format!("data.{ext}"));
            assert!(
                TableFormat::from_path(&path).is_some(),
                "{ext} is listed but has no format"
            );
        }
    }

    #[test]
    fn available_extensions_respect_features() {
        let expected: Vec<&str> = TABLE_EXTENSIONS
            .iter()
            .copied()
            .filter(|ext| {
                let name = format!("data.{ext}");
                TableFormat::from_path(Path::new(&name)).is_some_and(format_available)
            })
            .collect();
        assert_eq!(available_table_extensions(), expected);
        assert_eq!(
            available_table_extensions().contains(&"csv"),
            cfg!(feature = "tables-text")
        );
        assert_eq!(
            available_table_extensions().contains(&"parquet"),
            cfg!(feature = "tables-parquet")
        );
        assert_eq!(
            available_table_extensions().contains(&"arrow"),
            cfg!(feature = "tables-arrow")
        );
    }

    // ------------------------------------------------- numeric selection ---

    #[cfg(feature = "tables-text")]
    #[test]
    fn the_time_column_gives_the_sample_rate_and_is_not_the_signal() {
        let dir = TempDir::new("time-column");
        let path = dir.write("time.csv", "time,ax\n0.00,1\n0.01,2\n0.02,3\n0.03,4\n");
        let s = load_file(&path, &DataSettings::default()).unwrap();
        assert!((s.sample_rate - 100.0).abs() < 1e-6, "{}", s.sample_rate);
        assert_eq!(s.info.signal, "ax");
        assert_eq!(s.info.time.as_deref(), Some("time"));
        assert_eq!(s.info.columns, vec!["time", "ax"]);
        assert_eq!(s.info.rows, 4);
        assert_eq!(s.samples.len(), 4);
        assert!(s.info.notes.is_empty(), "{:?}", s.info.notes);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn the_mean_is_removed_unless_asked_not_to() {
        let dir = TempDir::new("mean");
        let text = "x\n10\n12\n10\n12\n";
        let path = dir.write("mean.csv", text);
        let s = load_file(&path, &DataSettings::default()).unwrap();
        assert_eq!(s.samples, vec![-1.0, 1.0, -1.0, 1.0]);
        assert_eq!(s.sample_rate, 1.0);
        assert!(s.info.notes.iter().any(|n| n.contains("1 Hz")));
        let keep = DataSettings {
            remove_mean: false,
            ..Default::default()
        };
        assert_eq!(
            load_file(&path, &keep).unwrap().samples,
            vec![10.0, 12.0, 10.0, 12.0]
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn a_fixed_sample_rate_and_a_named_column_win() {
        let dir = TempDir::new("named");
        let path = dir.write("named.csv", "time,a,b\n0,1,5\n1,2,6\n2,3,7\n");
        let d = DataSettings {
            column: Some("b".into()),
            sample_rate: Some(0.25),
            remove_mean: false,
            ..Default::default()
        };
        let s = load_file(&path, &d).unwrap();
        assert_eq!(s.sample_rate, 0.25);
        assert_eq!(s.samples, vec![5.0, 6.0, 7.0]);
        assert_eq!(s.info.time, None);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn a_fixed_rate_does_not_read_the_time_column() {
        let dir = TempDir::new("fixed-rate-time");
        let path = dir.write("fixed.csv", "v\n1\n2\n");
        let d = DataSettings {
            time_column: Some("not there".into()),
            sample_rate: Some(10.0),
            remove_mean: false,
            ..Default::default()
        };
        let s = load_file(&path, &d).unwrap();
        assert_eq!(s.sample_rate, 10.0);
        assert_eq!(s.samples, vec![1.0, 2.0]);
        assert_eq!(s.info.time, None);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn missing_values_repeat_the_previous_one() {
        let dir = TempDir::new("missing");
        let path = dir.write("nulls.csv", "a,b\n,1\n3,2\n,3\n5,4\n");
        let d = DataSettings {
            remove_mean: false,
            ..Default::default()
        };
        let s = load_file(&path, &d).unwrap();
        // The empty cells are missing: leading -> 0, later -> previous value.
        assert_eq!(s.samples, vec![0.0, 3.0, 3.0, 5.0]);
        assert!(
            s.info
                .notes
                .iter()
                .any(|n| n == "2 missing samples repeat the previous value"),
            "{:?}",
            s.info.notes
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn wrong_columns_are_reported_with_the_available_ones() {
        let dir = TempDir::new("wrong-column");
        let path = dir.write("wrong.csv", "a,b\n1,2\n3,4\n");
        let d = DataSettings {
            column: Some("nope".into()),
            ..Default::default()
        };
        let e = load_file(&path, &d).err().expect("must fail").to_string();
        assert!(e.contains("nope") && e.contains("a, b"), "{e}");

        let text_path = dir.write("text.csv", "name\nx\ny\n");
        let text_only = load_file(&text_path, &DataSettings::default())
            .err()
            .expect("must fail");
        assert!(
            text_only.to_string().contains("no numeric column"),
            "{text_only}"
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn signal_and_time_must_be_different_columns() {
        let dir = TempDir::new("same-column");
        let path = dir.write("same.csv", "time,v\n0,1\n0.01,2\n");
        let d = DataSettings {
            column: Some("time".into()),
            ..Default::default()
        };
        let e = load_file(&path, &d).err().expect("must fail").to_string();
        assert!(e.contains("cannot be both"), "{e}");
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn explicit_text_columns_are_rejected() {
        let dir = TempDir::new("text-column");
        let path = dir.write("labels.csv", "label,v\na,1\nb,2\n");
        for d in [
            DataSettings {
                column: Some("label".into()),
                ..Default::default()
            },
            DataSettings {
                time_column: Some("label".into()),
                ..Default::default()
            },
        ] {
            let e = load_file(&path, &d).err().expect("must fail").to_string();
            assert!(e.contains("column 'label' is not numeric"), "{e}");
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn time_must_advance_overall() {
        let dir = TempDir::new("time-steps");
        // A zero step is uneven but still analyzable when the total span
        // advances.
        let uneven = dir.write("uneven.csv", "time,v\n0,1\n1,2\n1,3\n");
        let signal = load_file(&uneven, &DataSettings::default()).unwrap();
        assert!((signal.sample_rate - 2.0).abs() < 1e-6);
        assert!(
            signal
                .info
                .notes
                .iter()
                .any(|n| n.contains("not evenly spaced")),
            "{:?}",
            signal.info.notes
        );

        let single = dir.write("single.csv", "time,v\n0,1\n");
        let e = load_file(&single, &DataSettings::default())
            .err()
            .expect("must fail")
            .to_string();
        assert!(e.contains("does not increase"), "{e}");
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn uneven_time_uses_the_mean_step_and_notes_it() {
        let dir = TempDir::new("uneven");
        let path = dir.write("uneven.csv", "time,v\n0,1\n0.01,2\n0.04,3\n");
        let d = DataSettings {
            remove_mean: false,
            ..Default::default()
        };
        let s = load_file(&path, &d).unwrap();
        assert!((s.sample_rate - 50.0).abs() < 1e-6, "{}", s.sample_rate);
        assert!(
            s.info
                .notes
                .iter()
                .any(|n| n == "'time' is not evenly spaced (> 1 %): the mean step is used"),
            "{:?}",
            s.info.notes
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn no_time_column_assumes_one_hz() {
        let dir = TempDir::new("one-hz");
        let path = dir.write("plain.csv", "v\n1\n2\n");
        let s = load_file(
            &path,
            &DataSettings {
                remove_mean: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.sample_rate, 1.0);
        assert_eq!(s.info.time, None);
        assert_eq!(
            s.info.notes,
            vec!["no time column or sample rate: assuming 1 Hz"]
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn non_finite_values_become_zero() {
        let dir = TempDir::new("non-finite");
        let path = dir.write("nonfinite.csv", "v\n1\nNaN\ninf\n");
        let s = load_file(
            &path,
            &DataSettings {
                remove_mean: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.samples, vec![1.0, 0.0, 0.0]);
    }

    // ------------------------------------------------------ delimited text ---

    #[cfg(feature = "tables-text")]
    #[test]
    fn quoted_fields_and_crlf_are_handled() {
        let dir = TempDir::new("quoted");
        for (name, separator) in [("quoted.csv", ","), ("quoted.tsv", "\t")] {
            let text = format!(
                "time{separator}label{separator}\"value\"\r\n0{separator}\"a{separator}b\"{separator}1\r\n0.01{separator}\"c\"\"d\"{separator}2\r\n"
            );
            let path = dir.write(name, text);
            let s = load_file(
                &path,
                &DataSettings {
                    remove_mean: false,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(s.info.columns, vec!["time", "value"], "{name}");
            assert_eq!(s.info.signal, "value", "{name}");
            assert_eq!(s.samples, vec![1.0, 2.0], "{name}");
            assert!((s.sample_rate - 100.0).abs() < 1e-6, "{name}");
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn sniffing_finds_comma_semicolon_and_pipe() {
        let dir = TempDir::new("sniff");
        for (name, separator) in [
            ("comma.csv", ","),
            ("semicolon.txt", ";"),
            ("pipe.dat", "|"),
            ("tab.csv", "\t"),
        ] {
            let text =
                format!("time{separator}v\n0{separator}1\n0.01{separator}2\n0.02{separator}3\n");
            let path = dir.write(name, text);
            let s = load_file(
                &path,
                &DataSettings {
                    remove_mean: false,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(s.info.signal, "v", "{name}");
            assert_eq!(s.samples, vec![1.0, 2.0, 3.0], "{name}");
            assert!((s.sample_rate - 100.0).abs() < 1e-6, "{name}");
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn delimiter_ties_prefer_comma() {
        let dir = TempDir::new("sniff-tie");
        let path = dir.write("tie.csv", "a,b;c\n1,2;3\n");
        let table = read_table(&path, TableFormat::from_path(&path).unwrap()).unwrap();
        assert_eq!(table.columns[0].name, "a");
        assert_eq!(table.columns[1].name, "b;c");
        assert_eq!(table.rows, 1);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn tsv_and_tab_force_tabs() {
        let dir = TempDir::new("forced-tab");
        for ext in ["tsv", "tab"] {
            let path = dir.write(&format!("forced.{ext}"), "time,v\n0,1\n0.01,2\n");
            let table = read_table(&path, TableFormat::from_path(&path).unwrap()).unwrap();
            assert_eq!(table.columns.len(), 1, "{ext}");
            assert_eq!(table.columns[0].name, "time,v", "{ext}");
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn sniffer_falls_back_to_comma() {
        let dir = TempDir::new("sniff-fallback");
        let path = dir.write("single.csv", "x\n1\n2\n");
        let table = read_table(&path, TableFormat::from_path(&path).unwrap()).unwrap();
        assert_eq!(table.columns.len(), 1);
        assert!(matches!(table.columns[0].kind, ColumnKind::Numeric(_)));
        let s = table
            .into_signal(&DataSettings {
                remove_mean: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.samples, vec![1.0, 2.0]);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn inconsistent_sniff_rows_fall_back_to_comma() {
        let dir = TempDir::new("sniff-inconsistent");
        let path = dir.write("ragged-semicolon.csv", "a;b;c\n1;2;3\n4;5\n");
        let table = read_table(&path, TableFormat::from_path(&path).unwrap()).unwrap();
        assert_eq!(table.rows, 2);
        assert_eq!(table.columns.len(), 1);
        assert_eq!(table.columns[0].name, "a;b;c");
        assert!(matches!(table.columns[0].kind, ColumnKind::Text));
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn empty_and_duplicate_headers_get_unique_names() {
        let dir = TempDir::new("headers");
        let path = dir.write("headers.csv", ",,a,a,a_2\n1,2,3,4,5\n");
        let table = read_table(&path, TableFormat::from_path(&path).unwrap()).unwrap();
        let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["column_0", "column_1", "a", "a_2", "a_2_2"]);
        assert!(table
            .columns
            .iter()
            .all(|c| matches!(c.kind, ColumnKind::Numeric(_))));
    }

    #[cfg(feature = "tables-text")]
    fn plot_from_csv(text: &str) -> TableData {
        read_table_bytes(
            text.as_bytes(),
            TableFormat::Csv { separator: b',' },
            Some("csv"),
        )
        .unwrap()
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn plot_columns_shares_the_first_column_as_x() {
        let table = plot_from_csv("x,a,b,label\n0,1,10,up\n1,2,20,down\n2,,30,flat\n");
        let plot = table.into_plot(PlotLayout::Columns).unwrap();
        assert_eq!(plot.x_label, "x");
        assert_eq!(plot.rows, 3);
        assert_eq!(plot.series.len(), 2);
        assert_eq!(plot.series[0].label, "a");
        assert_eq!(
            plot.series[0].points,
            vec![PlotPoint { x: 0.0, y: 1.0 }, PlotPoint { x: 1.0, y: 2.0 },]
        );
        assert_eq!(plot.series[1].points.len(), 3);
        assert!(plot
            .notes
            .iter()
            .any(|note| note.contains("text column 'label' ignored")));
        assert!(plot
            .notes
            .iter()
            .any(|note| note.contains("row(s) skipped in 'a'")));
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn plot_pairs_reads_each_x_y_column_pair() {
        let table = plot_from_csv("x1,y1,x2,y2\n0,1,10,5\n1,2,11,6\n");
        let plot = table.into_plot(PlotLayout::Pairs).unwrap();
        assert_eq!(plot.x_label, "x1");
        let labels: Vec<&str> = plot.series.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["y1", "y2"]);
        assert_eq!(
            plot.series[1].points,
            vec![PlotPoint { x: 10.0, y: 5.0 }, PlotPoint { x: 11.0, y: 6.0 },]
        );
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn plot_pairs_notes_a_trailing_column() {
        let table = plot_from_csv("x,y,extra\n0,1,9\n1,2,8\n");
        let plot = table.into_plot(PlotLayout::Pairs).unwrap();
        assert_eq!(plot.series.len(), 1);
        assert!(plot
            .notes
            .iter()
            .any(|note| note.contains("trailing column 'extra'")));
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn plot_columns_rejects_a_text_x_column() {
        let table = plot_from_csv("label,y\nup,1\ndown,2\n");
        let error = table
            .into_plot(PlotLayout::Columns)
            .unwrap_err()
            .to_string();
        assert!(error.contains("x/y pair layout"), "{error}");
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn short_rows_are_padded_and_long_rows_are_rejected() {
        let dir = TempDir::new("ragged");
        let padded = dir.write("padded.csv", "time,v\n0,1\n0.01\n0.02,3\n");
        let s = load_file(
            &padded,
            &DataSettings {
                remove_mean: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.info.rows, 3);
        assert_eq!(s.samples, vec![1.0, 1.0, 3.0]);
        assert!(
            s.info
                .notes
                .iter()
                .any(|n| n == "1 missing samples repeat the previous value"),
            "{:?}",
            s.info.notes
        );

        let long = dir.write("long.csv", "time,v\n0,1\n0.01,2,3\n");
        let e = load_file(&long, &DataSettings::default())
            .err()
            .expect("must fail")
            .to_string();
        assert!(e.contains("row 2") && e.contains("expected 2"), "{e}");
    }

    // ---------------------------------------------------------------- JSON ---

    #[cfg(feature = "tables-text")]
    fn expected_json_table() -> TableData {
        TableData {
            rows: 3,
            columns: vec![
                TableColumn {
                    name: "time".into(),
                    kind: ColumnKind::Numeric(vec![Some(0.0), Some(0.1), Some(0.2)]),
                    missing: 0,
                },
                TableColumn {
                    name: "v".into(),
                    kind: ColumnKind::Numeric(vec![Some(1.0), Some(2.0), None]),
                    missing: 1,
                },
                TableColumn {
                    name: "label".into(),
                    kind: ColumnKind::Text,
                    missing: 2,
                },
                TableColumn {
                    name: "extra".into(),
                    kind: ColumnKind::Text,
                    missing: 2,
                },
            ],
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn json_array_keeps_first_seen_key_order_and_types() {
        let dir = TempDir::new("json");
        let path = dir.write(
            "rows.json",
            r#"[{"time":0,"v":1,"label":"a"},
                {"time":0.1,"v":2,"extra":true},
                {"time":0.2,"v":null}]"#,
        );
        let table = read_table(&path, TableFormat::Json).unwrap();
        assert_eq!(table, expected_json_table());
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn json_lines_keeps_first_seen_key_order_and_skips_blank_lines() {
        let dir = TempDir::new("jsonl");
        let path = dir.write(
            "rows.jsonl",
            "{\"time\":0,\"v\":1,\"label\":\"a\"}\n\n{\"time\":0.1,\"v\":2,\"extra\":true}\n{\"time\":0.2,\"v\":null}\n",
        );
        let table = read_table(&path, TableFormat::JsonLines).unwrap();
        assert_eq!(table, expected_json_table());
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn a_string_in_a_numeric_json_column_makes_it_text() {
        let dir = TempDir::new("json-mixed");
        let path = dir.write("mixed.json", r#"[{"x":1},{"x":"two"},{"x":3}]"#);
        let table = read_table(&path, TableFormat::Json).unwrap();
        assert_eq!(table.rows, 3);
        assert!(matches!(table.columns[0].kind, ColumnKind::Text));
        assert_eq!(table.columns[0].missing, 0);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn malformed_json_reports_the_row_or_line() {
        let dir = TempDir::new("json-errors");
        let object = dir.write("object.json", r#"{"a":1}"#);
        let e = read_table(&object, TableFormat::Json)
            .unwrap_err()
            .to_string();
        assert!(e.contains("array of objects"), "{e}");

        let row = dir.write("row.json", r#"[{"a":1},2]"#);
        let e = read_table(&row, TableFormat::Json).unwrap_err().to_string();
        assert!(e.contains("row 2") && e.contains("object"), "{e}");

        let line = dir.write("line.jsonl", "{\"a\":1}\n42\n");
        let e = read_table(&line, TableFormat::JsonLines)
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 2") && e.contains("object"), "{e}");
    }

    // ------------------------------------------------- one format per test ---

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    fn sine() -> (Vec<f64>, Vec<f64>) {
        let time: Vec<f64> = (0..400).map(|i| i as f64 / 100.0).collect();
        let v = time
            .iter()
            .map(|t| (2.0 * std::f64::consts::PI * 5.0 * t).sin())
            .collect();
        (time, v)
    }

    #[cfg(any(
        feature = "tables-text",
        feature = "tables-parquet",
        feature = "tables-arrow"
    ))]
    fn assert_reaches_a_spectrogram(path: &Path) {
        let a = crate::settings::AnalysisSettings {
            frame: 128,
            ..Default::default()
        };
        let r = crate::pipeline::analyze(path, &a)
            .unwrap_or_else(|e| panic!("{} could not be imported: {e}", path.display()));
        assert!(
            (r.sample_rate - 100.0).abs() < 1e-6,
            "{}: {}",
            path.display(),
            r.sample_rate
        );
        assert!(
            !r.spectrogram.is_empty(),
            "{}: no spectrogram",
            path.display()
        );
        let info = r.table.expect("table info");
        assert_eq!(info.rows, 400, "{}", path.display());
        assert_eq!(info.signal, "v", "{}", path.display());
        assert_eq!(info.time.as_deref(), Some("time"), "{}", path.display());
    }

    #[cfg(feature = "tables-text")]
    fn delimited(separator: &str) -> String {
        let (time, v) = sine();
        let mut text = format!("time{separator}v\n");
        for (t, x) in time.iter().zip(&v) {
            text.push_str(&format!("{t:?}{separator}{x:?}\n"));
        }
        text
    }

    #[cfg(feature = "tables-text")]
    fn json_array() -> String {
        let (time, v) = sine();
        let rows: Vec<String> = time
            .iter()
            .zip(&v)
            .map(|(t, x)| format!("{{\"time\":{t:?},\"v\":{x:?}}}"))
            .collect();
        format!("[{}]", rows.join(","))
    }

    #[cfg(feature = "tables-text")]
    fn json_lines() -> String {
        let (time, v) = sine();
        let mut text = String::new();
        for (t, x) in time.iter().zip(&v) {
            text.push_str(&format!("{{\"time\":{t:?},\"v\":{x:?}}}\n"));
        }
        text
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn delimited_formats_are_importable() {
        let dir = TempDir::new("delimited-formats");
        for (ext, separator) in [("csv", ","), ("txt", ";"), ("dat", "|")] {
            let path = dir.write(&format!("data.{ext}"), delimited(separator));
            assert_reaches_a_spectrogram(&path);
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn tsv_and_tab_are_importable() {
        let dir = TempDir::new("tab-formats");
        for ext in ["tsv", "tab"] {
            let path = dir.write(&format!("data.{ext}"), delimited("\t"));
            assert_reaches_a_spectrogram(&path);
        }
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn json_is_importable() {
        let dir = TempDir::new("json-format");
        let path = dir.write("data.json", json_array());
        assert_reaches_a_spectrogram(&path);
    }

    #[cfg(feature = "tables-text")]
    #[test]
    fn json_lines_are_importable() {
        let dir = TempDir::new("jsonl-format");
        for ext in ["jsonl", "ndjson"] {
            let path = dir.write(&format!("data.{ext}"), json_lines());
            assert_reaches_a_spectrogram(&path);
        }
    }

    // ------------------------------------------------------ parquet / arrow ---

    #[cfg(feature = "tables-parquet")]
    fn parquet_sine(path: &Path) {
        use std::sync::Arc;

        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;

        let (time, v) = sine();
        let schema = Arc::new(Schema::new(vec![
            Field::new("time", DataType::Float64, false),
            Field::new("v", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(time)),
                Arc::new(Float64Array::from(v)),
            ],
        )
        .expect("parquet batch");
        let mut writer =
            ArrowWriter::try_new(std::fs::File::create(path).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }

    #[cfg(feature = "tables-parquet")]
    #[test]
    fn parquet_is_importable() {
        let dir = TempDir::new("parquet-format");
        for ext in ["parquet", "pq"] {
            let path = dir.write(&format!("data.{ext}"), []);
            parquet_sine(&path);
            assert_reaches_a_spectrogram(&path);
        }
    }

    #[cfg(feature = "tables-parquet")]
    #[test]
    fn parquet_integers_nulls_and_text_columns() {
        use std::sync::Arc;

        use arrow::array::{
            ArrayRef, BooleanArray, Float64Array, Int32Array, StringArray, UInt64Array,
        };
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;

        let dir = TempDir::new("parquet-types");
        let path = dir.write("types.parquet", []);
        let schema = Arc::new(Schema::new(vec![
            Field::new("time", DataType::Float64, false),
            Field::new("a", DataType::Int32, true),
            Field::new("b", DataType::UInt64, false),
            Field::new("label", DataType::Utf8, false),
            Field::new("flag", DataType::Boolean, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])) as ArrayRef,
                Arc::new(Int32Array::from(vec![Some(1), None, Some(3)])),
                Arc::new(UInt64Array::from(vec![10, 20, 30])),
                Arc::new(StringArray::from(vec!["x", "y", "z"])),
                Arc::new(BooleanArray::from(vec![true, false, true])),
            ],
        )
        .unwrap();
        let mut writer =
            ArrowWriter::try_new(std::fs::File::create(&path).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let table = read_table(&path, TableFormat::Parquet).unwrap();
        assert_eq!(table.rows, 3);
        assert_eq!(
            table.columns[1].kind,
            ColumnKind::Numeric(vec![Some(1.0), None, Some(3.0)])
        );
        assert_eq!(table.columns[1].missing, 1);
        assert_eq!(table.columns[3].kind, ColumnKind::Text);
        assert_eq!(table.columns[4].kind, ColumnKind::Text);
        let s = table
            .into_signal(&DataSettings {
                remove_mean: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.samples, vec![1.0, 1.0, 3.0]);
        assert!((s.sample_rate - 1.0).abs() < 1e-9);
    }

    #[cfg(feature = "tables-arrow")]
    fn arrow_schema() -> std::sync::Arc<arrow::datatypes::Schema> {
        use std::sync::Arc;

        use arrow::datatypes::{DataType, Field, Schema};
        Arc::new(Schema::new(vec![
            Field::new("time", DataType::Float64, false),
            Field::new("v", DataType::Float64, false),
        ]))
    }

    #[cfg(feature = "tables-arrow")]
    fn arrow_batch() -> arrow::record_batch::RecordBatch {
        use std::sync::Arc;

        use arrow::array::Float64Array;
        use arrow::record_batch::RecordBatch;

        let (time, v) = sine();
        RecordBatch::try_new(
            arrow_schema(),
            vec![
                Arc::new(Float64Array::from(time)),
                Arc::new(Float64Array::from(v)),
            ],
        )
        .expect("arrow batch")
    }

    #[cfg(feature = "tables-arrow")]
    fn arrow_file(path: &Path) {
        use arrow::ipc::writer::FileWriter;

        let mut writer =
            FileWriter::try_new(std::fs::File::create(path).unwrap(), &arrow_schema()).unwrap();
        writer.write(&arrow_batch()).unwrap();
        writer.finish().unwrap();
    }

    #[cfg(feature = "tables-arrow")]
    fn arrow_stream(path: &Path) {
        use arrow::ipc::writer::StreamWriter;

        let mut writer =
            StreamWriter::try_new(std::fs::File::create(path).unwrap(), &arrow_schema()).unwrap();
        writer.write(&arrow_batch()).unwrap();
        writer.finish().unwrap();
    }

    #[cfg(feature = "tables-arrow")]
    #[test]
    fn arrow_ipc_is_importable() {
        let dir = TempDir::new("arrow-format");
        for ext in ["arrow", "ipc"] {
            let path = dir.write(&format!("data.{ext}"), []);
            arrow_file(&path);
            assert_reaches_a_spectrogram(&path);
        }
        let feather = dir.write("data.feather", []);
        arrow_stream(&feather);
        assert_reaches_a_spectrogram(&feather);
    }

    #[cfg(feature = "tables-arrow")]
    #[test]
    fn arrow_primitive_types_are_numeric_and_bool_is_text() {
        use std::sync::Arc;

        use arrow::array::{
            ArrayRef, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
            Int8Array, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
        };
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;

        let dir = TempDir::new("arrow-types");
        let path = dir.write("types.arrow", []);
        let schema = Arc::new(Schema::new(vec![
            Field::new("i8", DataType::Int8, false),
            Field::new("i16", DataType::Int16, false),
            Field::new("i32", DataType::Int32, false),
            Field::new("i64", DataType::Int64, false),
            Field::new("u8", DataType::UInt8, false),
            Field::new("u16", DataType::UInt16, false),
            Field::new("u32", DataType::UInt32, false),
            Field::new("u64", DataType::UInt64, false),
            Field::new("f32", DataType::Float32, false),
            Field::new("f64", DataType::Float64, false),
            Field::new("flag", DataType::Boolean, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int8Array::from(vec![1i8])) as ArrayRef,
                Arc::new(Int16Array::from(vec![2i16])),
                Arc::new(Int32Array::from(vec![3i32])),
                Arc::new(Int64Array::from(vec![4i64])),
                Arc::new(UInt8Array::from(vec![5u8])),
                Arc::new(UInt16Array::from(vec![6u16])),
                Arc::new(UInt32Array::from(vec![7u32])),
                Arc::new(UInt64Array::from(vec![8u64])),
                Arc::new(Float32Array::from(vec![9.5f32])),
                Arc::new(Float64Array::from(vec![10.5f64])),
                Arc::new(BooleanArray::from(vec![true])),
            ],
        )
        .unwrap();
        let mut writer =
            arrow::ipc::writer::FileWriter::try_new(std::fs::File::create(&path).unwrap(), &schema)
                .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let table = read_table(&path, TableFormat::Ipc).unwrap();
        assert_eq!(table.rows, 1);
        for (index, expected) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.5, 10.5]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                table.columns[index].kind,
                ColumnKind::Numeric(vec![Some(expected)]),
                "column {index}"
            );
        }
        assert_eq!(table.columns[10].kind, ColumnKind::Text);
    }

    // ------------------------------------------------ disabled-feature errors ---

    #[cfg(not(feature = "tables-text"))]
    #[test]
    fn text_formats_report_the_missing_feature() {
        for format in [
            TableFormat::Csv { separator: b',' },
            TableFormat::Json,
            TableFormat::JsonLines,
        ] {
            let err = read_table(Path::new("data.csv"), format)
                .unwrap_err()
                .to_string();
            assert!(err.contains("built without text table support"), "{err}");
            assert!(err.contains("tables-text"), "{err}");
        }
        let err = crate::pipeline::analyze(
            Path::new("data.csv"),
            &crate::settings::AnalysisSettings::default(),
        )
        .err()
        .expect("must fail")
        .to_string();
        assert!(err.contains("built without text table support"), "{err}");
    }

    #[cfg(not(feature = "tables-parquet"))]
    #[test]
    fn parquet_reports_the_missing_feature() {
        let err = read_table(Path::new("data.parquet"), TableFormat::Parquet)
            .unwrap_err()
            .to_string();
        assert!(err.contains("built without parquet table support"), "{err}");
        assert!(err.contains("tables-parquet"), "{err}");
    }

    #[cfg(not(feature = "tables-arrow"))]
    #[test]
    fn arrow_reports_the_missing_feature() {
        let err = read_table(Path::new("data.arrow"), TableFormat::Ipc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("built without arrow table support"), "{err}");
        assert!(err.contains("tables-arrow"), "{err}");
    }
}
