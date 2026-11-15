//! The unmatched queue as a keyboard-first surface.
//!
//! The queue is the daily work surface: a list of bank lines nothing has settled,
//! and for the one line that is open, the ways it could be settled. Doing that
//! with a mouse is fine; doing it for forty lines is not, so every step has a key.
//!
//! The rules live here rather than in the drawing code, because a queue where
//! Enter silently does nothing, or where the highlight walks off the end, is a bug
//! you find at the worst possible moment. What is here is a small state machine
//! with no egui in it; the window asks it what a key meant and then does that.
//!
//! Two steps, on purpose: the line is opened, and only then can one of its offers
//! be accepted. A single Enter that moved money because a list happened to have a
//! row under the pointer is not a keyboard shortcut, it is a hazard.

use crux::{Money, SellerId};
use store::{Database, StoreError};

/// What a key means to the queue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Key {
    /// The previous line, or the previous offer once the line is open.
    Up,
    /// The next line, or the next offer once the line is open.
    Down,
    /// Enter the offers, or accept the highlighted one.
    Enter,
    /// Move between the lines and the offers.
    Tab,
    /// Leave the offers, then close the line.
    Escape,
}

/// Where the keyboard is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Focus {
    /// On the lines.
    #[default]
    List,
    /// On the offers of the open line.
    Offers,
}

/// One way to settle the open line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Offer {
    /// The open items this would settle, in document order.
    pub(crate) item_ids: Vec<String>,
    /// What a person reads: `2026-0001`, or `2026-0001 + 2026-0002`.
    pub(crate) number: String,
    /// What the payment would settle.
    pub(crate) amount: Money,
    /// Why it is offered.
    pub(crate) method: store::MatchMethod,
    /// The sentence that goes with it.
    pub(crate) reason: String,
}

/// What the window should do about a key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    /// Nothing.
    None,
    /// Open this line, which means loading its offers.
    Open(String),
    /// Accept an offer on the open line.
    Accept {
        /// The line.
        transaction: String,
        /// Which of its offers, by index.
        offer: usize,
    },
    /// Close the open line.
    Close,
}

/// Which line is open, which offer is highlighted, and where the keyboard is.
#[derive(Default, Debug)]
pub(crate) struct Queue {
    opened: Option<String>,
    offers: Vec<Offer>,
    highlight: usize,
    focus: Focus,
    note: Option<String>,
}

impl Queue {
    /// The line whose offers are showing.
    pub(crate) fn opened(&self) -> Option<&str> {
        self.opened.as_deref()
    }

    /// True when this line's offers are showing.
    pub(crate) fn is_open(&self, transaction: &str) -> bool {
        self.opened.as_deref() == Some(transaction)
    }

    /// The offers of the open line.
    pub(crate) fn offers(&self) -> &[Offer] {
        &self.offers
    }

    /// The highlighted offer, when the keyboard is on them.
    pub(crate) fn highlight(&self) -> usize {
        self.highlight
    }

    /// Where the keyboard is.
    pub(crate) fn focus(&self) -> Focus {
        self.focus
    }

    /// Something to say about the open line, when its offers could not be read.
    pub(crate) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Opens a line with the offers it turned out to have.
    ///
    /// The keyboard stays where it was: a person arrowing down the list is working
    /// the lines, and one who asked for the offers has already moved. It falls back
    /// to the list when there is nothing to move to.
    pub(crate) fn open(&mut self, transaction: &str, offers: Vec<Offer>) {
        self.opened = Some(transaction.to_owned());
        self.offers = offers;
        self.highlight = 0;
        self.note = None;
        if self.offers.is_empty() {
            self.focus = Focus::List;
        }
    }

    /// Records that a line's offers could not be read.
    pub(crate) fn fail(&mut self, transaction: &str, note: String) {
        self.opened = Some(transaction.to_owned());
        self.offers = Vec::new();
        self.highlight = 0;
        self.focus = Focus::List;
        self.note = Some(note);
    }

    /// Closes the open line.
    pub(crate) fn close(&mut self) {
        self.opened = None;
        self.offers = Vec::new();
        self.highlight = 0;
        self.focus = Focus::List;
        self.note = None;
    }

    /// Reads a key against the lines as they are drawn.
    ///
    /// `lines` is the order the window shows, after any sort, so arrowing down
    /// always means the row below the one on screen.
    pub(crate) fn press(&mut self, key: Key, lines: &[String]) -> Action {
        match key {
            Key::Down => match self.opened.as_deref() {
                None => lines.first().map_or(Action::None, |id| Action::Open(id.clone())),
                Some(_) if self.focus == Focus::Offers => {
                    self.move_highlight(1);
                    Action::None
                }
                Some(current) => match neighbours(lines, current).1 {
                    Some(next) => Action::Open(next),
                    None => Action::None,
                },
            },
            Key::Up => match self.opened.as_deref() {
                None => Action::None,
                Some(_) if self.focus == Focus::Offers => {
                    self.move_highlight(-1);
                    Action::None
                }
                Some(current) => match neighbours(lines, current).0 {
                    Some(previous) => Action::Open(previous),
                    // The arrows never close anything: backing out is Escape's job.
                    None => Action::None,
                },
            },
            Key::Enter => match self.focus {
                Focus::List => {
                    if self.offers.is_empty() {
                        return Action::None;
                    }
                    self.focus = Focus::Offers;
                    self.highlight = self.highlight.min(self.offers.len() - 1);
                    Action::None
                }
                Focus::Offers => match self.opened.clone() {
                    Some(transaction) if self.offers.get(self.highlight).is_some() => Action::Accept {
                        transaction,
                        offer: self.highlight,
                    },
                    _ => Action::None,
                },
            },
            Key::Tab => {
                if self.opened.is_some() && !self.offers.is_empty() {
                    self.focus = match self.focus {
                        Focus::List => Focus::Offers,
                        Focus::Offers => Focus::List,
                    };
                }
                Action::None
            }
            Key::Escape => match self.focus {
                Focus::Offers => {
                    self.focus = Focus::List;
                    self.highlight = 0;
                    Action::None
                }
                Focus::List => {
                    if self.opened.is_some() {
                        Action::Close
                    } else {
                        Action::None
                    }
                }
            },
        }
    }

    fn move_highlight(&mut self, by: i64) {
        if self.offers.is_empty() {
            return;
        }
        let last = self.offers.len() - 1;
        let moved = self.highlight as i64 + by;
        self.highlight = moved.clamp(0, last as i64) as usize;
    }
}

/// The lines either side of `current`, when it is in the list at all.
fn neighbours(lines: &[String], current: &str) -> (Option<String>, Option<String>) {
    match lines.iter().position(|id| id == current) {
        Some(index) => (
            index.checked_sub(1).and_then(|previous| lines.get(previous)).cloned(),
            lines.get(index + 1).cloned(),
        ),
        None => (None, None),
    }
}

/// Everything that could settle a line, strongest first.
///
/// Singles come before groups because a payment that settles one invoice is the
/// common case; a group is offered only when the remittance names every invoice in
/// it and the amounts add up exactly, which is what keeps a near miss a question.
pub(crate) fn offers(database: &Database, seller: SellerId, transaction: &str) -> Result<Vec<Offer>, StoreError> {
    let mut offers = Vec::new();
    for proposal in store::proposals(database.connection(), seller, transaction)? {
        offers.push(Offer {
            item_ids: vec![proposal.open_item_id],
            number: proposal.number,
            amount: proposal.applies,
            method: proposal.method,
            reason: proposal.reason,
        });
    }
    for group in store::group_proposals(database.connection(), seller, transaction)? {
        offers.push(Offer {
            item_ids: group.open_item_ids,
            number: group.numbers.join(" + "),
            amount: group.applies,
            method: store::MatchMethod::Group,
            reason: "one payment settles these together".to_owned(),
        });
    }
    Ok(offers)
}

/// Settles a line with one of its offers.
///
/// One item and several are the same operation from here: the store writes one
/// entry for the total either way, and a group's allocation per invoice is what
/// records which share belongs where.
pub(crate) fn accept(
    database: &Database,
    seller: SellerId,
    transaction: &str,
    offer: &Offer,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    match offer.item_ids.as_slice() {
        [only] => {
            store::allocate(
                database.connection(),
                seller,
                transaction,
                only,
                offer.method,
                actor,
                now,
            )?;
        }
        many if !many.is_empty() => {
            store::allocate_group(
                database.connection(),
                seller,
                transaction,
                many,
                store::MatchMethod::Group,
                actor,
                now,
            )?;
        }
        _ => {
            return Err(StoreError::Rejected(
                "an offer with no open item cannot be accepted".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn offer(number: &str) -> Offer {
        Offer {
            item_ids: vec![format!("item-{number}")],
            number: number.to_owned(),
            amount: Money::from_micros(1_000_000, crux::Currency::EUR),
            method: store::MatchMethod::ExactAmount,
            reason: "the amount matches".to_owned(),
        }
    }

    fn opened(numbers: &[&str]) -> Queue {
        let mut queue = Queue::default();
        queue.open("t1", numbers.iter().map(|number| offer(number)).collect());
        queue
    }

    #[test]
    fn down_opens_the_first_line_then_walks_it() {
        let mut queue = Queue::default();
        let list = lines(&["t1", "t2", "t3"]);
        assert_eq!(queue.press(Key::Down, &list), Action::Open("t1".to_owned()));
        queue.open("t1", Vec::new());
        assert_eq!(queue.press(Key::Down, &list), Action::Open("t2".to_owned()));
    }

    #[test]
    fn the_arrows_do_not_wrap_and_do_not_close() {
        let mut queue = Queue::default();
        let list = lines(&["t1", "t2"]);
        queue.open("t1", Vec::new());
        assert_eq!(queue.press(Key::Up, &list), Action::None, "no wrap to the end");
        queue.open("t2", Vec::new());
        assert_eq!(queue.press(Key::Down, &list), Action::None, "no wrap to the start");
        assert_eq!(queue.opened(), Some("t2"), "and the line stays open");
    }

    #[test]
    fn a_line_that_is_no_longer_listed_goes_nowhere() {
        let mut queue = Queue::default();
        queue.open("gone", Vec::new());
        assert_eq!(queue.press(Key::Down, &lines(&["t1"])), Action::None);
        assert_eq!(queue.press(Key::Up, &lines(&["t1"])), Action::None);
    }

    #[test]
    fn enter_moves_into_the_offers_before_it_accepts_anything() {
        let mut queue = opened(&["2026-0001", "2026-0002"]);
        assert_eq!(queue.focus(), Focus::List);
        assert_eq!(queue.press(Key::Enter, &lines(&["t1"])), Action::None);
        assert_eq!(queue.focus(), Focus::Offers);
        assert_eq!(
            queue.press(Key::Enter, &lines(&["t1"])),
            Action::Accept {
                transaction: "t1".to_owned(),
                offer: 0,
            }
        );
    }

    #[test]
    fn the_arrows_move_the_highlight_once_the_keyboard_is_on_the_offers() {
        let mut queue = opened(&["a", "b", "c"]);
        queue.press(Key::Enter, &lines(&["t1"]));
        assert_eq!(queue.highlight(), 0);
        queue.press(Key::Down, &lines(&["t1"]));
        assert_eq!(queue.highlight(), 1);
        queue.press(Key::Down, &lines(&["t1"]));
        queue.press(Key::Down, &lines(&["t1"]));
        assert_eq!(queue.highlight(), 2, "the highlight stops at the last offer");
        queue.press(Key::Up, &lines(&["t1"]));
        assert_eq!(queue.highlight(), 1);
        assert_eq!(
            queue.press(Key::Enter, &lines(&["t1"])),
            Action::Accept {
                transaction: "t1".to_owned(),
                offer: 1,
            }
        );
    }

    #[test]
    fn escape_leaves_the_offers_then_the_line() {
        let mut queue = opened(&["a"]);
        queue.press(Key::Enter, &lines(&["t1"]));
        assert_eq!(queue.press(Key::Escape, &lines(&["t1"])), Action::None);
        assert_eq!(queue.focus(), Focus::List);
        assert_eq!(queue.press(Key::Escape, &lines(&["t1"])), Action::Close);
        queue.close();
        assert_eq!(queue.opened(), None);
        assert_eq!(queue.press(Key::Escape, &lines(&["t1"])), Action::None);
    }

    #[test]
    fn a_line_with_nothing_to_offer_keeps_the_keyboard_on_the_list() {
        let mut queue = Queue::default();
        queue.open("t1", Vec::new());
        assert_eq!(queue.focus(), Focus::List);
        assert_eq!(queue.press(Key::Enter, &lines(&["t1"])), Action::None);
        assert_eq!(queue.press(Key::Tab, &lines(&["t1"])), Action::None);
        assert_eq!(queue.focus(), Focus::List);
    }

    #[test]
    fn tab_moves_between_the_two_panes() {
        let mut queue = opened(&["a"]);
        queue.press(Key::Tab, &lines(&["t1"]));
        assert_eq!(queue.focus(), Focus::Offers);
        queue.press(Key::Tab, &lines(&["t1"]));
        assert_eq!(queue.focus(), Focus::List);
    }

    #[test]
    fn opening_another_line_starts_its_offers_at_the_top() {
        let mut queue = opened(&["a", "b"]);
        queue.press(Key::Enter, &lines(&["t1"]));
        queue.press(Key::Down, &lines(&["t1"]));
        assert_eq!(queue.highlight(), 1);
        queue.open("t2", vec![offer("c")]);
        assert_eq!(queue.highlight(), 0);
        assert!(queue.is_open("t2"));
        assert!(!queue.is_open("t1"));
    }

    #[test]
    fn a_note_replaces_the_offers() {
        let mut queue = opened(&["a"]);
        queue.fail("t1", "the store said no".to_owned());
        assert!(queue.offers().is_empty());
        assert_eq!(queue.note(), Some("the store said no"));
        assert_eq!(queue.focus(), Focus::List);
    }
}
