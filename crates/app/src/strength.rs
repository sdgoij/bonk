//! A deliberately small passphrase strength estimate.
//!
//! This is not a password cracker in reverse and it does not pretend to be
//! one. It looks at three things a person can act on — length, character
//! variety, and a short list of phrases that are tried first — and says whether
//! the phrase is worth the protection it is being asked to provide. Argon2id
//! makes each guess expensive; nothing can make a guessable phrase safe.
//!
//! The estimate is a floor, not a measurement: `bits` counts the characters
//! present and assumes nothing about the order, which is why a repeated pattern
//! is collapsed to its period before the count.

/// How much protection a passphrase appears to offer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Strength {
    /// Too short, too common, or too repetitive to rely on.
    Weak,
    /// Worth using, but only just.
    Fair,
    /// Long or varied enough that guessing is expensive.
    Strong,
}

impl Strength {
    /// A short word for a label.
    pub fn label(self) -> &'static str {
        match self {
            Strength::Weak => "weak",
            Strength::Fair => "fair",
            Strength::Strong => "strong",
        }
    }
}

/// The result of [`assess`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Assessment {
    /// The level to show.
    pub strength: Strength,
    /// A floor on the search space, in bits.
    pub bits: u32,
    /// One sentence the user can act on.
    pub reason: &'static str,
}

/// Phrases that are tried before anything clever.
///
/// Matched against the whole value, and against its letters-only and
/// alphanumeric forms, so `Password123!` does not slip past.
const COMMON: &[&str] = &[
    "123456",
    "12345678",
    "123456789",
    "abc123",
    "admin",
    "bonk",
    "changeme",
    "dragon",
    "football",
    "iloveyou",
    "letmein",
    "monkey",
    "passphrase",
    "password",
    "passw0rd",
    "qwerty",
    "qwertyuiop",
    "secret",
    "trustno1",
    "welcome",
];

/// Below this many bits the phrase is called weak.
const WEAK_BELOW: u32 = 50;

/// From this many bits the phrase is called strong.
const STRONG_FROM: u32 = 70;

/// Estimates the protection a passphrase offers.
#[must_use]
pub fn assess(passphrase: &str) -> Assessment {
    let characters: Vec<char> = passphrase.chars().collect();
    if characters.iter().all(|character| character.is_whitespace()) {
        return Assessment {
            strength: Strength::Weak,
            bits: 0,
            reason: "enter a passphrase",
        };
    }

    if is_common(passphrase) {
        return Assessment {
            strength: Strength::Weak,
            bits: 0,
            reason: "this is one of the first phrases an attacker tries",
        };
    }

    // A repeated pattern is worth its period, not its length.
    let period = smallest_period(&characters);
    let effective = u32::try_from(period).unwrap_or(u32::MAX);
    let bits = effective.saturating_mul(per_character_bits(&characters));

    if period < characters.len() && bits < STRONG_FROM {
        return Assessment {
            strength: Strength::Weak,
            bits,
            reason: "repeating a short pattern adds almost nothing",
        };
    }
    if bits < WEAK_BELOW {
        return Assessment {
            strength: Strength::Weak,
            bits,
            reason: "too short; a few unrelated words beat a short scramble",
        };
    }
    if bits < STRONG_FROM {
        return Assessment {
            strength: Strength::Fair,
            bits,
            reason: "acceptable; length helps more than clever substitutions",
        };
    }
    Assessment {
        strength: Strength::Strong,
        bits,
        reason: "good: length and variety both count",
    }
}

/// True when the value matches a phrase from [`COMMON`] in any plain form.
fn is_common(value: &str) -> bool {
    let lower = value.trim().to_lowercase();
    let alphanumeric: String = lower.chars().filter(|character| character.is_alphanumeric()).collect();
    let letters: String = lower.chars().filter(|character| character.is_alphabetic()).collect();
    COMMON
        .iter()
        .any(|entry| *entry == lower || *entry == alphanumeric || (!letters.is_empty() && *entry == letters))
}

/// The shortest block the value is built from, which is the whole value when it
/// has no shorter period.
fn smallest_period(characters: &[char]) -> usize {
    let length = characters.len();
    for period in 1..=length / 2 {
        let repeats = characters
            .iter()
            .enumerate()
            .all(|(index, character)| *character == characters[index % period]);
        if repeats {
            return period;
        }
    }
    length
}

/// Floor of `log2(alphabet)`: the bits one character of this alphabet is worth.
fn per_character_bits(characters: &[char]) -> u32 {
    let mut alphabet = 0u32;
    let mut lower = false;
    let mut upper = false;
    let mut digit = false;
    let mut other = false;
    for character in characters {
        if character.is_lowercase() {
            lower = true;
        } else if character.is_uppercase() {
            upper = true;
        } else if character.is_ascii_digit() {
            digit = true;
        } else {
            other = true;
        }
    }
    if lower {
        alphabet += 26;
    }
    if upper {
        alphabet += 26;
    }
    if digit {
        alphabet += 10;
    }
    if other {
        alphabet += 33;
    }
    // `leading_zeros` of a non-zero value is the complement of its floor log2.
    31 - alphabet.max(1).leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_blank_passphrase_is_weak() {
        assert_eq!(assess("").strength, Strength::Weak);
        assert_eq!(assess("   ").strength, Strength::Weak);
        assert_eq!(assess("").bits, 0);
    }

    #[test]
    fn common_phrases_are_caught_even_with_decorations() {
        for value in [
            "password",
            "Password123!",
            "Passw0rd",
            "letmein",
            "qwerty",
            "bonk",
            "Secret42",
        ] {
            assert_eq!(assess(value).strength, Strength::Weak, "{value}");
        }
    }

    #[test]
    fn repetition_does_not_buy_length() {
        let repeated = assess("abcabcabcabcabcabcabc");
        assert_eq!(repeated.strength, Strength::Weak);
        assert!(repeated.bits < 30, "period 3 over alphabet 26 is 12 bits");
        assert_eq!(assess("aaaaaaaaaaaaaaaaaaaaaaaa").strength, Strength::Weak);
    }

    #[test]
    fn length_beats_substitution() {
        assert_eq!(assess("correct horse battery staple").strength, Strength::Strong);
        assert_eq!(assess("Tr0ub4dor&3").strength, Strength::Fair);
    }

    #[test]
    fn a_short_scramble_is_not_strong() {
        assert_eq!(assess("hunter2").strength, Strength::Weak);
        assert_eq!(assess("Xk9#mQ2").strength, Strength::Weak);
    }

    #[test]
    fn labels_follow_the_level() {
        assert_eq!(Strength::Weak.label(), "weak");
        assert_eq!(Strength::Fair.label(), "fair");
        assert_eq!(Strength::Strong.label(), "strong");
        assert!(Strength::Weak < Strength::Strong);
    }
}
