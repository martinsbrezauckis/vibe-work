//! sRGB colour arithmetic for the design capture: parsing the `#rrggbb[aa]` strings the page script
//! normalises every colour to, WCAG 2.x relative luminance and contrast ratio, HSL saturation/lightness for "is this a
//! brand colour or a grey", and a distance to tell near-duplicates apart.
//!
//! Depends on: std. Called by: `capture::design`, tests.
//! Invariant: only strings of exactly 7 or 9 characters `#` + hex digits parse; anything else is `None` (page data).

/// An sRGB colour with alpha, 8 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (255 = opaque).
    pub a: u8,
}

impl Rgba {
    /// Parses `#rrggbb` or `#rrggbbaa` (lowercase or uppercase).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if hex.len() == 8 { byte(6)? } else { 255 },
        })
    }

    /// `#rrggbb` (alpha dropped) or `#rrggbbaa` when not opaque.
    #[must_use]
    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// True when fully opaque.
    #[must_use]
    pub fn opaque(self) -> bool {
        self.a == 255
    }

    /// Channels as 0..=1 floats.
    fn unit(self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|c| f64::from(c) / 255.0)
    }

    /// WCAG 2.x relative luminance.
    #[must_use]
    pub fn luminance(self) -> f64 {
        let [r, g, b] = self.unit().map(|c| {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// HSL saturation (0..=1).
    #[must_use]
    pub fn saturation(self) -> f64 {
        let [r, g, b] = self.unit();
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let l = f64::midpoint(max, min);
        if (max - min).abs() < f64::EPSILON {
            0.0
        } else {
            (max - min) / (1.0 - (2.0 * l - 1.0).abs())
        }
    }

    /// HSL lightness (0..=1).
    #[must_use]
    pub fn lightness(self) -> f64 {
        let [r, g, b] = self.unit();
        f64::midpoint(r.max(g).max(b), r.min(g).min(b))
    }

    /// A colour that reads as a hue (a brand or accent colour), not a grey, near-black or near-white.
    #[must_use]
    pub fn chromatic(self) -> bool {
        let l = self.lightness();
        self.saturation() >= 0.25 && (0.12..=0.92).contains(&l)
    }

    /// Euclidean distance in RGB (0..=441).
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        let d = |a: u8, b: u8| (f64::from(a) - f64::from(b)).powi(2);
        (d(self.r, other.r) + d(self.g, other.g) + d(self.b, other.b)).sqrt()
    }

    /// The same colour opaque (alpha dropped).
    #[must_use]
    pub fn solid(self) -> Self {
        Self { a: 255, ..self }
    }
}

/// WCAG 2.x contrast ratio of two opaque colours (1..=21).
#[must_use]
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (light, dark) = if la >= lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}
