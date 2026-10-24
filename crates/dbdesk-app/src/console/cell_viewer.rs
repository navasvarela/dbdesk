//! A window showing one cell's whole value, for text too long for the grid. JSON objects and
//! arrays are pretty-printed, and XML (SQL Server's plans and `FOR XML` results) is indented.

use egui::{Id, RichText, ScrollArea, TextEdit, Window};

use super::{QueryConsole, ViewedCell};

const DEFAULT_SIZE: [f32; 2] = [520.0, 360.0];

/// Draws the console's cell viewer, if one is open, and closes it when the user does.
pub fn show(ctx: &egui::Context, console: &mut QueryConsole) {
    let Some(viewed) = &console.viewed_cell else {
        return;
    };
    let mut open = true;
    Window::new(format!("Value of {}", viewed.column))
        .id(Id::new(("cell_viewer", console.id.0)))
        .open(&mut open)
        .default_size(DEFAULT_SIZE)
        .show(ctx, |ui| contents(ui, viewed));
    if !open {
        console.viewed_cell = None;
    }
}

fn contents(ui: &mut egui::Ui, viewed: &ViewedCell) {
    let Some(value) = &viewed.value else {
        ui.label(RichText::new("NULL").italics().weak());
        return;
    };
    let pretty = pretty(value);
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(value.clone());
        }
        if let Some((structure, _)) = &pretty {
            ui.weak(structure.note());
        }
    });
    let mut shown = pretty
        .as_ref()
        .map_or(value.as_str(), |(_, text)| text.as_str());
    ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        // A `&str` buffer makes the editor read-only but still selectable.
        ui.add(
            TextEdit::multiline(&mut shown)
                .code_editor()
                .desired_width(f32::INFINITY),
        );
    });
}

/// A value the viewer lays out for reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Structure {
    Json,
    Xml,
}

impl Structure {
    fn note(self) -> &'static str {
        match self {
            Self::Json => "JSON, formatted for reading; Copy copies it as stored.",
            Self::Xml => "XML, indented for reading; Copy copies it as stored.",
        }
    }
}

fn pretty(value: &str) -> Option<(Structure, String)> {
    pretty_json(value)
        .map(|text| (Structure::Json, text))
        .or_else(|| pretty_xml(value).map(|text| (Structure::Xml, text)))
}

/// Spaces per level of nesting in indented XML, as JSON gets.
const XML_INDENT: usize = 2;

/// `value` with one tag per line, indented by how deeply it nests, if it is XML: SQL Server's
/// `SHOWPLAN_XML` and `FOR XML` give one long line. Text inside an element stays on its
/// line (`<a>text</a>`). `None` if the tags don't nest, so anything else is shown as stored.
fn pretty_xml(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with('<') || !trimmed.ends_with('>') {
        return None;
    }
    let mut lines = Vec::new();
    let mut depth: usize = 0;
    let mut rest = trimmed;
    while !rest.is_empty() {
        if !rest.starts_with('<') {
            let text_end = rest.find('<').unwrap_or(rest.len());
            let text = rest[..text_end].trim();
            if !text.is_empty() {
                lines.push((depth, text.to_owned()));
            }
            rest = &rest[text_end..];
            continue;
        }
        let end = tag_end(rest)?;
        let tag = &rest[..end];
        rest = &rest[end..];
        if tag.starts_with("</") {
            depth = depth.checked_sub(1)?;
            lines.push((depth, tag.to_owned()));
        } else if tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!") {
            lines.push((depth, tag.to_owned()));
        } else {
            let text_end = rest.find('<').unwrap_or(rest.len());
            let (text, after) = rest.split_at(text_end);
            if !text.trim().is_empty() && after.starts_with("</") {
                let close = tag_end(after)?;
                lines.push((depth, format!("{tag}{}{}", text.trim(), &after[..close])));
                rest = &after[close..];
            } else {
                lines.push((depth, tag.to_owned()));
                depth += 1;
            }
        }
    }
    (depth == 0).then(|| {
        lines
            .iter()
            .map(|(depth, line)| format!("{}{line}", " ".repeat(depth * XML_INDENT)))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Where the tag `text` starts with ends: just past its `>`, which doesn't count inside a
/// quoted attribute value.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(index + 1),
            (None, _) => {}
        }
    }
    None
}

/// `value` indented, if it is a JSON object or array. Scalars are left alone: a number or a
/// quoted word looks the same either way.
fn pretty_json(value: &str) -> Option<String> {
    let trimmed = value.trim_start();
    if !trimmed.starts_with(['{', '[']) {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    serde_json::to_string_pretty(&parsed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_prints_objects_and_arrays_keeping_key_order() {
        assert_eq!(
            pretty_json(r#"{"b":1,"a":[true,null]}"#).as_deref(),
            Some("{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null\n  ]\n}")
        );
        for not_json in ["42", "\"word\"", "{not json", "plain text"] {
            assert_eq!(pretty_json(not_json), None, "{not_json}");
        }
    }

    #[test]
    fn indents_xml_keeping_text_with_its_element() {
        let plan = r#"<?xml version="1.0"?><ShowPlanXML Version="1.564"><Stmt Text="a > 1"><Op/><Name>scan</Name></Stmt></ShowPlanXML>"#;
        assert_eq!(
            pretty(plan),
            Some((
                Structure::Xml,
                "<?xml version=\"1.0\"?>\n<ShowPlanXML Version=\"1.564\">\n  <Stmt Text=\"a > 1\">\n    \
                 <Op/>\n    <Name>scan</Name>\n  </Stmt>\n</ShowPlanXML>"
                    .to_owned()
            ))
        );
        for not_xml in ["<a><b></a>", "<unclosed", "a < b and c > d", "</a>"] {
            assert_eq!(pretty_xml(not_xml), None, "{not_xml}");
        }
    }
}
