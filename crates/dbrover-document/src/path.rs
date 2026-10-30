//! Where a value sits inside a document, written as JavaScript and the MongoDB shell write it:
//! `items[2].sku`.

use std::fmt;

use crate::write::write_string;

/// One step down into a value: a field of an object, or an item of an array.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    Field(String),
    Index(usize),
}

/// The steps from a document's top to one of its values. Empty for the document itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Path(Vec<Step>);

impl Path {
    pub fn steps(&self) -> &[Step] {
        &self.0
    }

    /// This path, one step further down.
    pub fn child(&self, step: Step) -> Self {
        let mut steps = self.0.clone();
        steps.push(step);
        Self(steps)
    }
}

impl From<Vec<Step>> for Path {
    fn from(steps: Vec<Step>) -> Self {
        Self(steps)
    }
}

/// `address.city`, `items[2].sku`; a field whose name isn't an identifier is written in
/// brackets and quotes, `["first name"]`, so the path reads back the same way.
impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, step) in self.0.iter().enumerate() {
            match step {
                Step::Index(item) => write!(f, "[{item}]")?,
                Step::Field(name) if is_identifier(name) => {
                    if index > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(name)?;
                }
                Step::Field(name) => {
                    let mut quoted = String::new();
                    write_string(&mut quoted, name);
                    write!(f, "[{quoted}]")?;
                }
            }
        }
        Ok(())
    }
}

/// A name JavaScript reads after a dot: a letter, `_` or `$`, then those or digits.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> Step {
        Step::Field(name.to_owned())
    }

    #[test]
    fn writes_paths_as_the_shell_does() {
        let cases = [
            (
                vec![field("items"), Step::Index(2), field("sku")],
                "items[2].sku",
            ),
            (vec![field("address"), field("city")], "address.city"),
            (vec![Step::Index(0), field("_id")], "[0]._id"),
            (
                vec![field("first name"), field("$oid")],
                r#"["first name"].$oid"#,
            ),
            (vec![field("a.b"), field("2x")], r#"["a.b"]["2x"]"#),
            (vec![field("say \"hi\"")], r#"["say \"hi\""]"#),
            (vec![], ""),
        ];
        for (steps, expected) in cases {
            assert_eq!(Path::from(steps).to_string(), expected);
        }
    }
}
