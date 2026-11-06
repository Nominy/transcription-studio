use std::collections::BTreeMap;

use exaudio_extension::{ExtensionManifest, ModeManifest, ResolveError, Runtime, resolve_mode};

fn extension(id: &str, version: &str, dependencies: &[(&str, &str)]) -> ExtensionManifest {
    ExtensionManifest {
        id: id.into(),
        name: id.into(),
        version: version.parse().unwrap(),
        api: "^0.1".parse().unwrap(),
        runtime: Runtime::Wasm,
        description: String::new(),
        license: "MIT".into(),
        dependencies: dependencies
            .iter()
            .map(|(id, range)| ((*id).into(), range.parse().unwrap()))
            .collect(),
        permissions: vec![],
        modes: vec![],
        providers: vec![],
    }
}

fn owner(required: &[(&str, &str)]) -> ExtensionManifest {
    let mut extension = extension("example.workspace", "1.0.0", &[]);
    extension.modes.push(ModeManifest {
        id: "example.edit".into(),
        name: "Example editing".into(),
        requires: required
            .iter()
            .map(|(id, range)| ((*id).into(), range.parse().unwrap()))
            .collect(),
        recommends: BTreeMap::new(),
        provider: None,
        launch: Default::default(),
    });
    extension
}

#[test]
fn modes_resolve_shared_dependencies_once_before_dependents() {
    let owner = owner(&[("example.tags", "^1"), ("example.checks", "^1")]);
    let tags = extension("example.tags", "1.0.0", &[("example.language", "^1")]);
    let checks = extension("example.checks", "1.0.0", &[("example.language", "^1")]);
    let language = extension("example.language", "1.1.0", &[]);
    let plan = resolve_mode(&owner, "example.edit", &[&tags, &checks, &language]).unwrap();
    assert_eq!(
        plan.extensions
            .iter()
            .map(|extension| extension.id.as_str())
            .collect::<Vec<_>>(),
        [
            "example.language",
            "example.checks",
            "example.tags",
            "example.workspace"
        ]
    );
    assert_eq!(
        plan.required_by["example.language"]
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        ["example.checks", "example.tags"]
    );
    assert!(plan.required_by["example.checks"].contains("example.edit"));
}

#[test]
fn newest_incompatible_dependency_combination_backtracks_to_working_release() {
    let owner = owner(&[("example.a", "^1"), ("example.z", "^1")]);
    let a_new = extension("example.a", "1.2.0", &[("example.shared", "^2")]);
    let a_old = extension("example.a", "1.0.0", &[("example.shared", "^1")]);
    let z = extension("example.z", "1.0.0", &[("example.shared", "^1")]);
    let shared_v1 = extension("example.shared", "1.0.0", &[]);
    let shared_v2 = extension("example.shared", "2.0.0", &[]);
    let plan = resolve_mode(
        &owner,
        "example.edit",
        &[&a_new, &a_old, &z, &shared_v1, &shared_v2],
    )
    .unwrap();
    assert_eq!(
        plan.extensions
            .iter()
            .find(|extension| extension.id == "example.a")
            .unwrap()
            .version,
        "1.0.0".parse().unwrap()
    );
    assert_eq!(
        plan.extensions
            .iter()
            .find(|extension| extension.id == "example.shared")
            .unwrap()
            .version,
        "1.0.0".parse().unwrap()
    );
}

#[test]
fn unavailable_required_extension_prevents_activation_without_mutating_owner() {
    let mut owner = owner(&[("example.checks", "^2")]);
    owner.modes[0]
        .recommends
        .insert("example.optional".into(), "^1".parse().unwrap());
    let checks = extension("example.checks", "1.0.0", &[]);
    let before = owner.clone();
    assert!(matches!(
        resolve_mode(&owner, "example.edit", &[&checks]),
        Err(ResolveError::Unsatisfied { .. })
    ));
    assert_eq!(owner, before);
    let compatible = extension("example.checks", "2.0.0", &[]);
    let plan = resolve_mode(&owner, "example.edit", &[&compatible]).unwrap();
    assert_eq!(
        plan.extensions
            .iter()
            .map(|extension| extension.id.as_str())
            .collect::<Vec<_>>(),
        ["example.checks", "example.workspace"]
    );
}

#[test]
fn cycles_through_mode_requirements_are_rejected() {
    let owner = owner(&[("example.checks", "^1")]);
    let checks = extension("example.checks", "1.0.0", &[("example.workspace", "^1")]);
    let error = resolve_mode(&owner, "example.edit", &[&checks]).unwrap_err();
    assert!(
        matches!(error, ResolveError::Cycle(path) if path == "example.workspace → example.checks → example.workspace")
    );
}

#[test]
fn multiple_claims_to_same_identity_and_version_are_rejected() {
    let owner = owner(&[("example.checks", "^1")]);
    let checks = extension("example.checks", "1.0.0", &[]);
    let mut replacement = checks.clone();
    replacement
        .permissions
        .push(exaudio_extension::Permission::Secrets);
    assert!(matches!(
        resolve_mode(&owner, "example.edit", &[&checks, &replacement]),
        Err(ResolveError::DuplicateRelease(_))
    ));
}

#[test]
fn newer_host_api_releases_remain_visible_but_are_not_selected() {
    let owner = owner(&[("example.checks", "^1")]);
    let compatible = extension("example.checks", "1.0.0", &[]);
    let mut future = extension("example.checks", "1.1.0", &[]);
    future.api = "^2".parse().unwrap();
    future
        .validate()
        .expect("a future release is still valid catalog metadata");
    assert!(future.check_host_compatibility().is_err());
    let plan = resolve_mode(&owner, "example.edit", &[&future, &compatible]).unwrap();
    assert_eq!(plan.extensions[0].version, compatible.version);
    assert!(matches!(
        resolve_mode(&owner, "example.edit", &[&future]),
        Err(ResolveError::Unsatisfied { .. })
    ));
}

#[test]
fn mode_provider_must_come_from_the_resolved_set() {
    let mut owner = owner(&[("example.account", "^1")]);
    owner.modes[0].provider = Some("example.account.login".into());
    let mut account = extension("example.account", "1.0.0", &[]);
    account.providers.push(exaudio_extension::ProviderManifest {
        id: "example.account.login".into(),
        name: "Example account".into(),
        multiple: false,
    });
    let plan = resolve_mode(&owner, "example.edit", &[&account]).unwrap();
    assert_eq!(plan.provider_owner, Some("example.account"));

    let unrelated = extension("example.account", "1.0.0", &[]);
    assert!(matches!(
        resolve_mode(&owner, "example.edit", &[&unrelated]),
        Err(ResolveError::UnknownProvider { .. })
    ));
}

#[test]
fn duplicate_provider_declarations_are_invalid() {
    let mut account = extension("example.account", "1.0.0", &[]);
    for _ in 0..2 {
        account.providers.push(exaudio_extension::ProviderManifest {
            id: "example.account.login".into(),
            name: "Example account".into(),
            multiple: true,
        });
    }
    assert!(matches!(
        account.validate(),
        Err(exaudio_extension::ManifestError::DuplicateProvider(_))
    ));
}
