//! Native schema admission over committed test state, ordered by batch prerequisites.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{
    Admission, AdmissionFailure, Candidate, Decided, LifecycleStatus, Reason, Stored, decide,
    is_type,
};

#[derive(Clone, Copy)]
pub(super) enum Readiness<'a> {
    Ready,
    Waiting,
    Failed(&'a str),
}

pub(super) fn decide_batch(
    entities: &mut BTreeMap<String, Stored>,
    rules: &Admission,
    candidates: &[Candidate],
) -> Vec<Decided> {
    let selected: HashMap<_, _> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, candidate)| match candidate {
            Candidate::Register { gts_id, .. } if gts_id.is_type() => Some((gts_id.id(), i)),
            _ => None,
        })
        .collect();
    let dependencies: Vec<_> = candidates
        .iter()
        .map(|candidate| prerequisites(entities, &selected, candidate))
        .collect();
    let mut outcomes = vec![None; candidates.len()];
    while outcomes.iter().any(Option::is_none) {
        let mut progress = false;
        for (i, candidate) in candidates.iter().enumerate() {
            if outcomes[i].is_some() {
                continue;
            }
            let readiness = if let Some((_, id)) = dependencies[i]
                .iter()
                .find(|(slot, _)| matches!(outcomes[*slot], Some(Decided::Failed(_))))
            {
                Readiness::Failed(id)
            } else if dependencies[i]
                .iter()
                .any(|(slot, _)| outcomes[*slot].is_none())
            {
                Readiness::Waiting
            } else {
                Readiness::Ready
            };
            if let Some(outcome) = decide(entities, rules, candidate, readiness) {
                outcomes[i] = Some(outcome);
                progress = true;
            }
        }
        if !progress {
            // Only the members of a cycle are refused as invalid; the candidates that
            // wait on them are then blocked by that dependency, as the registry does.
            let members = cycle_members(&dependencies, &outcomes);
            for (i, outcome) in outcomes.iter_mut().enumerate() {
                if outcome.is_none() && members.contains(&i) {
                    *outcome = Some(Decided::Failed(AdmissionFailure::new(
                        Reason::InvalidSchema,
                        "cyclic in-batch schema prerequisites",
                    )));
                }
            }
        }
    }
    outcomes
        .into_iter()
        .map(|o| o.expect("every candidate decided"))
        .collect()
}

/// The undecided candidates that wait on themselves through other undecided ones.
/// Called only when no undecided candidate can progress, so at least one exists.
fn cycle_members(
    dependencies: &[Vec<(usize, String)>],
    outcomes: &[Option<Decided>],
) -> HashSet<usize> {
    let undecided = |slot: usize| outcomes[slot].is_none();
    (0..dependencies.len())
        .filter(|&start| undecided(start))
        .filter(|&start| {
            let mut seen = HashSet::new();
            let mut pending: Vec<usize> = dependencies[start].iter().map(|(s, _)| *s).collect();
            while let Some(slot) = pending.pop() {
                if slot == start {
                    return true;
                }
                if undecided(slot) && seen.insert(slot) {
                    pending.extend(dependencies[slot].iter().map(|(s, _)| *s));
                }
            }
            false
        })
        .collect()
}

/// Follow committed schema edges until a submitted type owns the next edge.
/// Never use its old stored revision when a selected revision fails.
fn prerequisites(
    entities: &BTreeMap<String, Stored>,
    selected: &HashMap<&str, usize>,
    candidate: &Candidate,
) -> Vec<(usize, String)> {
    let Candidate::Register {
        gts_id, content, ..
    } = candidate
    else {
        return Vec::new();
    };
    if !gts_id.is_type() {
        return Vec::new();
    }
    let mut pending = references(gts_id.id(), content);
    let mut seen = HashSet::from([gts_id.id().to_owned()]);
    let mut prerequisites = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(&slot) = selected.get(id.as_str()) {
            prerequisites.push((slot, id));
        } else if let Some(stored) = entities.get(&id)
            && stored.lifecycle == LifecycleStatus::Active
        {
            pending.extend(references(&id, &stored.content));
        }
    }
    prerequisites
}

fn references(id: &str, content: &serde_json::Value) -> Vec<String> {
    // Malformed references are rejected by native validation, not interpreted here.
    let mut refs: Vec<_> = gts::extract_gts_refs(content)
        .unwrap_or_default()
        .into_iter()
        .collect();
    if let Some(parent) = gts::GtsId::try_new(id).ok().and_then(|id| id.get_type_id()) {
        refs.push(parent);
    }
    refs
}

pub(super) fn validate_schema(
    entities: &BTreeMap<String, Stored>,
    candidate: &gts::GtsId,
    content: &serde_json::Value,
) -> Result<(), gts::StoreError> {
    let mut store = gts::GtsStore::new();
    for (id, stored) in entities.iter().filter(|(id, stored)| {
        id.as_str() != candidate.id() && is_type(id) && stored.lifecycle == LifecycleStatus::Active
    }) {
        // seed() deliberately permits invalid fixtures. An unrelated invalid seed
        // cannot poison admission; a needed but unusable schema fails native resolution.
        drop(store.register_schema(id, &stored.content));
    }
    store.register_schema(candidate.id(), content)?;
    let resolved = store.validate_schema_with(candidate.id(), gts::GtsRefValidation::None)?;
    // Match the real registry's requirement that managed artifacts materialize fully.
    store.resolve_schema_refs(&resolved.schema)?;
    Ok(())
}
