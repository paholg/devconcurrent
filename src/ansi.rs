use crossterm::style::{Attribute, Color, SetAttribute, SetForegroundColor};

pub(crate) const RESET: SetAttribute = SetAttribute(Attribute::Reset);

pub(crate) const GRAY: SetForegroundColor = SetForegroundColor(Color::DarkGrey);
pub(crate) const RED: SetForegroundColor = SetForegroundColor(Color::Red);
pub(crate) const GREEN: SetForegroundColor = SetForegroundColor(Color::Green);
pub(crate) const YELLOW: SetForegroundColor = SetForegroundColor(Color::Yellow);
pub(crate) const BLUE: SetForegroundColor = SetForegroundColor(Color::Blue);
pub(crate) const MAGENTA: SetForegroundColor = SetForegroundColor(Color::Magenta);
pub(crate) const CYAN: SetForegroundColor = SetForegroundColor(Color::Cyan);
