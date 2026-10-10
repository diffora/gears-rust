//! Functional-currency scale failures keep their request/server classification.

use super::functional_scale_error;
use crate::domain::error::DomainError;
use crate::domain::model::RepoError;
use crate::domain::money::ScaleError;

#[test]
fn an_unregistered_functional_currency_is_a_request_error_not_a_500() {
    assert!(matches!(
        functional_scale_error(ScaleError::UnknownCurrencyScale("XTS".into())),
        DomainError::InvalidRequest(d) if d.contains("XTS")
    ));
    assert!(matches!(
        functional_scale_error(ScaleError::Repo(RepoError::Db("down".into()))),
        DomainError::Internal(_)
    ));
    assert!(matches!(
        functional_scale_error(ScaleError::CorruptStoredScale {
            currency: "XTS".into(),
            currency_scale: 40,
        }),
        DomainError::Internal(_)
    ));
}
