//! Fixed user command constraints. No registry reads or resource-schema projection.

use std::net::{Ipv4Addr, Ipv6Addr};

use email_address::{EmailAddress, Options};

use crate::{IdpNewUser, IdpUserAttribute, IdpUserPatch};

/// Minimum username or supplied display/given/family name length in Unicode scalar values.
pub const USER_TEXT_MIN_CHARS: usize = 1;
/// Maximum username or supplied display/given/family name length in Unicode scalar values.
pub const USER_TEXT_MAX_CHARS: usize = 255;
// Existing AM command cap, not a new constraint on the published User v1 resource.
const USER_INPUT_EMAIL_MAX_CHARS: usize = 255;

/// Invalid fixed input fields; diagnostics never include supplied profile/credential values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UserInputValidationError {
    #[error("{} must contain {min}..={max} characters", field.as_field_token())]
    Length {
        field: IdpUserAttribute,
        min: usize,
        max: usize,
    },
    #[error("email must be a plain ASCII email address with a valid hostname or IP literal")]
    InvalidEmail,
    #[error("patch is empty; at least one field is required")]
    EmptyPatch,
}

impl IdpNewUser {
    /// Validate fixed create fields without a resource identity or registry dependency.
    ///
    /// Username and set display/given/family names contain 1–255 Unicode scalars.
    /// Email is a plain ASCII mailbox (no display text), at most 255 characters,
    /// with a DNS-style hostname or valid IPv4/IPv6 literal; a TLD is not required.
    /// No values are trimmed or changed. The AM service owns username whitespace
    /// normalization; the provider owns credential policy. Extensible data, if added,
    /// requires its own typed schema contract rather than these fixed-field rules.
    ///
    /// # Errors
    /// [`UserInputValidationError`] for an invalid fixed profile field.
    pub fn validate(&self) -> Result<(), UserInputValidationError> {
        validate_fields(
            Some(&self.username),
            self.email.as_deref(),
            self.display_name.as_deref(),
            self.first_name.as_deref(),
            self.last_name.as_deref(),
        )
    }
}

impl IdpUserPatch {
    /// Validate only supplied values; omitted fields and explicit clears are skipped.
    ///
    /// Uses the same bounds/email policy as [`IdpNewUser::validate`]. A credential-only
    /// patch is permitted, with password policy delegated to the provider. Inputs are
    /// never normalized or mutated.
    ///
    /// # Errors
    /// [`UserInputValidationError`] for an empty patch or invalid supplied profile field.
    pub fn validate(&self) -> Result<(), UserInputValidationError> {
        if self.is_empty() {
            return Err(UserInputValidationError::EmptyPatch);
        }
        validate_fields(
            self.username.as_deref(),
            self.email.as_ref().and_then(|v| v.as_deref()),
            self.display_name.as_ref().and_then(|v| v.as_deref()),
            self.first_name.as_ref().and_then(|v| v.as_deref()),
            self.last_name.as_ref().and_then(|v| v.as_deref()),
        )
    }
}

fn validate_fields(
    username: Option<&str>,
    email: Option<&str>,
    display_name: Option<&str>,
    first_name: Option<&str>,
    last_name: Option<&str>,
) -> Result<(), UserInputValidationError> {
    for (field, value) in [
        (IdpUserAttribute::Username, username),
        (IdpUserAttribute::DisplayName, display_name),
        (IdpUserAttribute::FirstName, first_name),
        (IdpUserAttribute::LastName, last_name),
    ] {
        if let Some(value) = value {
            check_length(field, value, USER_TEXT_MIN_CHARS, USER_TEXT_MAX_CHARS)?;
        }
    }
    if let Some(email) = email {
        check_length(
            IdpUserAttribute::Email,
            email,
            0,
            USER_INPUT_EMAIL_MAX_CHARS,
        )?;
        if !valid_email(email) {
            return Err(UserInputValidationError::InvalidEmail);
        }
    }
    Ok(())
}

fn check_length(
    field: IdpUserAttribute,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), UserInputValidationError> {
    if !(min..=max).contains(&value.chars().count()) {
        return Err(UserInputValidationError::Length { field, min, max });
    }
    Ok(())
}

fn valid_email(value: &str) -> bool {
    let Ok(address) =
        EmailAddress::parse_with_options(value, Options::default().without_display_text())
    else {
        return false;
    };
    if !address.local_part().is_ascii() {
        return false;
    }
    let domain = address.domain();
    if let Some(literal) = domain.strip_prefix('[').and_then(|d| d.strip_suffix(']')) {
        match literal.split_once(':') {
            Some((tag, ip)) if tag.eq_ignore_ascii_case("ipv6") => ip.parse::<Ipv6Addr>().is_ok(),
            _ => literal.parse::<Ipv4Addr>().is_ok(),
        }
    } else {
        // The parser checks label lengths and leading/trailing characters; require
        // DNS label characters too, rather than its broader RFC mailbox atom set.
        domain.split('.').all(|label| {
            label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    }
}
