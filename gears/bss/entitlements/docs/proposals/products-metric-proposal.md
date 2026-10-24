---
description: "Metric, unit and subject semantics and the versioned mapping to the Quota Enforcement metric registry, requested from Products by Entitlements."
---

# Products metric proposal from Entitlements

**Status:** proposed owner handoff from Entitlements, 2026-10-23. This is a request to the Products owner, not an accepted amendment to the Products design, a delivered SDK or an external-owner approval. Examples are illustrative, not catalog defaults. The [proposal index](README.md) provides scope and source precedence; the consuming feature is [quota projection](../features/03-quota-projection.md).

<!-- toc -->

- [1. Baseline](#1-baseline)
- [2. Required changes](#2-required-changes)
- [3. Example](#3-example)
- [4. Acceptance](#4-acceptance)
- [5. Open points](#5-open-points)

<!-- /toc -->

## 1. Baseline

| Inspected source | What it establishes |
| --- | --- |
| [Products PRD](../../../products/docs/PRD.md) | SKU registry and dated versions; existing descriptors are billing descriptors (`gl_code`, `tax_category`, `invoice_line_template`). Usage SKUs declare `usage_type_ref` and `unit`; bundles have no meter. This does not establish an existing general quota-definition catalog. |
| [Entitlements DESIGN §3.1](../DESIGN.md#31-domain-model) | `PlanQuotaRule` refers to a Products SKU/version and a metric semantics reference with digest; Entitlements owns the rule meaning (ENT-D-03) and needs Products to own the metric meaning. |

Products' place in the ownership after the split: stable SKU and usage metric/unit references, the proposed versioned quota semantics, the valid subject and resource kinds and the available technical capabilities. A usage meter is evidence of unit semantics, not automatically a point-of-use enforcement meter. Non-usage allocation metrics need an explicit descriptor or capability contract; usage-only meter fields are not added to bundles.

Existing billing descriptors should not be described as if this quota schema already existed. A Products `usage_type_ref`, including `products.derived/<code>@<n>`, is not automatically a valid QE GTS `MetricId`. The metric owner must register the enforcement metric as the required GTS instance and supply a versioned Products-to-QE mapping with unit, measurement and supported subject semantics. A Rating-derived quantity is not assumed observable synchronously at point of use. Technical safety ceilings independent of purchases remain operator or provider policy and must be reconciled explicitly with commercial caps.

## 2. Required changes

| Request | Minimal change / release evidence |
| --- | --- |
| PD-P01 Metric, unit and subject semantics | Publish, per SKU version, a versioned quota semantics descriptor with a digest: metric code, canonical unit and its exact conversions to the enforcement integer unit, the quota kinds the metric supports (`Allocation`, `Consumption`), the permitted periods for consumption, the supported `subject_kind` values and the resource kinds a subject may bind to, and whether point-of-use enforcement exists for the metric. The descriptor is read by Pricing at publication and frozen by reference and digest in the acceptance receipt; a later edit never alters an accepted cap. Mark derived and Rating-side quantities as not enforceable at point of use unless an enforcement meter exists. |
| PD-P02 Registered enforcement metric and GTS mapping | Register each enforceable metric as the required QE GTS `MetricId` instance and publish a versioned mapping from the Products metric reference to that `MetricId` and from each supported subject kind to the QE `projection_type`, with the mapping version and digest. A string name alone is insufficient. The mapping is historical: an old version stays resolvable for receipts that reference it. Report registration loss or an enforcement-mode change (a metric becoming `Direct` or inert) as a versioned change, not as a silent edit. |

Products does not store rules, coefficients or caps; those belong to Pricing under the [pricing rule proposal](pricing-rule-proposal.md). Products does not count usage or make point-of-use decisions; those stay with Quota Enforcement.

## 3. Example

Products publishes, for SKU `object-storage@4`, the quota semantics descriptor `storage.allocated_bytes/S2`: unit `byte`, conversions `GiB = 1073741824 byte` as accepted data, kind `Allocation`, no period, subject kinds `project` and `tenant`, point-of-use enforcement present. The mapping `S2 -> M3` resolves `storage.allocated_bytes` to the registered QE GTS `MetricId` and `project` to the supported `projection_type`, with version `M3` and its digest. These identifiers and values are examples, not catalog defaults or literal SDK values.

1. Pricing publishes plan revision `P7` with a per-plan-unit rule of 100 GiB on `storage.allocated_bytes`, validating the unit, kind and subject against `S2` and freezing `S2` and `M3` by digest in the acceptance receipt (PD-P01).
2. Entitlements derives cap `322122547200` bytes for quantity three and the adapter maps the member to the QE `MetricId` and `projection_type` of `M3` (PD-P02).
3. Products later publishes `S3` with a new subject kind. Receipts that froze `S2` still resolve `S2`; nothing changes for existing subscriptions until a new acceptance.
4. If the registered metric becomes `Direct` and inert, Products publishes that as a versioned change; Entitlements reports the materialized but unenforced record as a finding and keeps the purchased obligation visible.

## 4. Acceptance

The cases are defined in the [quota enforcement proposal](quota-enforcement-proposal.md); Products owns their evidence side.

| Case | Products obligation |
| --- | --- |
| QE-C21 Metric drift | When a metric becomes `Direct` or inert or loses registration after materialization, the change is visible as a versioned mapping or descriptor change; the old version stays resolvable for existing receipts; no silent metric substitution occurs and no false readiness is reported. |

## 5. Open points

| Question | Owners | What this proposal needs |
| --- | --- | --- |
| ENT-Q01 | Pricing, Products, Entitlements | Which Products or provider descriptor supplies each allocation metric, unit and authorized subject kind; existing usage semantics may be reused only when enforcement equivalence is proved. |
| ENT-Q03 | Quota Enforcement, Products, Entitlements | Which operator safety ceilings coexist with commercial caps and with what precedence. |

These questions are registered in [DECISIONS.md](../DECISIONS.md#open-questions). The gate this proposal closes is E-U04 in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md).
