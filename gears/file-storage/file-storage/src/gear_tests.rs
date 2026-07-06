use toolkit::DatabaseCapability;

use super::*;

#[test]
fn gear_provides_p1_and_p2_migrations() {
    // The DatabaseCapability wiring must hand the runtime all current migrations:
    //   1. P1 initial (control-plane metadata tables)
    //   2. P2 initial (policy store + retention rules + multipart + idempotency
    //      keys + audit outbox + file events outbox, in one step)
    //   3. P2 multipart plan columns (declared_size + part_size on multipart_uploads)
    //   4. P2 remediation 0.10: idempotency_keys.subject_id (binds a replay to
    //      the authenticated caller, not just the request-body owner)
    //   5. P2 remediation 2.1: idempotency_keys.request_hash (binds a replay to
    //      the request body that created it, not just the caller)
    //   6. P2 remediation 2.4: policies partial unique indexes (at most one row
    //      per (tenant_id, scope, scope_owner_id), closing the upsert race)
    // (init()/register_rest() need a live GearCtx — those seams are covered by
    // the E2E suite, not here.)
    let gear = FileStorageGear::default();
    assert_eq!(
        gear.migrations().len(),
        6,
        "gear must provide the P1, P2 initial, P2 multipart plan columns, P2 \
         remediation 0.10 idempotency subject_id, P2 remediation 2.1 \
         idempotency request_hash, and P2 remediation 2.4 policies unique \
         scope migrations"
    );
}

#[test]
fn gear_default_config_excludes_in_memory_backend() {
    // P2 remediation 0.5: the non-durable `memory` backend must not be part
    // of the registry unless a deployment explicitly opts in — otherwise
    // every deployment silently exposes a volatile backend.
    let cfg = FileStorageConfig::default();
    assert!(!cfg.enable_in_memory_backend);

    let registry =
        build_backend_registry(&cfg).expect("default config must build a valid registry");
    assert!(
        registry.list().iter().all(|(id, _)| id != "memory"),
        "memory backend must be absent by default"
    );
}

#[test]
fn gear_dev_flag_enables_in_memory_backend() {
    // Opting in via `enable_in_memory_backend: true` registers the `memory`
    // backend alongside the always-present `local-fs` default.
    let cfg = FileStorageConfig {
        enable_in_memory_backend: true,
        ..FileStorageConfig::default()
    };

    let registry = build_backend_registry(&cfg).expect("dev config must build a valid registry");
    assert!(
        registry.list().iter().any(|(id, _)| id == "memory"),
        "memory backend must be present when enable_in_memory_backend is set"
    );
}
