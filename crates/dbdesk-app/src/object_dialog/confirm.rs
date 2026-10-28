//! What a destructive change (Drop, Truncate) asks before Execute: it names the object and
//! what goes with it, and on a data source coloured red, the colour that marks production,
//! the user types the object's name.

use egui::{TextEdit, Ui};

use super::Target;
use crate::theme::Palette;

const NAME_FIELD_WIDTH: f32 = 240.0;

/// What a change destroys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destruction {
    /// The name the user types to confirm, as the explorer shows it.
    pub name: String,
    /// What happens, naming the object: "Drops the table shop.orders, with every row in it."
    pub summary: String,
    /// What else goes with it, such as what `CASCADE` drops, if anything does.
    pub consequences: Vec<String>,
}

/// What the user typed to confirm.
#[derive(Debug, Default)]
pub(super) struct Confirmation {
    typed: String,
}

impl Confirmation {
    /// Draws the confirmation. Returns whether Execute may run.
    pub(super) fn show(&mut self, ui: &mut Ui, destruction: &Destruction, target: &Target) -> bool {
        let palette = &Palette::for_visuals(ui.visuals());
        ui.colored_label(palette.warning, &destruction.summary);
        for consequence in &destruction.consequences {
            ui.label(consequence);
        }
        if !target.red {
            return true;
        }
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(format!(
                "\u{201c}{}\u{201d} is marked red. Type {} to confirm:",
                target.name, destruction.name
            ));
            ui.add(
                TextEdit::singleline(&mut self.typed)
                    .desired_width(NAME_FIELD_WIDTH)
                    .hint_text(&destruction.name),
            )
            .labelled_by(label.id);
        });
        confirms(&self.typed, &destruction.name)
    }
}

/// Exactly the name, ignoring spaces around it: a name that differs in case may be another
/// object.
fn confirms(typed: &str, name: &str) -> bool {
    typed.trim() == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_name_confirms() {
        let cases = [
            ("orders", true),
            (" orders ", true),
            ("Orders", false),
            ("order", false),
            ("", false),
        ];
        for (typed, expected) in cases {
            assert_eq!(confirms(typed, "orders"), expected, "{typed:?}");
        }
    }
}
