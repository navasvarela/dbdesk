//! `theme.toml`: colours that replace the palette's, for the light theme, the dark one, or
//! both.

use std::collections::BTreeMap;
use std::fmt;

use super::{Checked, toml_table};

/// A colour of the palette, as `theme.toml` names it. The app's theme module says what each
/// is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThemeToken {
    Primary,
    Secondary,
    Success,
    Warning,
    Error,
    Base,
    Panel,
    Field,
    Control,
    ControlHover,
    Border,
    Text,
    TextWeak,
    Selection,
    OnPrimary,
}

impl ThemeToken {
    pub const ALL: [Self; 15] = [
        Self::Primary,
        Self::Secondary,
        Self::Success,
        Self::Warning,
        Self::Error,
        Self::Base,
        Self::Panel,
        Self::Field,
        Self::Control,
        Self::ControlHover,
        Self::Border,
        Self::Text,
        Self::TextWeak,
        Self::Selection,
        Self::OnPrimary,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Base => "base",
            Self::Panel => "panel",
            Self::Field => "field",
            Self::Control => "control",
            Self::ControlHover => "control_hover",
            Self::Border => "border",
            Self::Text => "text",
            Self::TextWeak => "text_weak",
            Self::Selection => "selection",
            Self::OnPrimary => "on_primary",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.name() == name)
    }
}

/// An sRGB colour, written `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
        })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// The colours `theme.toml` replaces, per theme.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    pub light: BTreeMap<ThemeToken, Rgb>,
    pub dark: BTreeMap<ThemeToken, Rgb>,
}

/// Checks the text of `theme.toml`.
pub fn parse_theme(text: &str) -> Checked<ThemeOverrides> {
    let mut checked = Checked::<ThemeOverrides>::default();
    let table = match toml_table(text) {
        Ok(table) => table,
        Err(problem) => {
            checked.problems.push(problem);
            return checked;
        }
    };
    for (theme, value) in table {
        let colours = match theme.as_str() {
            "light" => &mut checked.value.light,
            "dark" => &mut checked.value.dark,
            _ => {
                checked
                    .problems
                    .push(format!("{theme:?} is not a theme; use [light] and [dark]"));
                continue;
            }
        };
        let toml::Value::Table(entries) = value else {
            checked
                .problems
                .push(format!("{theme} must be a table of colours, as [{theme}]"));
            continue;
        };
        for (name, value) in entries {
            match colour(&name, &value) {
                Ok((token, rgb)) => {
                    colours.insert(token, rgb);
                }
                Err(problem) => checked.problems.push(format!("{theme}.{name}: {problem}")),
            }
        }
    }
    checked
}

fn colour(name: &str, value: &toml::Value) -> Result<(ThemeToken, Rgb), String> {
    let token = ThemeToken::parse(name).ok_or_else(|| {
        let names: Vec<&str> = ThemeToken::ALL.into_iter().map(ThemeToken::name).collect();
        format!("not a colour of the palette; they are {}", names.join(", "))
    })?;
    let rgb = value
        .as_str()
        .and_then(Rgb::parse)
        .ok_or_else(|| format!("{value} is not a colour; write it as \"#rrggbb\""))?;
    Ok((token, rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_colours_for_each_theme() {
        let checked = parse_theme(
            "[light]\nprimary = \"#5b3fe0\"\n\n[dark]\nprimary = \"#7C5CFF\"\nbase = \"#14121a\"\n",
        );

        assert!(checked.problems.is_empty(), "{:?}", checked.problems);
        let theme = checked.value;
        assert_eq!(
            theme.light.get(&ThemeToken::Primary),
            Some(&Rgb {
                r: 0x5b,
                g: 0x3f,
                b: 0xe0
            })
        );
        assert_eq!(theme.dark[&ThemeToken::Primary].to_string(), "#7c5cff");
        assert_eq!(theme.dark[&ThemeToken::Base].to_string(), "#14121a");
        assert_eq!(theme.light.len(), 1);
    }

    #[test]
    fn skips_and_names_each_broken_colour() {
        let cases = [
            (
                "base = \"dark\"\n",
                "\"base\" is not a theme; use [light] and [dark]",
            ),
            (
                "light = \"#ffffff\"\n",
                "light must be a table of colours, as [light]",
            ),
            (
                "[dark]\nbackground = \"#000000\"\n",
                "dark.background: not a colour of the palette; they are primary, secondary, \
                 success, warning, error, base, panel, field, control, control_hover, border, \
                 text, text_weak, selection, on_primary",
            ),
            (
                "[dark]\nprimary = \"#7c5cf\"\n",
                "dark.primary: \"#7c5cf\" is not a colour; write it as \"#rrggbb\"",
            ),
            (
                "[light]\ntext = 0x000000\n",
                "light.text: 0 is not a colour; write it as \"#rrggbb\"",
            ),
            (
                "[light]\ntext = \"#gg0000\"\n",
                "light.text: \"#gg0000\" is not a colour; write it as \"#rrggbb\"",
            ),
        ];
        for (text, problem) in cases {
            let checked = parse_theme(text);
            assert_eq!(checked.problems, [problem], "{text}");
            assert_eq!(checked.value, ThemeOverrides::default(), "{text}");
        }
    }
}
