//! Password generator: uniform choice from the selected character classes using the OS CSPRNG,
//! with rejection sampling (no modulo bias) and at least one character from each class.

use zeroize::Zeroizing;

use crate::random_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratorOptions {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Drops look-alikes such as `l`, `1`, `O` and `0`.
    pub avoid_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 24,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
        }
    }
}

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.<>/?~";
const AMBIGUOUS: &str = "lI1O0o|";

impl GeneratorOptions {
    fn classes(&self) -> Vec<Vec<char>> {
        [
            (self.lowercase, LOWER),
            (self.uppercase, UPPER),
            (self.digits, DIGITS),
            (self.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, set)| {
            set.chars()
                .filter(|c| !(self.avoid_ambiguous && AMBIGUOUS.contains(*c)))
                .collect()
        })
        .collect()
    }

    /// Entropy in bits of a password from these options (ignoring the one-per-class rule).
    pub fn entropy_bits(&self) -> f64 {
        let n: usize = self.classes().iter().map(Vec::len).sum();
        if n < 2 {
            0.0
        } else {
            self.length as f64 * (n as f64).log2()
        }
    }
}

pub fn generate(opts: &GeneratorOptions) -> Zeroizing<String> {
    let classes = opts.classes();
    if classes.is_empty() || opts.length == 0 {
        return Zeroizing::new(String::new());
    }
    let all: Vec<char> = classes.concat();
    let length = opts.length.max(classes.len());

    loop {
        let mut chars: Vec<char> = (0..length).map(|_| all[uniform(all.len())]).collect();
        // Regenerate rather than patch, so every valid password stays equally likely.
        if classes
            .iter()
            .all(|class| chars.iter().any(|c| class.contains(c)))
        {
            let out = Zeroizing::new(chars.iter().collect());
            chars.iter_mut().for_each(|c| *c = '\0');
            return out;
        }
    }
}

/// Uniform integer in `0..n` by rejection sampling over u32.
fn uniform(n: usize) -> usize {
    let n = n as u32;
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        random_bytes(&mut b);
        let v = u32::from_le_bytes(b);
        if v < zone {
            return (v % n) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_length_and_classes() {
        let opts = GeneratorOptions {
            length: 40,
            ..Default::default()
        };
        for _ in 0..200 {
            let p = generate(&opts);
            assert_eq!(p.chars().count(), 40);
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            assert!(p.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn avoids_ambiguous_and_disabled_classes() {
        let opts = GeneratorOptions {
            length: 64,
            symbols: false,
            avoid_ambiguous: true,
            ..Default::default()
        };
        for _ in 0..100 {
            let p = generate(&opts);
            assert!(
                !p.chars()
                    .any(|c| AMBIGUOUS.contains(c) || SYMBOLS.contains(c))
            );
        }
    }

    #[test]
    fn entropy_estimate() {
        let opts = GeneratorOptions {
            length: 20,
            lowercase: true,
            uppercase: false,
            digits: false,
            symbols: false,
            avoid_ambiguous: false,
        };
        assert!((opts.entropy_bits() - 20.0 * 26f64.log2()).abs() < 1e-9);
    }
}
