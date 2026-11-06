//! Side-effect-free dependency planning. A caller must finish permission approval and
//! package validation before applying a plan; failure never partially activates a mode.

use std::collections::{BTreeMap, BTreeSet};

use semver::VersionReq;
use thiserror::Error;

use crate::{ExtensionManifest, ManifestError};

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("no compatible extension set for {requirements}")]
    Unsatisfied { requirements: String },
    #[error("dependency cycle: {0}")]
    Cycle(String),
    #[error("mode `{0}` is not declared by its extension")]
    UnknownMode(String),
    #[error("duplicate extension release `{0}`")]
    DuplicateRelease(String),
    #[error("mode `{mode}` needs provider `{provider}`, which no required extension declares")]
    UnknownProvider { mode: String, provider: String },
}

/// A dependency-first plan. Shared dependencies occur once. Recommendations are excluded;
/// selecting them is a separate user action, not a prerequisite for opening a document.
#[derive(Debug)]
pub struct ActivationPlan<'a> {
    pub extensions: Vec<&'a ExtensionManifest>,
    /// Direct dependents, including the root mode, for required-by UI and disable guards.
    pub required_by: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// Extension declaring the mode's provider type, when the mode has one.
    pub provider_owner: Option<&'a str>,
}

/// Resolves an installed or bundled mode together with its hard requirements. The root
/// version is fixed: this operation cannot silently update the extension owning the mode.
pub fn resolve_mode<'a>(
    owner: &'a ExtensionManifest,
    mode_id: &'a str,
    available: &[&'a ExtensionManifest],
) -> Result<ActivationPlan<'a>, ResolveError> {
    owner.validate()?;
    owner.check_host_compatibility()?;
    let mode = owner
        .modes
        .iter()
        .find(|mode| mode.id == mode_id)
        .ok_or_else(|| ResolveError::UnknownMode(mode_id.to_owned()))?;
    let mut catalog: BTreeMap<&str, Vec<&ExtensionManifest>> = BTreeMap::new();
    for manifest in available {
        manifest.validate()?;
        let versions = catalog.entry(manifest.id.as_str()).or_default();
        if versions
            .iter()
            .any(|existing| existing.version == manifest.version)
        {
            return Err(ResolveError::DuplicateRelease(format!(
                "{}@{}",
                manifest.id, manifest.version
            )));
        }
        versions.push(manifest);
    }
    for versions in catalog.values_mut() {
        versions.retain(|manifest| manifest.check_host_compatibility().is_ok());
        versions.sort_unstable_by(|left, right| right.version.cmp(&left.version));
    }
    let mut selected = BTreeMap::from([(owner.id.as_str(), owner)]);
    let mut requirements: BTreeMap<&str, Vec<&VersionReq>> = BTreeMap::new();
    for (id, requirement) in mode.requires.iter().chain(&owner.dependencies) {
        requirements.entry(id).or_default().push(requirement);
    }
    if !solve(&catalog, &mut requirements, &mut selected) {
        return Err(ResolveError::Unsatisfied {
            requirements: mode
                .requires
                .iter()
                .chain(&owner.dependencies)
                .map(|(id, version)| format!("{id} {version}"))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    let mut ordered = Vec::with_capacity(selected.len());
    let mut visiting = Vec::new();
    let mut complete = BTreeSet::new();
    // Treat the mode requirements as edges from its owning extension. This also catches a
    // dependency that points back to that owner through a mode-only requirement.
    visit(
        owner,
        owner,
        &mode.requires,
        &selected,
        &mut visiting,
        &mut complete,
        &mut ordered,
    )?;
    let mut required_by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for extension in &ordered {
        for dependency in extension.dependencies.keys() {
            required_by
                .entry(dependency)
                .or_default()
                .insert(&extension.id);
        }
    }
    for dependency in mode.requires.keys() {
        required_by.entry(dependency).or_default().insert(mode_id);
    }
    required_by.entry(&owner.id).or_default().insert(mode_id);
    let provider_owner = match &mode.provider {
        None => None,
        Some(provider) => Some(
            ordered
                .iter()
                .find(|extension| extension.providers.iter().any(|p| &p.id == provider))
                .map(|extension| extension.id.as_str())
                .ok_or_else(|| ResolveError::UnknownProvider {
                    mode: mode_id.to_owned(),
                    provider: provider.clone(),
                })?,
        ),
    };
    Ok(ActivationPlan {
        extensions: ordered,
        required_by,
        provider_owner,
    })
}

fn solve<'a>(
    catalog: &BTreeMap<&'a str, Vec<&'a ExtensionManifest>>,
    requirements: &mut BTreeMap<&'a str, Vec<&'a VersionReq>>,
    selected: &mut BTreeMap<&'a str, &'a ExtensionManifest>,
) -> bool {
    if requirements.iter().any(|(id, ranges)| {
        selected.get(id).is_some_and(|extension| {
            ranges
                .iter()
                .any(|range| !range.matches(&extension.version))
        })
    }) {
        return false;
    }
    let Some((&id, ranges)) = requirements
        .iter()
        .find(|(id, _)| !selected.contains_key(*id))
    else {
        return true;
    };
    let Some(candidates) = catalog.get(id) else {
        return false;
    };
    // Requirements for this ID may change during recursion. Keep this branch's constraints
    // separately so candidate iteration does not hold a borrow over recursive mutations.
    let ranges = ranges.clone();
    for &candidate in candidates {
        if ranges
            .iter()
            .any(|range| !range.matches(&candidate.version))
        {
            continue;
        }
        selected.insert(id, candidate);
        for (dependency, range) in &candidate.dependencies {
            requirements.entry(dependency).or_default().push(range);
        }
        if solve(catalog, requirements, selected) {
            return true;
        }
        for dependency in candidate.dependencies.keys() {
            if let Some(ranges) = requirements.get_mut(dependency.as_str()) {
                ranges.pop();
                if ranges.is_empty() {
                    requirements.remove(dependency.as_str());
                }
            }
        }
        selected.remove(id);
    }
    false
}

fn visit<'a>(
    extension: &'a ExtensionManifest,
    owner: &'a ExtensionManifest,
    mode_requires: &'a BTreeMap<String, VersionReq>,
    selected: &BTreeMap<&'a str, &'a ExtensionManifest>,
    visiting: &mut Vec<&'a str>,
    complete: &mut BTreeSet<&'a str>,
    ordered: &mut Vec<&'a ExtensionManifest>,
) -> Result<(), ResolveError> {
    if complete.contains(extension.id.as_str()) {
        return Ok(());
    }
    if let Some(start) = visiting.iter().position(|&id| id == extension.id) {
        let mut cycle = visiting[start..].join(" → ");
        cycle.push_str(" → ");
        cycle.push_str(&extension.id);
        return Err(ResolveError::Cycle(cycle));
    }
    visiting.push(&extension.id);
    for dependency in extension
        .dependencies
        .keys()
        .chain(mode_requires.keys().filter(|_| extension.id == owner.id))
    {
        visit(
            selected[dependency.as_str()],
            owner,
            mode_requires,
            selected,
            visiting,
            complete,
            ordered,
        )?;
    }
    visiting.pop();
    complete.insert(&extension.id);
    ordered.push(extension);
    Ok(())
}
