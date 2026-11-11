//! Admission-boundary checks shared by the fake and the real registry clients.

use serde_json::json;
use toolkit_gts::gts_id;

use super::{
    CandidateStatus, Checker, EntityLookup, RegisterEntitiesRequest, RegistrationOutcome, create,
    derived, every_field, found, gts_key, light, publisher, reason, revise, root, uri,
};

impl Checker<'_> {
    pub(super) async fn schema_admission(&self) {
        self.invalid_schema_candidates_are_refused().await;
        self.child_before_parent_is_admitted().await;
        self.invalid_parent_blocks_only_its_dependents().await;
        self.a_cycle_refuses_its_members_and_blocks_their_dependents()
            .await;
        self.refused_revision_preserves_the_previous_snapshot()
            .await;
        self.invalid_dry_run_has_no_writes().await;
    }

    async fn invalid_schema_candidates_are_refused(&self) {
        const PARENT: &str = gts_id!("cf.conformance.admission.base.v1~");
        const BAD_SCHEMA: &str = gts_id!("cf.conformance.admission.broken.v1~");
        const BAD_TRAITS: &str = gts_id!("cf.conformance.admission.traits.v1~");
        const BAD_DERIVATION: &str =
            gts_id!("cf.conformance.admission.base.v1~cf.conformance.admission.invalid.v1~");
        let mut final_parent = root(PARENT, None);
        final_parent["x-gts-final"] = json!(true);
        self.register(vec![create(PARENT, final_parent)]).await;
        let mut invalid_schema = root(BAD_SCHEMA, None);
        invalid_schema["type"] = json!(42);
        let mut invalid_traits = root(BAD_TRAITS, Some(json!({"tier": {"type": "string"}})));
        invalid_traits["x-gts-traits"] = json!({"tier": 42});
        let invalid_derivation = derived(BAD_DERIVATION, PARENT, None);
        for (id, content) in [
            (BAD_SCHEMA, invalid_schema),
            (BAD_TRAITS, invalid_traits),
            (BAD_DERIVATION, invalid_derivation),
        ] {
            let outcome = self.register_one(create(id, content), false).await;
            assert_eq!(
                outcome.status(),
                CandidateStatus::Failed,
                "{id}: {outcome:?}"
            );
            assert_eq!(
                reason(outcome.error()),
                "invalid_schema",
                "{id}: {outcome:?}"
            );
            assert!(matches!(
                self.get(gts_key(id), light()).await,
                EntityLookup::NotFound
            ));
        }
    }

    async fn child_before_parent_is_admitted(&self) {
        const PARENT: &str = gts_id!("cf.conformance.unordered.base.v1~");
        const CHILD: &str =
            gts_id!("cf.conformance.unordered.base.v1~cf.conformance.unordered.child.v1~");
        self.register(vec![
            create(CHILD, derived(CHILD, PARENT, None)),
            create(PARENT, root(PARENT, None)),
        ])
        .await;
        let (child, _) = found(self.get(gts_key(CHILD), every_field()).await);
        assert!(child.resolved_schema.is_some());
    }

    async fn invalid_parent_blocks_only_its_dependents(&self) {
        const PARENT: &str = gts_id!("cf.conformance.mixed.base.v1~");
        const CHILD: &str = gts_id!("cf.conformance.mixed.base.v1~cf.conformance.mixed.child.v1~");
        const INDEPENDENT: &str = gts_id!("cf.conformance.mixed.independent.v1~");
        let mut invalid = root(PARENT, None);
        invalid["type"] = json!(42);
        let outcomes = self
            .try_register(vec![
                create(CHILD, derived(CHILD, PARENT, None)),
                create(PARENT, invalid),
                create(INDEPENDENT, root(INDEPENDENT, None)),
            ])
            .await;
        assert_eq!(
            outcomes,
            vec![
                (CandidateStatus::Failed, "blocked_by_dependency".to_owned()),
                (CandidateStatus::Failed, "invalid_schema".to_owned()),
                (CandidateStatus::Succeeded, String::new()),
            ]
        );
        for id in [PARENT, CHILD] {
            assert!(matches!(
                self.get(gts_key(id), light()).await,
                EntityLookup::NotFound
            ));
        }
        let (independent, _) = found(self.get(gts_key(INDEPENDENT), every_field()).await);
        assert!(independent.resolved_schema.is_some());
    }

    async fn a_cycle_refuses_its_members_and_blocks_their_dependents(&self) {
        const A: &str = gts_id!("cf.conformance.cycle.a.v1~");
        const B: &str = gts_id!("cf.conformance.cycle.b.v1~");
        const DEPENDENT: &str = gts_id!("cf.conformance.cycle.a.v1~cf.conformance.cycle.child.v1~");
        let mut a = root(A, None);
        a["properties"]["peer"] = json!({ "$ref": uri(B) });
        let mut b = root(B, None);
        b["properties"]["peer"] = json!({ "$ref": uri(A) });
        let outcomes = self
            .try_register(vec![
                create(DEPENDENT, derived(DEPENDENT, A, None)),
                create(A, a),
                create(B, b),
            ])
            .await;
        assert_eq!(
            outcomes,
            vec![
                (CandidateStatus::Failed, "blocked_by_dependency".to_owned()),
                (CandidateStatus::Failed, "invalid_schema".to_owned()),
                (CandidateStatus::Failed, "invalid_schema".to_owned()),
            ]
        );
        for id in [A, B, DEPENDENT] {
            assert!(matches!(
                self.get(gts_key(id), light()).await,
                EntityLookup::NotFound
            ));
        }
    }

    async fn refused_revision_preserves_the_previous_snapshot(&self) {
        const PARENT: &str = gts_id!("cf.conformance.revision.base.v1~");
        const CHILD: &str =
            gts_id!("cf.conformance.revision.base.v1~cf.conformance.revision.child.v1~");
        let original = root(PARENT, None);
        self.register(vec![create(PARENT, original.clone())]).await;
        let before = found(self.get(gts_key(PARENT), every_field()).await);
        let mut invalid = original;
        invalid["type"] = json!(42);
        let outcomes = self
            .try_register(vec![
                create(CHILD, derived(CHILD, PARENT, None)),
                revise(PARENT, invalid, 1),
            ])
            .await;
        assert_eq!(
            outcomes,
            vec![
                (CandidateStatus::Failed, "blocked_by_dependency".to_owned()),
                (CandidateStatus::Failed, "invalid_schema".to_owned()),
            ]
        );
        let after = found(self.get(gts_key(PARENT), every_field()).await);
        assert_eq!(
            after, before,
            "a rejected revision changes no artifact or validator"
        );
        assert!(matches!(
            self.get(gts_key(CHILD), light()).await,
            EntityLookup::NotFound
        ));
    }

    async fn invalid_dry_run_has_no_writes(&self) {
        const BAD: &str = gts_id!("cf.conformance.preview.broken.v1~");
        const GOOD: &str = gts_id!("cf.conformance.preview.good.v1~");
        let mut invalid = root(BAD, None);
        invalid["type"] = json!(42);
        let operation = self
            .registration(RegisterEntitiesRequest {
                items: vec![create(BAD, invalid), create(GOOD, root(GOOD, None))],
                dry_run: true,
                publisher: publisher(),
            })
            .await;
        assert_eq!(reason(operation.items[0].outcome.error()), "invalid_schema");
        assert!(matches!(
            operation.items[1].outcome,
            RegistrationOutcome::WouldSucceed
        ));
        for id in [BAD, GOOD] {
            assert!(matches!(
                self.get(gts_key(id), light()).await,
                EntityLookup::NotFound
            ));
        }
    }
}
