//! User-facing interface text.
//!
//! English first. Every string the shell shows lives here rather than in the
//! widget call, so adding a language means a second table and a lookup, not a
//! hunt through the drawing code. Text the user wrote — invoice descriptions,
//! notes, customer names — is theirs and never comes from here.
//!
//! The `_` parameter on each function is the language it will be selected by;
//! until a second table exists every function ignores it, and the call sites do
//! not change when one arrives.

use crate::strength::{Reason, Strength};

/// Languages the interface can speak.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// The only table so far.
    #[default]
    English,
}

/// The lock screen.
pub mod shell {
    use super::Language;

    /// The product's working name. A name does not translate.
    pub const PRODUCT: &str = "Bonk";

    /// The line under the name, before anything is unlocked.
    pub fn tagline(_: Language) -> &'static str {
        "Local-first invoicing. Nothing leaves this machine unless you send it."
    }

    /// Label above the database path.
    pub fn database_label(_: Language) -> &'static str {
        "Database"
    }

    /// Label above the keystore path.
    pub fn keystore_label(_: Language) -> &'static str {
        "Keystore"
    }

    /// Label above the passphrase field.
    pub fn passphrase_label(_: Language) -> &'static str {
        "Passphrase"
    }

    /// Opens an existing archive.
    pub fn unlock_action(_: Language) -> &'static str {
        "Unlock"
    }

    /// Starts creating one.
    pub fn create_database_action(_: Language) -> &'static str {
        "Create new database"
    }

    /// Switches to the recovery route.
    pub fn forgotten_action(_: Language) -> &'static str {
        "Forgotten the passphrase?"
    }

    /// Explains what creating an archive does.
    pub fn create_note(_: Language) -> &'static str {
        "A new archive is created empty and sealed with this passphrase. Nothing else can open it, so type it twice."
    }

    /// Label above the first field when creating.
    pub fn new_passphrase_label(_: Language) -> &'static str {
        "New passphrase"
    }

    /// Label above the second field when creating.
    pub fn repeat_passphrase_label(_: Language) -> &'static str {
        "Repeat the passphrase"
    }

    /// Confirms a creation.
    pub fn create_action(_: Language) -> &'static str {
        "Create"
    }

    /// Leaves the current form.
    pub fn back_action(_: Language) -> &'static str {
        "Back"
    }

    /// Explains the recovery route.
    pub fn reset_note(_: Language) -> &'static str {
        "A recovery key opens this archive without the passphrase. The key used here is replaced by the reset, because it has now been typed somewhere."
    }

    /// Label above the recovery-key field.
    pub fn recovery_key_label(_: Language) -> &'static str {
        "Recovery key"
    }

    /// Label above the second field when resetting.
    pub fn repeat_new_passphrase_label(_: Language) -> &'static str {
        "Repeat the new passphrase"
    }

    /// Performs the reset.
    pub fn reset_action(_: Language) -> &'static str {
        "Reset passphrase"
    }

    /// Tells the user the reset happened and the old key is spent.
    pub fn reset_done_note(_: Language) -> &'static str {
        "Passphrase replaced. The recovery key that was used no longer works."
    }

    /// Introduces the replacement key.
    pub fn new_recovery_key_label(_: Language) -> &'static str {
        "New recovery key, shown once:"
    }

    /// Tells the user what to do with a key.
    pub fn keep_key_note(_: Language) -> &'static str {
        "Write it down and keep it apart from the database and the passphrase."
    }

    /// Acknowledges the replacement key.
    pub fn written_down_action(_: Language) -> &'static str {
        "I have written it down"
    }
}

/// The strength estimate shown under a passphrase field.
pub mod strength {
    use super::{Language, Reason, Strength};

    /// Label before the level itself.
    pub fn label(_: Language) -> &'static str {
        "Strength"
    }

    /// The level, as one word.
    pub fn level(_: Language, strength: Strength) -> &'static str {
        match strength {
            Strength::Weak => "weak",
            Strength::Fair => "fair",
            Strength::Strong => "strong",
        }
    }

    /// One sentence of advice for the level.
    pub fn reason(_: Language, reason: Reason) -> &'static str {
        match reason {
            Reason::Empty => "enter a passphrase",
            Reason::Common => "this is one of the first phrases an attacker tries",
            Reason::Repetition => "repeating a short pattern adds almost nothing",
            Reason::TooShort => "too short; a few unrelated words beat a short scramble",
            Reason::Fair => "acceptable; length helps more than clever substitutions",
            Reason::Strong => "good: length and variety both count",
        }
    }

    /// The hover text, which explains what the number means.
    pub fn detail(language: Language, bits: u32) -> String {
        match language {
            Language::English => format!(
                "At least {bits} bits by this estimate. Every guess costs the attacker one Argon2id run, so \
                 length is what makes a copied keystore expensive to open."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_has_its_own_word() {
        let language = Language::default();
        let words = [
            strength::level(language, Strength::Weak),
            strength::level(language, Strength::Fair),
            strength::level(language, Strength::Strong),
        ];
        assert_eq!(words, ["weak", "fair", "strong"]);
        assert!(words.iter().all(|word| !word.is_empty()));
    }

    #[test]
    fn every_reason_says_something() {
        let language = Language::default();
        for reason in [
            Reason::Empty,
            Reason::Common,
            Reason::Repetition,
            Reason::TooShort,
            Reason::Fair,
            Reason::Strong,
        ] {
            let text = strength::reason(language, reason);
            assert!(!text.is_empty(), "{reason:?} has no sentence");
            assert!(!text.ends_with(' '), "{reason:?} ends with a stray space");
        }
    }

    #[test]
    fn shell_text_is_never_empty() {
        let language = Language::default();
        for text in [
            shell::tagline(language),
            shell::database_label(language),
            shell::keystore_label(language),
            shell::passphrase_label(language),
            shell::unlock_action(language),
            shell::create_database_action(language),
            shell::forgotten_action(language),
            shell::create_note(language),
            shell::new_passphrase_label(language),
            shell::repeat_passphrase_label(language),
            shell::create_action(language),
            shell::back_action(language),
            shell::reset_note(language),
            shell::recovery_key_label(language),
            shell::repeat_new_passphrase_label(language),
            shell::reset_action(language),
            shell::reset_done_note(language),
            shell::new_recovery_key_label(language),
            shell::keep_key_note(language),
            shell::written_down_action(language),
        ] {
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn the_detail_mentions_the_number_it_was_given() {
        let detail = strength::detail(Language::English, 72);
        assert!(detail.contains("72"), "{detail}");
    }
}
