use console::{Color, style};

struct Unit<'a> {
    value: f32,
    name: &'a str,
    color: Color,
}

impl Unit<'_> {
    const fn new(value: f32, name: &str, color: Color) -> Unit<'_> {
        Unit { value, name, color }
    }
}

const K: f32 = 1000.0;
const M: f32 = 1000.0 * K;
const G: f32 = 1000.0 * M;
const T: f32 = 1000.0 * G;
const P: f32 = 1000.0 * T;
const E: f32 = 1000.0 * P;

const BYTE_UNITS: [Unit; 6] = [
    Unit::new(K, "k", Color::Blue),
    Unit::new(M, "M", Color::Cyan),
    Unit::new(G, "G", Color::Magenta),
    Unit::new(T, "T", Color::Red),
    Unit::new(P, "P", Color::Red),
    Unit::new(E, "E", Color::Red),
];

/// A byte count, rendered with a unit and color.
#[derive(Clone, Copy)]
pub(crate) struct Bytes(pub(crate) u64);

impl std::fmt::Display for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes = self.0 as f32;
        let unit = BYTE_UNITS
            .iter()
            .take_while(|unit| unit.value <= bytes)
            .last()
            .unwrap_or(&BYTE_UNITS[0]);

        let value = bytes / unit.value;
        let n_decimals = if value < 10.0 {
            2
        } else {
            usize::from(value < 100.0)
        };

        let decimal_point = if n_decimals == 0 { "." } else { "" };
        let text = format!("{value:.n_decimals$}{decimal_point} {}", unit.name);

        write!(f, "{}", style(text).fg(unit.color).bright())
    }
}
