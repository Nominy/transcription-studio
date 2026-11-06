use crate::{Catalog, DeveloperPreview, InstallProvenance, Installed, Release};
use exaudio_extension::{ExtensionManifest, ModeManifest, Permission, resolve_mode};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type PermissionGrants = BTreeMap<String, BTreeSet<Permission>>;

/// Every currently open document/active mode supplies explicit version requirements,
/// including its owning extension. The marketplace never guesses global document state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveRequirement {
    pub required_by: String,
    pub extension: String,
    pub version: VersionReq,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostState {
    pub requirements: Vec<ActiveRequirement>,
    pub builtins: Vec<ExtensionManifest>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Change {
    Install {
        id: String,
        version: Version,
    },
    Update {
        id: String,
        from: Version,
        to: Version,
    },
    Keep {
        id: String,
        version: Version,
    },
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct PlanDiff {
    pub changes: Vec<Change>,
    pub releases: Vec<Release>,
    pub developer: Option<DeveloperPreview>,
    /// Unapproved requests, never grants. The host must obtain explicit consent.
    pub requested_permissions: PermissionGrants,
    pub required_by: BTreeMap<String, BTreeSet<String>>,
    pub blockers: Vec<String>,
}

pub fn plan_mode(
    catalog: &Catalog,
    owner: &ExtensionManifest,
    mode: &str,
    installed: &BTreeMap<String, Installed>,
    host: &HostState,
    grants: &PermissionGrants,
) -> PlanDiff {
    plan_selection(
        catalog,
        Selection::Mode(owner, mode),
        installed,
        host,
        grants,
    )
}
pub fn plan_release(
    catalog: &Catalog,
    release: &Release,
    installed: &BTreeMap<String, Installed>,
    host: &HostState,
    grants: &PermissionGrants,
) -> PlanDiff {
    if catalog.release(&release.manifest.id, &release.version) != Some(release)
        || release.withdrawn.is_some()
    {
        return PlanDiff {
            blockers: vec!["release is withdrawn or does not match index".into()],
            ..PlanDiff::default()
        };
    }
    plan_selection(
        catalog,
        Selection::Release(release),
        installed,
        host,
        grants,
    )
}
pub fn plan_developer(
    catalog: &Catalog,
    preview: &DeveloperPreview,
    installed: &BTreeMap<String, Installed>,
    host: &HostState,
    grants: &PermissionGrants,
) -> PlanDiff {
    plan_selection(
        catalog,
        Selection::Developer(preview),
        installed,
        host,
        grants,
    )
}

enum Selection<'a> {
    Mode(&'a ExtensionManifest, &'a str),
    Release(&'a Release),
    Developer(&'a DeveloperPreview),
}

fn plan_selection(
    catalog: &Catalog,
    selection: Selection<'_>,
    installed: &BTreeMap<String, Installed>,
    host: &HostState,
    grants: &PermissionGrants,
) -> PlanDiff {
    let mut diff = PlanDiff::default();
    if let Err(error) = catalog.validate() {
        diff.blockers.push(error.to_string());
        return diff;
    }
    let (owner, mode) = match &selection {
        Selection::Mode(owner, mode) => (*owner, Some(*mode)),
        Selection::Release(release) => (&release.manifest, None),
        Selection::Developer(preview) => {
            diff.developer = Some((*preview).clone());
            (&preview.manifest, None)
        }
    };
    let mut root = owner.clone();
    let synthetic_mode = "exaudio.marketplace-install";
    let mode = if let Some(mode) = mode {
        mode
    } else {
        // The temporary planning-only mode is never persisted or compared as package identity.
        root.modes.retain(|mode| mode.id != synthetic_mode);
        root.modes.push(ModeManifest {
            id: synthetic_mode.into(),
            name: "Install".into(),
            requires: BTreeMap::new(),
            recommends: BTreeMap::new(),
            provider: None,
            launch: Default::default(),
        });
        synthetic_mode
    };
    let mut candidates: BTreeMap<(String, Version), &ExtensionManifest> = BTreeMap::new();
    let mut developer_candidates = BTreeSet::new();
    for entry in &catalog.extensions {
        for release in entry
            .releases
            .iter()
            .filter(|release| release.withdrawn.is_none())
        {
            candidates.insert(
                (entry.id.clone(), release.version.clone()),
                &release.manifest,
            );
        }
    }
    for value in installed.values() {
        if matches!(value.provenance, InstallProvenance::Developer { .. })
            && let std::collections::btree_map::Entry::Vacant(candidate) =
                candidates.entry((value.manifest.id.clone(), value.manifest.version.clone()))
        {
            developer_candidates.insert(candidate.key().clone());
            candidate.insert(&value.manifest);
        }
    }
    for builtin in &host.builtins {
        candidates.retain(|(id, _), _| id != &builtin.id);
        developer_candidates.retain(|(id, _)| id != &builtin.id);
        candidates.insert((builtin.id.clone(), builtin.version.clone()), builtin);
    }
    let available: Vec<_> = candidates.values().copied().collect();
    let resolved = match resolve_mode(&root, mode, &available) {
        Ok(plan) => plan,
        Err(error) => {
            diff.blockers.push(error.to_string());
            return diff;
        }
    };
    diff.required_by = resolved
        .required_by
        .iter()
        .map(|(id, reasons)| {
            (
                (*id).to_owned(),
                reasons.iter().map(|reason| (*reason).to_owned()).collect(),
            )
        })
        .collect();
    let mut resulting: BTreeMap<String, ExtensionManifest> = installed
        .iter()
        .map(|(id, value)| (id.clone(), value.manifest.clone()))
        .collect();
    for selected in resolved.extensions {
        let manifest = if selected.id == owner.id {
            owner
        } else {
            selected
        };
        if host.builtins.iter().any(|builtin| builtin == manifest) {
            if manifest.id == owner.id && matches!(selection, Selection::Developer(_)) {
                diff.blockers.push(format!(
                    "{} is already supplied by an enabled built-in",
                    manifest.id
                ));
            }
            diff.changes.push(Change::Keep {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
            });
            continue;
        }
        resulting.insert(manifest.id.clone(), manifest.clone());
        let existing = installed.get(&manifest.id);
        let developer_root =
            manifest.id == owner.id && matches!(selection, Selection::Developer(_));
        let selected_developer = if manifest.id == owner.id {
            matches!(selection, Selection::Mode(_, _))
        } else {
            developer_candidates.contains(&(manifest.id.clone(), manifest.version.clone()))
        };
        let retained_developer = selected_developer
            && existing.is_some_and(|old| {
                old.manifest == *manifest
                    && matches!(old.provenance, InstallProvenance::Developer { .. })
            });
        let release = catalog
            .release(&manifest.id, &manifest.version)
            .filter(|release| release.withdrawn.is_none() && &release.manifest == manifest);
        if !developer_root && !retained_developer && release.is_none() {
            diff.blockers.push(format!(
                "{}@{} has no reviewed release or approved local installation",
                manifest.id, manifest.version
            ));
            continue;
        }
        let unchanged = existing.is_some_and(|old| {
            if developer_root {
                diff.developer.as_ref().is_some_and(|preview| old.sha256 == preview.sha256 && old.manifest == *manifest
                    && matches!(&old.provenance, InstallProvenance::Developer { source } if source == &preview.source))
            }
            else if retained_developer { true }
            else { matches!(&old.provenance, InstallProvenance::Reviewed { release: previous } if Some(previous.as_ref()) == release) }
        });
        if unchanged {
            diff.changes.push(Change::Keep {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
            });
        } else {
            diff.changes.push(match existing {
                Some(old) => Change::Update {
                    id: manifest.id.clone(),
                    from: old.manifest.version.clone(),
                    to: manifest.version.clone(),
                },
                None => Change::Install {
                    id: manifest.id.clone(),
                    version: manifest.version.clone(),
                },
            });
            if !developer_root && let Some(release) = release {
                diff.releases.push(release.clone());
            }
        }
        let approved = grants.get(&manifest.id);
        let missing: BTreeSet<_> = manifest
            .permissions
            .iter()
            .filter(|permission| !approved.is_some_and(|set| set.contains(*permission)))
            .cloned()
            .collect();
        if !missing.is_empty() {
            diff.requested_permissions
                .insert(manifest.id.clone(), missing);
        }
    }
    diff.blockers.extend(dependency_blockers(&resulting, host));
    diff
}

pub(crate) fn dependency_blockers(
    installed: &BTreeMap<String, ExtensionManifest>,
    host: &HostState,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut available: BTreeMap<&str, &ExtensionManifest> = installed
        .iter()
        .map(|(id, manifest)| (id.as_str(), manifest))
        .collect();
    for builtin in &host.builtins {
        if available.insert(&builtin.id, builtin).is_some() {
            errors.push(format!("{} conflicts with a host built-in", builtin.id));
        }
        if let Err(error) = builtin.validate() {
            errors.push(error.to_string());
        }
        if let Err(error) = builtin.check_host_compatibility() {
            errors.push(error.to_string());
        }
    }
    for requirement in &host.requirements {
        if !available
            .get(requirement.extension.as_str())
            .is_some_and(|manifest| requirement.version.matches(&manifest.version))
        {
            errors.push(format!(
                "{} requires {} {}",
                requirement.required_by, requirement.extension, requirement.version
            ));
        }
    }
    for manifest in available.values() {
        for (dependency, requirement) in &manifest.dependencies {
            if !available
                .get(dependency.as_str())
                .is_some_and(|manifest| requirement.matches(&manifest.version))
            {
                errors.push(format!(
                    "{} requires {} {}",
                    manifest.id, dependency, requirement
                ));
            }
        }
    }
    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for id in available.keys() {
        if dependency_cycle(id, &available, &mut visiting, &mut complete) {
            errors.push(format!("dependency cycle involving {id}"));
            break;
        }
    }
    errors
}
fn dependency_cycle<'a>(
    id: &'a str,
    available: &BTreeMap<&'a str, &'a ExtensionManifest>,
    visiting: &mut BTreeSet<&'a str>,
    complete: &mut BTreeSet<&'a str>,
) -> bool {
    if complete.contains(id) {
        return false;
    }
    if !visiting.insert(id) {
        return true;
    }
    if let Some(manifest) = available.get(id) {
        for dependency in manifest.dependencies.keys() {
            if available.contains_key(dependency.as_str())
                && dependency_cycle(dependency, available, visiting, complete)
            {
                return true;
            }
        }
    }
    visiting.remove(id);
    complete.insert(id);
    false
}
