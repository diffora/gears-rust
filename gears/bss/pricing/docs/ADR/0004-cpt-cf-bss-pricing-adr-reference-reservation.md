# ADR-0004: Reserve References before Pricing Writes

**ID**: `cpt-cf-bss-pricing-adr-reference-reservation`

- [ ] `p1` - **ADR implementation status**

**Status:** accepted 2026-09-25 · **Deciders:** product owner · **Source:** spec §2 decision 17; §4 reference registry; §13.

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
- [More Information](#more-information)

<!-- /toc -->

## Context and Problem Statement

A remote reference count races a SKU retirement or type-change fence: a caller can observe zero just before Pricing commits a new reference. The owner rejects even a bounded window containing a retired SKU with a live reference.

## Decision Drivers

`cpt-cf-bss-pricing-fr-reference-protocol`, `cpt-cf-bss-pricing-nfr-tenant-isolation`, `cpt-cf-bss-pricing-nfr-two-backends`. The approved PriceBook model is the content authority. The decision must hold under tenant isolation,
concurrent writes and either supported database, and must remain explainable to operators and reviewers.

## Considered Options

1. Remote count with reconciliation.
2. Cross-gear distributed transaction.
3. Reference reservation with durable confirmation.

## Decision Outcome

Chosen: **Reference reservation with durable confirmation**. Products owns live reserved/confirmed/released receipts. Pricing reserves a logical reference, re-reads the SKU, commits its object with the receipt and durable confirmation work, then confirms. Reserve and fence are reciprocal guarded writes in Products. An unconfirmed receipt counts until released. A confirm timeout never releases; definite rollback first records durable cancellation, deletion first commits removal, then release is retried. Released receipts never reactivate.

### Consequences

Creating a price requires Products availability: failure before write is REGISTRY_UNAVAILABLE. Confirmation outage leaves confirmation_pending, protecting the SKU. REFERENCE_RELEASED becomes reference_lost and PriceReferenceLost, requiring remediation. Dead callers may block retirement until recovery or audited operator force-release; this availability cost is accepted. Phase 3 uses the same barrier for plan_item and sold_as.

### Confirmation

Use real Products/Pricing integration tests on SQLite and Postgres to race reserve with fence: both cannot succeed. Inject failure before write, during commit and after commit. Verify pending confirmation survives restart, timeout never releases, deletion releases only after removal, and released receipt confirmation surfaces reference loss. These are implementation acceptance conditions; Part 2a replaces documents only.

## Pros and Cons of the Options

Remote counts cannot prevent the race and were rejected. A distributed transaction couples both databases and infrastructure. Reservations close the race using local guarded writes but need durable reconciliation of confirmations and releases, and expose stalled receipts to operators.

## More Information

[PRD](../PRD.md), [DESIGN](../DESIGN.md), [DECISIONS](../DECISIONS.md).
Source: `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout, §2 decision 17; §4 reference registry; §13.
The superseded set remains on `bss/products-backup` and in history.
