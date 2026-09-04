//! The retention gate — whether a `CatalogVersion`'s manifest rows may be
//! collected (`design/10-retention-erasure.md`; **P-D-49**,
//! `dod-retention-gate`).
//!
//! # The domain is the version's own snapshot, never the registration rows
//!
//! **P-D-49**: the gate ranges over that version's `participant_set_snapshot`
//! and **not** over whatever registration rows exist. Both halves of that
//! sentence are corrections with a named failure behind them, and the `DoD`
//! states them:
//!
//! - Quantifying over **registrations** let an **empty ledger** satisfy the
//!   gate *vacuously* and collect a version nobody had frozen. So a snapshot
//!   member with **no registration row holds** the version — the freeze
//!   fan-out has not reached it yet, and absence is not release.
//! - An **empty snapshot** is collectable, because nobody ever owed an ack.
//!   That is not the same vacuity: the domain is empty by the version's own
//!   record rather than by a store the fan-out has not filled.
//!
//! # The two arms are a pair, and the timestamp alone is not one of them
//!
//! Every registration must satisfy `state = released`, **or**
//! `state = not_frozen(forced)` **and** `released_at` stamped. Reading the
//! **timestamp alone** collected a version holding live grandfathered
//! references, because nothing clears the stamp: a forced participant that
//! later recovered and acked leaves `state = acked` beside a live
//! `released_at` (P-D-67 — *"the state moving is what makes the stamp
//! inert"*). And reading the **state alone** would be wrong in the other
//! direction, because a door-released row carries `state = released` with the
//! stamp **NULL** while a forced row carries both — which is why
//! [`FreezeRegistration`] carries them separately rather than deriving one
//! from the other.
//!
//! # A held version is skipped, never forced
//!
//! C4: a candidate with a live registration is **skipped** with
//! `retention_orphan_blocked` — fail closed. This module answers the
//! predicate and names the reason; nothing here deletes, and no caller may
//! read a `Hold` as a soft warning.
//!
//! # What this module deliberately does not decide
//!
//! Its operand is `06-catalog-version`'s `inst-fz-liveness`, and that
//! feature's §7 rows 6, 11 and 33 hold the open half — whether
//! `freezeComplete`'s formula is restated to match this predicate, the
//! ledger's transition table, and who writes `released_at`. **Cited, not
//! re-raised**: this predicate reads the ledger as it ships and takes no
//! position on those three.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-retention-gate:p1
//!
//! The PII detector policy and its allow-list normalization are the second
//! half of this module (`inst-pp-detect`).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-pii-detector:p1

use crate::infra::storage::repo::FreezeRegistration;

use crate::domain::states::FreezeAckState;

/// Why a version's manifest rows are held back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetentionHold {
    /// A snapshot member has no registration row at all — the fan-out has not
    /// reached it, and absence is never release.
    NoRegistration {
        /// The participant the snapshot names.
        participant: String,
    },
    /// A registration exists and is live: neither released nor
    /// forced-with-a-stamp.
    LiveRegistration {
        /// The participant.
        participant: String,
        /// Its state, for the skip reason.
        state: FreezeAckState,
    },
    /// The forced arm without its stamp — the shape `CHECK` refuses this on
    /// both engines, so reaching it means a row was written past the guard.
    ForcedWithoutStamp {
        /// The participant.
        participant: String,
    },
}

impl RetentionHold {
    /// The participant this hold is about.
    #[must_use]
    pub fn participant(&self) -> &str {
        match self {
            Self::NoRegistration { participant }
            | Self::LiveRegistration { participant, .. }
            | Self::ForcedWithoutStamp { participant } => participant,
        }
    }

    /// The skip reason C4 names — **one constant for every arm**, because the
    /// requirement is that a held candidate is skipped and never forced,
    /// whatever holds it. As a const rather than a method, "every arm carries
    /// the same reason" is true by construction and needs no probe; the test
    /// that asserted it was a tautology and is deleted rather than kept.
    pub const REASON: &'static str = "retention_orphan_blocked";
}

/// The gate's verdict for one `CatalogVersion`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetentionVerdict {
    /// Every snapshot member is released, or the snapshot is empty.
    Collectable,
    /// At least one member holds the version. **Every** hold is reported, not
    /// the first: an operator repairing one and re-running would otherwise
    /// discover the rest one pass at a time.
    Held(Vec<RetentionHold>),
}

/// Evaluate the gate over one version's snapshot and its registrations.
///
/// `snapshot` is the participant set the version froze — the members of its
/// own `participant_set_snapshot`, already parsed. `registrations` is the
/// ledger as it stands.
#[must_use]
pub fn evaluate(snapshot: &[String], registrations: &[FreezeRegistration]) -> RetentionVerdict {
    let mut holds = Vec::new();
    for participant in snapshot {
        let Some(row) = registrations
            .iter()
            .find(|row| row.participant == *participant)
        else {
            holds.push(RetentionHold::NoRegistration {
                participant: participant.clone(),
            });
            continue;
        };
        match row.state {
            FreezeAckState::Released => {}
            FreezeAckState::NotFrozenForced if row.released_at_stamped => {}
            FreezeAckState::NotFrozenForced => holds.push(RetentionHold::ForcedWithoutStamp {
                participant: participant.clone(),
            }),
            state => holds.push(RetentionHold::LiveRegistration {
                participant: participant.clone(),
                state,
            }),
        }
    }
    if holds.is_empty() {
        RetentionVerdict::Collectable
    } else {
        RetentionVerdict::Held(holds)
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;

// -- The PII detector policy and its allow-list (`inst-pp-detect`,
//    `inst-pp-allowlist`; `dod-pii-detector`, `dod-pii-allowlist`) --

/// Normalize an allow-list value, and the detector's own candidate, to the
/// one form the exact match compares.
///
/// **This function is the whole of the match rule** (**P-D-117** item 23:
/// *"exact match on `value_normalized`"*), and both sides of the equality run
/// through it, so the stored column and the inspected text cannot drift apart
/// into two spellings of "normalized".
///
/// Four steps, in this order, each with its reason:
///
/// 1. **Unicode NFKC.** The compatibility decomposition is what makes a
///    full-width, ligatured or otherwise presentation-variant spelling reach
///    the same bytes as the plain one. Without it an entry signed off as
///    `Ann Fritz` would not match `Ann Fritz` typed from a document that
///    carried a ligature, and Legal would be asked to sign off twice for one
///    name.
/// 2. **Trim, then collapse internal whitespace runs to a single `U+0020`.**
///    An operator pasting a name out of a document brings its whitespace with
///    it, including the non-breaking spaces NFKC has just turned into plain
///    ones. Two spaces between a first and last name is not a different name.
/// 3. **Lowercase.** Case is not part of a name's identity for this purpose,
///    and `Ann Fritz` and `ANN FRITZ` must not need two sign-offs.
///
/// Lowercasing runs **last** because `str::to_lowercase` is defined over the
/// decomposed form NFKC produces; folding first and decomposing after can
/// leave a different string for the same input.
///
/// **What it deliberately does not do**: strip punctuation, drop diacritics,
/// or reorder words. Each of those would widen the match beyond what the
/// sign-off covered — `O'Neill` and `ONeill` are different strings and Legal
/// signed off on one of them — and the narrowest rule is the one that cannot
/// admit more than Legal admitted.
#[must_use]
pub fn normalize_allowlist_value(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let folded: String = value.nfkc().collect();
    folded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The detector's verdict-bearing policy, over one tenant's active
/// allow-list.
///
/// The allow-list is **handed in, already read**: [`crate::domain::taxonomy::PiiDetector::inspect`] is
/// synchronous by design — *"a door calls it inside its own transaction and an
/// `async` seam would let a detector make a network call there"* — so the
/// door loads the tenant's active entries and builds this. An empty set is a
/// legitimate state and not a missing read: a tenant Legal has signed nothing
/// off for has no entries, and every person-shaped candidate is then
/// undecidable, which is the correct answer rather than a failure.
#[derive(Debug, Clone, Default)]
pub struct RegistryPiiDetector {
    /// The tenant's active entries, already normalized by
    /// [`normalize_allowlist_value`].
    allowed: std::collections::BTreeSet<String>,
}

/// What the detector found in a field, before the allow-list is consulted.
///
/// Kept separate from [`crate::domain::taxonomy::PiiVerdict`] because the two
/// answer different questions: this one is *"what shape is in the text"* and
/// the verdict is *"what should the door do"*. Only the person-shaped arm is
/// the allow-list's business, and folding them would let a future arm reach
/// the list by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finding {
    /// An `@`-bearing token with a dotted right-hand side.
    EmailAddress,
    /// A run of digits, separators and an optional leading `+`, carrying at
    /// least [`PHONE_MIN_DIGITS`] digits.
    PhoneNumber,
    /// Two or more adjacent capitalized words — the person-name shape, and
    /// the only finding the allow-list can answer.
    PersonShapedName,
}

/// How many digits a run needs before it is read as a phone number.
///
/// Nine rather than seven: a seven-digit floor blocks ordinary catalog
/// identifiers (`SKU-1234567`), and a field refused for carrying its own SKU
/// code is a false positive an operator cannot clear — the allow-list holds
/// names, not numbers, so there is no lane out of it. Nine is the shortest
/// national significant number in the E.164 plans this registry ships to.
const PHONE_MIN_DIGITS: usize = 9;

impl RegistryPiiDetector {
    /// Build the detector over one tenant's active allow-list values.
    ///
    /// Values are normalized on the way in, so a caller handing raw column
    /// text and a caller handing already-normalized text agree.
    pub fn new(active_values: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed: active_values
                .into_iter()
                .map(|value| normalize_allowlist_value(&value))
                .collect(),
        }
    }

    /// Whether the normalized candidate is covered by an active entry.
    fn is_allowed(&self, candidate: &str) -> bool {
        self.allowed.contains(candidate)
    }
}

/// Does this token look like an email address?
///
/// Deliberately crude — an `@` with a non-empty local part and a dotted
/// domain. A stricter grammar would reject malformed addresses, and a
/// malformed address someone typed is still that person's address.
fn looks_like_email(token: &str) -> bool {
    let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric());
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && domain.split('.').all(|part| !part.is_empty())
        && !domain.ends_with('.')
}

/// The separators a written telephone number carries between its digits.
const PHONE_SEPARATORS: [char; 6] = ['-', '(', ')', ' ', '.', '+'];

/// The most digits an E.164 number can hold. A longer run is an identifier,
/// an account number or a hash fragment, and calling it a phone number would
/// refuse writes no reasonable operator can rephrase.
const PHONE_MAX_DIGITS: usize = 15;

/// Does `text` carry a written telephone number?
///
/// **Whole-text and not per token**, which is the correction a probe made:
/// `+44 20 7946 0958` is four whitespace-separated tokens of two to four
/// digits each, so a per-token rule saw no number at all and the arm that
/// existed to block phone numbers blocked none.
///
/// A candidate is a maximal run of digits and [`PHONE_SEPARATORS`]. It is a
/// number when it holds between [`PHONE_MIN_DIGITS`] and [`PHONE_MAX_DIGITS`]
/// digits **and every digit group in it is at least two digits long**. That
/// last clause is what keeps `tiers 1 2 3 4 5 6 7 8 9` — nine digits in one
/// run of single-digit groups — out: an enumeration is not a phone number,
/// and a rule without it refuses ordinary prose that happens to count.
fn text_carries_phone(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_digit() && !PHONE_SEPARATORS.contains(&c))
        .any(|run| {
            let groups: Vec<usize> = run
                .split(|c: char| !c.is_ascii_digit())
                .filter(|group| !group.is_empty())
                .map(str::len)
                .collect();
            let digits: usize = groups.iter().sum();
            (PHONE_MIN_DIGITS..=PHONE_MAX_DIGITS).contains(&digits)
                && groups.iter().all(|len| *len >= 2)
        })
}

/// Is this word capitalized in the way a name part is?
fn is_capitalized_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_uppercase() && chars.clone().count() > 0 && chars.all(char::is_lowercase)
}

/// The longest run of adjacent capitalized words in `text`, as its own
/// string, or `None` when there is no run of two or more.
///
/// The run rather than the pair, because `Maria Del Carmen Ruiz` is one name
/// and four pairwise candidates would ask Legal to sign off on three
/// fragments of it.
fn person_shaped_run(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut best: Option<(usize, usize)> = None;
    let mut start: Option<usize> = None;
    for (index, word) in words.iter().enumerate() {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-');
        if is_capitalized_word(trimmed) {
            start.get_or_insert(index);
        } else if let Some(from) = start.take()
            && index - from >= 2
            && best.is_none_or(|(b, e)| e - b < index - from)
        {
            best = Some((from, index));
        }
    }
    if let Some(from) = start
        && words.len() - from >= 2
        && best.is_none_or(|(b, e)| e - b < words.len() - from)
    {
        best = Some((from, words.len()));
    }
    best.map(|(from, to)| {
        words[from..to]
            .iter()
            .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-'))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// What the text carries, and the candidate the allow-list would be asked
/// about.
fn inspect_text(text: &str) -> Option<(Finding, String)> {
    for token in text.split_whitespace() {
        if looks_like_email(token) {
            return Some((Finding::EmailAddress, String::new()));
        }
    }
    if text_carries_phone(text) {
        return Some((Finding::PhoneNumber, String::new()));
    }
    person_shaped_run(text).map(|run| (Finding::PersonShapedName, run))
}

impl crate::domain::taxonomy::PiiDetector for RegistryPiiDetector {
    /// # The four arms, and which of them the allow-list can reach
    ///
    /// - **block** — an email address or a phone number. These are personal
    ///   data by shape and no allow-list entry covers them: `inst-pp-allowlist`
    ///   admits *names*, and a signed-off phone number would be Legal
    ///   consenting to store the very thing the map exists to keep out of
    ///   immutable records.
    /// - **allow-by-list** — a person-shaped run whose normalized form is an
    ///   active entry. This is C2's *"legitimate person-named products"*.
    /// - **uncertainty, which the hook blocks** — a person-shaped run that is
    ///   **not** listed. See below.
    /// - **allow** — none of the above.
    ///
    /// # What makes this detector uncertain, stated
    ///
    /// **It cannot tell a person's name from a product named after one**, and
    /// that is not a gap to be closed by a better heuristic — it is the exact
    /// question the allow-list was created to answer, by a human, on paper.
    /// So an unlisted person-shaped run is [`crate::domain::taxonomy::PiiVerdict::Uncertain`] and never
    /// [`crate::domain::taxonomy::PiiVerdict::Blocked`]: `Blocked` asserts a finding — *this is
    /// personal data* — that nothing here established, and it would put that
    /// false assertion into the refusal an operator reads and an audit row
    /// keeps. `Uncertain` says the true thing, and the write is still refused,
    /// because [`crate::domain::taxonomy::content_pii_block`] holds the
    /// fail-closed rule (C2). The operator's lane out is the allow-list, and a
    /// refusal that told them *"this is personal data"* would not point at it.
    ///
    /// Email and phone are `Blocked` for the mirror reason: there the shape
    /// **is** the finding, and calling it uncertain would understate what the
    /// detector actually knows.
    ///
    /// # No reason ever carries the matched text
    ///
    /// The `DoD`'s own clause — a block *"naming the field and never the
    /// detected value"* — and the reason it gives: a refusal that echoed the
    /// match would write the personal data into the refusal's own audit row,
    /// which is a record erasure cannot reach. Every string below names a
    /// **shape**; the candidate is used to consult the list and is then
    /// dropped.
    fn inspect(&self, _subject: &str, text: &str) -> crate::domain::taxonomy::PiiVerdict {
        use crate::domain::taxonomy::PiiVerdict;
        match inspect_text(text) {
            None => PiiVerdict::Clean,
            Some((Finding::EmailAddress, _)) => PiiVerdict::Blocked(
                "an email address appears in this field, and an address is personal data no \
                 allow-list entry covers"
                    .to_owned(),
            ),
            Some((Finding::PhoneNumber, _)) => PiiVerdict::Blocked(
                "a telephone number appears in this field, and a number is personal data no \
                 allow-list entry covers"
                    .to_owned(),
            ),
            Some((Finding::PersonShapedName, candidate)) => {
                if self.is_allowed(&normalize_allowlist_value(&candidate)) {
                    PiiVerdict::Clean
                } else {
                    PiiVerdict::Uncertain(
                        "this field carries adjacent capitalized words, which is the shape of a \
                         personal name and equally the shape of a product named after one; no \
                         active allow-list entry covers it, and only a Legal sign-off can tell \
                         the two apart"
                            .to_owned(),
                    )
                }
            }
        }
    }
}
