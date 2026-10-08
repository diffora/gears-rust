---
description: "Proposed Subscriptions, Rating and Billing full-replacement contract, recovery boundaries and sixteen conformance scenarios."
---

# Subscriptions, Rating and Billing replacement contract

**Status:** Subscriptions integration proposal, 2026-10-15. Provider adoption, exact mappings and runtime conformance remain open. Adjacent gear documents are unchanged by this PR. Use the [Rating proposal](rating-proposal.md) for the current PR #5175 baseline and the [Billing proposal](billing-proposal.md) for consumer ownership.

<!-- toc -->

- [RB01 identities and authoritative evidence](#rb01-identities-and-authoritative-evidence)
- [RB02 valuation and qualifying one-time actions](#rb02-valuation-and-qualifying-one-time-actions)
- [RB03 independent completeness and immutable cuts](#rb03-independent-completeness-and-immutable-cuts)
- [RB04 full replacements and invoice concurrency](#rb04-full-replacements-and-invoice-concurrency)
- [RB05 trial and technical-delay allocation](#rb05-trial-and-technical-delay-allocation)
- [RB06 event-independent recovery and delivery states](#rb06-event-independent-recovery-and-delivery-states)
- [RB07 delivery gates and conformance](#rb07-delivery-gates-and-conformance)

<!-- /toc -->

**RO-2026-10-15 integration.** [RO-2026-10-15](../features/04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) specifies frozen term/default evaluation and Contract-end handling, one renewal/dwell episode, durable cutoff versus actual stop/cancel, and independent retention/hold/delete/archive generations. Terms or payment events do not mutate running terms or reopen terminal state. Resource/hold adapters must prove stale-action suppression and hold-versus-delete arbitration at the actual execution boundary. Deferred notification delivery is not assumed complete. Exact provider mapping and RO-C01-RO-C16 execution remain open; no native LM/OSS dependency is introduced.

For RO cutoffs, RB accepts the versioned confirmed bound and truthful qualifying stop evidence as separate references. A delayed worker/stop callback cannot extend customer tariff past an established bound. Applied cancel releases normal occupancy only under Subscriptions; retention data alone is not a billed service. Billing preserves valid prior obligations and independently agreed retention fees, corrects erroneous tail once and retains financial history after resource deletion.


**RB-2026-10-15 — retained Subscriptions profile.** This is the shared target semantic contract for S-U02/S-U03/S-U08/S-U13 and the accepted S-Q03/S-Q04 directions. It defines the proposed profile instead of the older local Rating baseline's Adjustment-only and one-time-bypass paths. Adopting it in Rating/Billing requires their own PRs; this file does not amend those documents. It does not enable unrelated Pricing/Rating features, customer backdating/decreases, migration/import or a Change Orders authoring programme. Historical source IDs remain traceability; exact versioned SDK/storage/transport mappings and executed evidence are still delivery work. The older local Rating baseline contains specifications and gear metadata, not a delivered rating-core/pipeline/SDK implementation; a Billing orchestration provider is also not delivered here. Ledger accounting APIs do not implement this whole chain.

## RB01 identities and authoritative evidence

| Evidence | Owner and required contents | Invariant |
| --- | --- | --- |
| Commercial fact | Subscriptions: stable fact/obligation identity, authorized tenant/payer/group coordinates, version/digest, charge kind, component/item and purchase provenance, period/served extent, complete temporal slices and concurrent quantity-lot allocations, accepted terms and posture/cutoff references, predecessor | Money-free full replacement. Price ID, transport key and result revision never define a new economic occurrence. Same ID/version with different digest conflicts; old versions cannot replace newer accepted heads. |
| Accepted terms and calculation snapshot | Pricing owns immutable accepted bindings/policy; Rating owns its calculation snapshot/mapping of those exact inputs, with digests and recoverable historical references | Distinct identities, neither a mutable catalog lookup nor a recreated legacy CatalogVersion. Missing historical inputs block valuation; a correction cannot select today's price. |
| Canonical rated result | Rating: stable result/unit identity and immutable result version/digest/predecessor; source fact/version/digest, calculation/input revisions, original accepted binding/lot/quantity provenance, charge kind, currency, full supported monetary result at agreed precision and rounding-policy reference | A new version replaces the complete prior result for the same unit. No independent monetary delta is required or accepted on this boundary. A genuine zero result remains an explicit version with lineage. Pending/unpriceable is not zero. |
| Usage manifest | Rating: parent fact/version, pinned usage policy, authorized scope and window geometry, complete expected child identities, exact accepted result versions/digests and scope/usage/attribution closure evidence at declared source barriers; manifest revision/predecessor | Children are enumerated independently of received usage/results. Membership changes supersede the manifest; previously billed disappearing children need explicit zero/replacement lineage. Missing child or evidence prevents completeness. |
| Commercial seal | Subscriptions: immutable group/composition version and predecessor, full fact membership/minimum versions and digest at its recoverable source barrier | Proves obligations only, not usage closure, complete rating or invoice posting. Same-membership corrections advance fact requirements in a successor seal. |
| Financial target and disposition | Billing: immutable target ID/revision/digest, selected seal, exact fact/result/manifest versions, invoice/financial basis, benefit/advance allocations, execution identity and durable draft/posted/pending outcomes | Billing derives customer payable and accounting effects once from a complete supported target. A delivery acknowledgement or zero replacement is not proof of posting, refund or financial completion. |

Version orders are owner-local; no global ordering between fact, result, manifest, grant and invoice revisions is inferred. Authenticate and authorize every read/write, including historical replay; identifiers/cursors are not authority. Exact encoding and digest algorithms must be versioned and agreed before enabling the profile.

## RB02 valuation and qualifying one-time actions

Subscriptions owns WHEN, obligation identity, quantities/posture and frozen accepted terms; Rating owns tariff math, supported proration and tariff minimum-fee treatment; Billing owns invoice aggregation/rounding, financial allocation, corrections and collection/remedy coordination. Ledger records the selected posting contract. This adds no unsupported period floor/cap, FX, overlays or promotions. Accepted tariff aggregation scope must be preserved: do not charge a minimum once per transport chunk, temporal slice or purchase lot unless that is the accepted rule.

Complete frozen inputs can be valued and full results published without Billing's `periodState` projection. Rating never guesses open/posted state and never uses that projection as an evaluation-context prerequisite or valuation gate. Billing independently recovers authoritative invoice state/version before financial execution; unavailable financial history leaves that work pending.

Recurring and usage results retain the full source timeline. A same-item increase under different accepted bindings carries the original and new purchase lots together, never reprices old quantity or creates a second full-quantity obligation. Pooled metered usage across differently priced lots stays disabled until its explicit allocation contract is supported; the consumer cannot invent FIFO or proportional allocation.

A separately agreed chargeable one-time action produces one stable occurrence/fact. Rating values that occurrence at its frozen accepted tariff and returns the same canonical result family; it is not an hourly or recurring tick unit. Reprice, retry, resume, trial conversion, a new transport key or a replacement draft alone cannot create another occurrence. Cancellation/correction keeps the original occurrence lineage and may publish an explicit zero replacement; any separately agreed later purchase needs distinct business evidence. Subscriptions/Billing must not bypass Rating by copying the catalog amount. Existing advances remain financial evidence: a full EUR 50 rated fee with an applicable EUR 50 advance is still rated at EUR 50; Billing applies the advance once rather than collecting again. These figures are illustrative, not defaults or refund policy.

## RB03 independent completeness and immutable cuts

Subscriptions opens a usage parent even with no received usage and supplies commercial geometry/served extent. Rating enumerates the expected UTC hourly windows for the supported usage policy, including first/last partial extents, from that parent and authoritative scope. Scope/inventory, attribution and usage closure are independent required proofs. A proven empty scope can yield an explicit zero child with its evidence; absent messages, a missing resource read or elapsed wall time cannot. For a full October UTC parent the expected hourly geometry is 744 windows; receiving 743 results is incomplete even if their total looks plausible. No hourly Subscriptions timer or fact is introduced.

Billing selects one recoverable commercial seal and verifies every required fact, then every required recurring/one-time result and every usage parent manifest/child result. Minimum fact versions are lower bounds, not permission to mix arbitrary future heads: all selected versions must be compatible with the chosen/successor seal and exact manifests. Explicit zero obligations are still present with their original lineage. Unauthorized, missing, partial, conflicting or unsupported evidence leaves that financial target pending; it is not silently omitted from the invoice.

At local target acceptance, Billing serializes the group/head decision and stores the exact verified reference set and digest. No remote call occurs inside that transaction. Evidence is an immutable declared cut, not a globally simultaneous snapshot of all producers. A newer head already observed locally prevents a stale target from replacing it; a correction committed/delivered after the selected cut produces a successor target and the correction path. A seal never promises that no future correction can arrive. Finalization policy/required windows must come from supported accepted terms; the consumer cannot invent an early cutoff to conceal missing inputs.

## RB04 full replacements and invoice concurrency

Rating persists the complete replacement, source/result lineage, operation receipt and publication/recovery work atomically. The same full-result identity/version/digest returns its original outcome; changed content under that identity conflicts. A lower version cannot replace a newer accepted head. Independent legacy Adjustment deduplication does not create a second monetary lane for this profile. A worker prepared against a superseded local source/head cannot overwrite the newer head. Preserve old immutable versions for replay; local difference calculation may be diagnostic/internal, but the public monetary contract carries the full new result. No separate delta channel races this result.

Billing first validates the replacement's full input cut and source lineage. Out-of-order delivery is staged/recovered against the current head. A later full version may replace an older one only when the source/manifest chain proves the intervening scope and obligation history; unsupported gaps remain pending. A newer result with missing evidence does not erase the prior accepted target or declare the new one complete. Cancellation produces full zero/replacement facts/results; missing records never stand for reversal.

For an unposted draft, Billing replaces the applicable draft projection under the current invoice revision and membership rules. For already posted amounts, it derives the necessary correction from the accepted new full target and actual prior financial history; posted invoice financials remain immutable. The downstream correction may be a credit/debit posting under the financial owner's contract; this is not a Rating delta delivery lane. If the draft posts while replacement is prepared, the expected invoice-version guard must refuse the stale draft write and recover posted history before preparing the posted correction. A detached `periodState` read does not close this race. Rating's full output is unchanged by open/posted status; Billing alone chooses the safe financial path.

Billing atomically records the target, correction/replace intent, stable execution key, expected financial basis and outbox, then performs external work. Unknown posting results are recovered using that same identity before another financial effect can be planned against an assumed balance. Per-group/affected-invoice execution is ordered; a new target may be queued/coalesced only with a proven disposition of predecessor work. No cross-gear transaction or in-place Ledger edit is assumed.

Illustrative race: EUR 30 is posted; full target v2 is EUR 25 and its EUR 5 credit reply is lost; v3 then targets EUR 27. Recover v2 first. If the credit applied, v3 requires a EUR 2 debit against EUR 25; if v2 is authoritatively unapplied and fenced, the supported coalesced path may instead correct EUR 30 to EUR 27. Never post both independent credits of EUR 5 and EUR 3 or silently discard unknown v2. Existing advances, tax/rounding policy and financial allocations must be part of the actual financial basis; the illustration does not prescribe tax or refund mechanics.

## RB05 trial and technical-delay allocation

[TP](../TRIAL_PRICING_PROPOSAL.md#10-reciprocal-trial-delivery-contract) governs the shared grant: free trials use explicit zero bindings; credit trials preserve gross ordinary tariff results. Billing allocates bonus/platform/customer portions from full replacements under original grant restrictions and actual served/paid boundaries. A correction received after conversion remains attributable to its original served time. Expired credit cannot become spendable or restart service through a correction. No raw bonus amount or monetary delta is added to Subscriptions facts.

Confirmed cancellation cutoffs and authoritative stop evidence retain their actual coordinates. A technical tail after a known charge cutoff creates no ordinary customer tariff despite delayed physical stop; truthful runtime evidence is retained. Do not forge an earlier stop, mutate catalog prices to zero or mislabel collection pause. Billing corrects any erroneous customer tail through the same versioned financial path; explicit paid retention terms or valid prior debt remain separate. Unsupported boundary attribution stays pending rather than guessing the customer amount.

## RB06 event-independent recovery and delivery states

Events announce immutable references; they are not the only discovery path. Subscriptions exposes complete group/fact/seal snapshots plus commit-ordered catch-up for old-period corrections, as specified in [consumer recovery](../features/09-consumer-contracts.md#38-recover-gaps-cursor-loss-and-retained-history). Rating must likewise expose authorized immutable result/manifest reads and a recoverable change stream with committed partition barriers, stable snapshot pagination, explicit continuation/completion and typed cursor-expiry/history-loss outcomes. Billing retains target/execution outcome reads and recovers each downstream financial operation. A due-date scan alone cannot find a correction to last month's invoice committed today.

Consumers persist accepted inbox/projection and applied checkpoint atomically only after required references are validated. An incomplete page, missing referenced version or sequence gap cannot advance the applied checkpoint past unresolved work. Broker acknowledgement can mean durable receipt/staging, not valuation/completeness/posting. Keep received, validated/staged, target accepted, financial work pending and financial effect applied/replayed distinguishable without inventing a shared wire enum.

Snapshot expiry with intact history requires rebootstrap; actual retained-history loss requires complete historical repair or an explicit pending repair exception. Current active-resource/subscription lists cannot reconstruct removed/zero obligations or prior financial history. Replay retention must cover supported correction/finalization/outage windows; exact duration/limits are release configuration. Financially referenced terms, attribution, versions and execution lineage survive operational deletion. Reauthorize each page; expanded/changed scope needs a fresh bootstrap rather than reusing an old filtered checkpoint.

## RB07 delivery gates and conformance

| Owner | Required delivery |
| --- | --- |
| Subscriptions | Versioned money-free fact/lot/occurrence, seal and attribution/recovery contracts; preserve actual boundaries and immutable identities. No monetary evaluator or new backdate/import API. |
| Rating | Supported accepted-binding mapping; recurring, usage and qualifying one-time valuation; full result persistence/replacement; expected-window manifests, scope/usage proofs and event-independent recovery. |
| Billing | Provider contract for complete target validation, shared benefit/advance allocation, invoice-version guards, serialized draft/posted correction execution and unknown-outcome recovery. |
| Ledger / financial adapters | Explicit mapping to existing idempotent posting/allocation/correction primitives, stable business keys and authoritative outcome recovery; a posting receipt alone does not prove the upstream group complete. |

| Case | Intervention | Required outcome |
| --- | --- | --- |
| RB-C01 | A fact/result version replays, arrives stale or conflicts on digest | Replay has no new effect; stale cannot replace newer head; same-version conflict is explicit. |
| RB-C02 | Full result changes EUR 30 to EUR 25 | Billing targets EUR 25 once, replacing draft or correcting posted history; never bills EUR 55 or expects an independent Rating delta. |
| RB-C03 | Zero-obligation replacement or missing fact | Explicit zero retains lineage and corrects prior effects; absence stays pending, never implicit reversal. |
| RB-C04 | One of 744 required October hourly children or scope/usage proof is missing | Parent/group incomplete; a proven empty hour is explicitly zero with closure evidence. |
| RB-C05 | A membership or same-member fact correction races seal/result delivery | Accept one compatible immutable cut; a newer observed head cannot be overwritten, later corrections create successor targets. |
| RB-C06 | Ten units under A gain two under B mid-period | Full timeline retains A and B lot terms; no reprice of original quantity or duplicate twelve-unit obligation; unsupported pooled usage allocation refuses. |
| RB-C07 | One-time action retries with changed transport key; a full/partial advance exists | Same qualifying occurrence and original tariff; Billing applies applicable advance once, never captures again merely because it received valuation. |
| RB-C08 | Draft posts between replacement preparation and write | Revision fence rejects stale draft mutation; recover posted basis and use one financial correction path. |
| RB-C09 | v2 correction reply is lost before v3 arrives | Recover/fence v2 before deriving v3 effects; EUR 30→25→27 converges to EUR 27 once. |
| RB-C10 | All events are lost, then an old-period correction is committed | Snapshot plus ordered catch-up discovers the complete membership and correction; no duplicate financial effect. |
| RB-C11 | Consumer crashes before checkpoint, cursor expires or history is lost | Replay exact immutable work; distinguish rebootstrap from repair-required loss, never skip a gap. |
| RB-C12 | Trial valuation is corrected after expiry/paid conversion | Preserve gross/time/terms and same-grant allocation; no expired spend, revived service or invented trial debt. |
| RB-C13 | Runtime stop is delayed beyond confirmed cutoff | Preserve truthful runtime; no ordinary customer tail charge, idempotent correction if already posted. |
| RB-C14 | Frozen binding or boundary evidence is unavailable | Pending/unpriceable, no current-price fallback, zero default or guessed time split. |
| RB-C15 | Invoice/Ledger write commits but response is lost | Recover the same execution/business key and effect; target acceptance is not falsely reported as financial completion. |
| RB-C16 | Authorization changes or incomplete snapshot pages arrive | No leaked history, mixed-scope completeness or checkpoint beyond missing references; new scope bootstraps independently. |

These refine V05-V10/V12/V13/V19 and TP/FA cases; they are specified acceptance scenarios, not executed tests. This Subscriptions proposal specifies the required semantics; adjacent-owner document adoption remains separate work. Exact producer/consumer versions, Billing/ledger mapping, implementation and independent owner ratification remain open capability gates. No schema, service or runtime conformance is claimed to exist because this section is present.
