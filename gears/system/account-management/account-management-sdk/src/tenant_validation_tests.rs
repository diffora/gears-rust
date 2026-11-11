//! Fixed tenant command bounds, independent of the registry and normalization.

use crate::{CreateTenantRequest, UpdateTenantRequest, validate_tenant_name};
use toolkit_gts::gts_id;
use uuid::Uuid;

#[test]
fn name_bounds_count_unicode_scalars_without_trimming() {
    for (name, valid) in [
        (String::new(), false),
        ("\u{44f}".repeat(255), true),
        ("\u{44f}".repeat(256), false),
        (format!(" {} ", "x".repeat(254)), false),
        ("  name  ".to_owned(), true),
    ] {
        assert_eq!(validate_tenant_name(&name).is_ok(), valid);
    }
}

#[test]
fn request_validation_delegates_name_bounds_without_mutating_inputs() {
    for (name, valid) in [(" name ".to_owned(), true), ("x".repeat(256), false)] {
        let request = CreateTenantRequest::new(
            Uuid::nil(),
            Uuid::nil(),
            &name,
            gts::GtsTypeId::new(gts_id!("cf.core.am.tenant_type.v1~cf.core.am.customer.v1~")),
        );
        let patch = UpdateTenantRequest::new().with_name(&name);
        assert_eq!(request.validate().is_ok(), valid);
        assert_eq!(patch.validate().is_ok(), valid);
        assert_eq!(request.name, name);
        assert_eq!(patch.name.as_deref(), Some(name.as_str()));
    }
}

#[test]
fn omitted_name_is_skipped_and_errors_do_not_include_the_name() {
    UpdateTenantRequest::new()
        .validate()
        .expect("empty-patch policy belongs to the service");
    let name = "secret".repeat(256);
    let error = validate_tenant_name(&name).expect_err("oversized name");
    assert!(!error.to_string().contains("secret"));
}
