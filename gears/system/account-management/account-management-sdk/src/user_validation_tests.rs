//! DTO contracts: no registry, complete User document or provider credential checks.

use crate::{IdpNewUser, IdpUserAttribute, IdpUserPatch, UserInputValidationError};

#[test]
fn create_validation_needs_no_resource_identity_and_does_not_normalize_fields() {
    let mut user = IdpNewUser::new("  alice  ");
    user.display_name = Some(" Alice ".to_owned());
    user.validate()
        .expect("structurally valid DTO without a user id");
    assert_eq!(user.username, "  alice  ");
    assert_eq!(user.display_name.as_deref(), Some(" Alice "));
}

#[test]
fn username_bounds_count_unicode_scalars() {
    for (name, valid) in [
        (String::new(), false),
        ("\u{44f}".repeat(255), true),
        ("\u{44f}".repeat(256), false),
    ] {
        assert_eq!(IdpNewUser::new(name).validate().is_ok(), valid);
    }
}

#[test]
fn optional_profile_fields_have_the_same_bounds_on_create_and_patch() {
    for field in [
        IdpUserAttribute::DisplayName,
        IdpUserAttribute::FirstName,
        IdpUserAttribute::LastName,
    ] {
        for (value, valid) in [
            (String::new(), false),
            ("\u{44f}".repeat(255), true),
            ("\u{44f}".repeat(256), false),
        ] {
            let mut create = IdpNewUser::new("alice");
            let mut patch = IdpUserPatch::default();
            match field {
                IdpUserAttribute::DisplayName => {
                    create.display_name = Some(value.clone());
                    patch.display_name = Some(Some(value));
                }
                IdpUserAttribute::FirstName => {
                    create.first_name = Some(value.clone());
                    patch.first_name = Some(Some(value));
                }
                IdpUserAttribute::LastName => {
                    create.last_name = Some(value.clone());
                    patch.last_name = Some(Some(value));
                }
                _ => unreachable!("profile fields"),
            }
            for result in [create.validate(), patch.validate()] {
                assert_eq!(result.is_ok(), valid, "{field:?}: {result:?}");
                if let Err(UserInputValidationError::Length { field: actual, .. }) = result {
                    assert_eq!(actual, field);
                }
            }
        }
    }
}

#[test]
fn patch_distinguishes_omitted_cleared_and_set_fields() {
    IdpUserPatch::new()
        .with_password("", false)
        .validate()
        .expect("provider-specific credential policy is not a DTO profile constraint");
    let mut patch = IdpUserPatch::default();
    assert!(matches!(
        patch.validate(),
        Err(UserInputValidationError::EmptyPatch)
    ));
    patch.email = Some(None);
    patch.display_name = Some(None);
    patch.first_name = Some(None);
    patch.last_name = Some(None);
    patch
        .validate()
        .expect("clearing optional fields remains valid");
    patch.username = Some(String::new());
    assert!(matches!(
        patch.validate(),
        Err(UserInputValidationError::Length {
            field: IdpUserAttribute::Username,
            ..
        })
    ));
}

#[test]
fn email_policy_accepts_plain_ascii_mailboxes_and_valid_ip_literals() {
    for email in [
        "alice@example.com",
        "alice+tag@example.com",
        "alice@localhost",
        "\"alice smith\"@example.com",
        "alice@[127.0.0.1]",
        "alice@[IPv6:::1]",
    ] {
        let user = IdpNewUser::new("alice").with_email(email);
        user.validate()
            .unwrap_or_else(|error| panic!("{email}: {error}"));
    }
}

#[test]
fn email_policy_rejects_display_text_unicode_local_bad_domains_and_literals() {
    for email in [
        "",
        "not-an-email",
        "Alice <alice@example.com>",
        "<alice@example.com>",
        "t\u{e9}st@example.com",
        // An IDN host: the previous property validator accepted it.
        "user@m\u{fc}nchen.de",
        "a@exa_mple.com",
        "a@[not-an-ip]",
        "a@[127.0.0.256]",
        "a@-example.com",
        " alice@example.com",
        "alice@example.com ",
        "\"\"@example.com",
    ] {
        let user = IdpNewUser::new("alice").with_email(email);
        assert!(
            matches!(user.validate(), Err(UserInputValidationError::InvalidEmail)),
            "{email}"
        );
        let patch = IdpUserPatch::default().with_email(Some(email.to_owned()));
        assert!(
            matches!(
                patch.validate(),
                Err(UserInputValidationError::InvalidEmail)
            ),
            "{email}"
        );
    }
}

#[test]
fn email_cap_precedes_parsing_and_errors_never_contain_supplied_values() {
    let boundary = format!(
        "{}@{}.{}.{}",
        "a".repeat(64),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(62)
    );
    assert_eq!(boundary.len(), 255);
    IdpNewUser::new("alice")
        .with_email(&boundary)
        .validate()
        .expect("input cap boundary");
    let over = format!("{boundary}d");
    assert!(matches!(
        IdpNewUser::new("alice").with_email(over).validate(),
        Err(UserInputValidationError::Length {
            field: IdpUserAttribute::Email,
            ..
        })
    ));
    let email = format!("{}@example.com", "secret".repeat(50));
    let error = IdpNewUser::new("alice")
        .with_email(&email)
        .validate()
        .expect_err("oversized email");
    assert!(matches!(
        error,
        UserInputValidationError::Length {
            field: IdpUserAttribute::Email,
            ..
        }
    ));
    assert!(!error.to_string().contains("secret"));
}
