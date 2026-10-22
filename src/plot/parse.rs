use std::collections::BTreeMap;

use super::{PlotCandidate, PlotDiagnostic, PlotKind, PlotLayout, PlotOutputTarget, PlotSpec};
use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

pub(crate) fn collect_plots(snapshot: &dyn TextSnapshot, arena: &BlockArena) -> Vec<PlotCandidate> {
    let nodes = arena.nodes();
    let mut plots = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if !matches!(nodes[index].kind, BlockKind::TableRow) {
            index += 1;
            continue;
        }
        let start = index;
        let mut previous = start;
        while index + 1 < nodes.len()
            && matches!(nodes[index + 1].kind, BlockKind::TableRow)
            && nodes[index].source.end == nodes[index + 1].source.start
        {
            index += 1;
        }
        let end = index;
        while previous > 0
            && matches!(nodes[previous - 1].kind, BlockKind::Keyword)
            && nodes[previous - 1].source.end == nodes[previous].source.start
        {
            previous -= 1;
        }
        let declarations = nodes[previous..start]
            .iter()
            .filter_map(|node| {
                let text = snapshot.copy_range(node.content);
                let (name, value) = text.trim().strip_prefix("#+")?.split_once(':')?;
                name.eq_ignore_ascii_case("PLOT")
                    .then(|| (node.source, value.trim().to_owned()))
            })
            .collect::<Vec<_>>();
        if let (Some(first), Some(last)) = (declarations.first(), declarations.last()) {
            plots.push(PlotCandidate {
                declaration: ByteRange {
                    start: first.0.start,
                    end: last.0.end,
                },
                table: ByteRange {
                    start: nodes[start].source.start,
                    end: nodes[end].source.end,
                },
                owner: end as u32,
                options: declarations.into_iter().map(|(_, value)| value).collect(),
                image_link: None,
                image_attributes: Default::default(),
            });
        }
        index += 1;
    }
    bind_images(snapshot, arena, &mut plots, None);
    plots
}

pub(crate) fn bind_images(
    snapshot: &dyn TextSnapshot,
    arena: &BlockArena,
    plots: &mut [PlotCandidate],
    document: Option<&std::path::Path>,
) {
    use std::{collections::HashMap, path::Path};
    for plot in plots.iter_mut() {
        plot.image_link = None;
        plot.image_attributes = Default::default();
    }
    if !plots.iter().any(|plot| {
        matches!(
            parse_spec(&plot.options),
            Ok(PlotSpec {
                output: PlotOutputTarget::File(_),
                ..
            })
        )
    }) {
        return;
    }
    let directory = super::output::document_directory(document).ok();
    let path_key = |path: &Path| {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root) = directory.as_ref() {
            root.join(path)
        } else {
            path.to_path_buf()
        };
        super::output::normalize_path(&path)
    };
    let nodes = arena.nodes();
    let mut links = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if let BlockKind::Image { path } = &node.kind {
            links
                .entry(path_key(Path::new(path.as_ref())))
                .or_insert(index);
        }
    }
    for plot in plots {
        let Ok(PlotSpec {
            output: PlotOutputTarget::File(path),
            ..
        }) = parse_spec(&plot.options)
        else {
            continue;
        };
        let Some(&index) = links.get(&path_key(&path)) else {
            continue;
        };
        plot.image_link = Some(nodes[index].source);
        for attribute in nodes[..index].iter().rev() {
            if !matches!(attribute.kind, BlockKind::Keyword) {
                break;
            }
            let text = snapshot.copy_range(attribute.content);
            if !crate::org_syntax::attributes::is_affiliated_keyword(&text) {
                break;
            }
            if crate::org_syntax::attributes::is_attr_org_line(&text) {
                plot.image_attributes =
                    crate::org_syntax::attributes::parse_attr_org_line(&text).unwrap_or_default();
                break;
            }
        }
    }
}

enum Value {
    Atom(String),
    Quoted(String),
    List(Vec<Value>),
}

pub(super) fn parse_spec(lines: &[String]) -> Result<PlotSpec, PlotDiagnostic> {
    let mut options = BTreeMap::new();
    // org-plot scans upward and overwrites each option: the first declaration wins.
    for line in lines {
        let mut text = line.as_str();
        while !text.trim_start().is_empty() {
            text = text.trim_start();
            let Some(colon) = text.find(':') else {
                return Err(syntax("Expected an Org Plot option followed by ':'"));
            };
            let name = &text[..colon];
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(syntax("Invalid Org Plot option name"));
            }
            text = &text[colon + 1..];
            let value = read_value(&mut text)?;
            let name = match name.to_ascii_lowercase().as_str() {
                "trans" => "transpose".to_owned(),
                "min" => "ymin".to_owned(),
                "max" => "ymax".to_owned(),
                name => name.to_owned(),
            };
            options.entry(name).or_insert(value);
        }
    }
    let mut spec = PlotSpec::default();
    let mut map = None;
    let explicit_ind = options.contains_key("ind");
    for (name, value) in options {
        match name.as_str() {
            "file" => {
                let path = std::path::PathBuf::from(atom(&value)?);
                if path.as_os_str().is_empty()
                    || crate::typst_runtime::OutputFormat::infer_from_path(&path).is_none()
                {
                    return Err(PlotDiagnostic::new(
                        "invalid-output",
                        "Org Plot file must name an SVG, PNG or PDF file",
                    ));
                }
                spec.output = PlotOutputTarget::File(path);
            }
            "type" => {
                spec.layout = match atom(&value)? {
                    "2d" => PlotLayout::Cartesian,
                    "radar" => PlotLayout::Radar,
                    "grid" => PlotLayout::Grid,
                    _ => {
                        return Err(PlotDiagnostic::new(
                            "unsupported-option",
                            "Supported Org Plot types are 2d, radar and planar grid",
                        ));
                    }
                };
            }
            "title" => {
                spec.title = match value {
                    Value::Atom(ref value) if value == "nil" => None,
                    _ => Some(atom(&value)?.to_owned()),
                };
            }
            "with" => {
                spec.kind = Some(match atom(&value)? {
                    "lines" => PlotKind::Lines,
                    "points" => PlotKind::Points,
                    "linespoints" => PlotKind::LinesPoints,
                    "histograms" => PlotKind::Histograms,
                    "steps" => PlotKind::Steps,
                    "fsteps" => PlotKind::FSteps,
                    "histeps" => PlotKind::HiSteps,
                    "filledcurves" | "filledcurves closed" => PlotKind::FilledCurves,
                    "filledcurves x1" => {
                        spec.fill_to_zero = true;
                        PlotKind::FilledCurves
                    }
                    "impulses" => PlotKind::Impulses,
                    "boxes" => PlotKind::Boxes,
                    other => {
                        return Err(PlotDiagnostic::new(
                            "unsupported-option",
                            format!("Unsupported Org Plot style: {other}"),
                        ));
                    }
                })
            }
            "ind" => spec.x_column = column(&value)?,
            "timeind" => {
                if let Ok(column) = atom(&value)?.parse::<usize>() {
                    spec.time = column != 0;
                    if !explicit_ind {
                        spec.x_column = column;
                    }
                } else {
                    spec.time = boolean(&value)?;
                }
            }
            "timefmt" => spec.time_format = Some(atom(&value)?.to_owned()),
            "transpose" => spec.transpose = boolean(&value)?,
            "map" => map = Some(boolean(&value)?),
            "xmin" => spec.x_axis.min = Some(number(&value)?),
            "xmax" => spec.x_axis.max = Some(number(&value)?),
            "ymin" => spec.y_axis.min = Some(number(&value)?),
            "ymax" => spec.y_axis.max = Some(number(&value)?),
            "ticks" => {
                let ticks = atom(&value)?
                    .parse::<usize>()
                    .ok()
                    .filter(|ticks| *ticks <= 12)
                    .ok_or_else(|| syntax("ticks must be between 0 and 12"))?;
                spec.ticks = Some(ticks);
            }
            "deps" => {
                spec.columns = match value {
                    Value::Atom(ref v) if v == "nil" => None,
                    Value::List(values) if values.is_empty() => None,
                    Value::List(values) => {
                        Some(values.iter().map(column).collect::<Result<_, _>>()?)
                    }
                    _ => return Err(syntax("deps must be a list, for example deps:(2 3)")),
                }
            }
            "labels" => {
                spec.labels = match value {
                    Value::Atom(ref v) if v == "nil" => Some(Vec::new()),
                    Value::List(values) => Some(
                        values
                            .iter()
                            .map(|value| match value {
                                Value::Atom(v) if v == "nil" => Ok(None),
                                Value::Atom(v) | Value::Quoted(v) => Ok(Some(v.clone())),
                                _ => Err(syntax("labels must contain strings or nil")),
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    _ => {
                        return Err(syntax(
                            "labels must be a list, for example labels:(\"Month\" \"Orders\")",
                        ));
                    }
                }
            }
            other => {
                return Err(PlotDiagnostic::new(
                    "unsupported-option",
                    format!("Org Plot option '{other}' is not supported"),
                ));
            }
        }
    }
    if map.is_some() && (spec.layout != PlotLayout::Grid || map == Some(false)) {
        return Err(PlotDiagnostic::new(
            "unsupported-option",
            "Only planar grid maps are supported; use type:grid map:t",
        ));
    }
    if spec.layout != PlotLayout::Cartesian && spec.kind.is_some() {
        return Err(PlotDiagnostic::new(
            "unsupported-option",
            "with selects a 2d drawing style; omit it for radar and grid charts",
        ));
    }
    if spec.layout == PlotLayout::Radar && !explicit_ind {
        spec.x_column = 1;
    }
    Ok(spec)
}

fn boolean(value: &Value) -> Result<bool, PlotDiagnostic> {
    match atom(value)? {
        "y" | "yes" | "t" | "true" => Ok(true),
        "n" | "no" | "nil" | "false" => Ok(false),
        _ => Err(syntax("Expected yes, t, no or nil")),
    }
}

fn number(value: &Value) -> Result<f64, PlotDiagnostic> {
    atom(value)?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| syntax("Axis bounds must be finite numbers"))
}

fn atom(value: &Value) -> Result<&str, PlotDiagnostic> {
    match value {
        Value::Atom(value) | Value::Quoted(value) => Ok(value),
        _ => Err(syntax("Expected a single Org Plot value")),
    }
}
fn column(value: &Value) -> Result<usize, PlotDiagnostic> {
    atom(value)?.parse().map_err(|_| {
        PlotDiagnostic::new(
            "invalid-column",
            "Org Plot columns must be nonnegative integers",
        )
    })
}
fn syntax(message: &str) -> PlotDiagnostic {
    PlotDiagnostic::new("invalid-option", message)
}

fn read_value(text: &mut &str) -> Result<Value, PlotDiagnostic> {
    *text = text.trim_start();
    if text.starts_with('(') {
        *text = &text[1..];
        let mut values = Vec::new();
        loop {
            *text = text.trim_start();
            if text.starts_with(')') {
                *text = &text[1..];
                break;
            }
            if text.is_empty() {
                return Err(syntax("Unclosed Org Plot list"));
            }
            if text.starts_with('(') {
                return Err(syntax("Nested Org Plot lists are not supported"));
            }
            values.push(read_value(text)?);
        }
        return Ok(Value::List(values));
    }
    if text.starts_with('"') {
        *text = &text[1..];
        let mut value = String::new();
        let mut escaped = false;
        for (offset, c) in text.char_indices() {
            if escaped {
                value.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    _ => c,
                });
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                *text = &text[offset + 1..];
                return Ok(Value::Quoted(value));
            } else {
                value.push(c);
            }
        }
        return Err(syntax("Unclosed Org Plot string"));
    }
    let end = text
        .find(|c: char| c.is_whitespace() || c == ')')
        .unwrap_or(text.len());
    if end == 0 {
        return Err(syntax("Missing Org Plot value"));
    }
    let value = text[..end].to_owned();
    *text = &text[end..];
    Ok(Value::Atom(value))
}
