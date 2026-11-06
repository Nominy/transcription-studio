//! Optional document format contributions. The host attaches imported records to selected
//! audio; providers cannot invent audio paths, mutate a document, or enable themselves.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use studio_core::Project;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatDescriptor {
    pub id: String,
    pub name: String,
    /// Lowercase suffixes without leading dots, for host-owned file pickers.
    pub extensions: Vec<String>,
    pub media_type: String,
    pub can_import: bool,
    pub can_export: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedCue {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    /// A display label, not a host speaker ID. The host maps labels to local speakers.
    pub speaker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ImportContent {
    Timed(Vec<ImportedCue>),
    /// No fabricated timestamps: the host must retain this content for manual timing
    /// or an explicitly selected alignment provider before creating timed segments.
    Untimed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Explicitly include speaker display labels in output text. Off by default.
    pub speaker_labels: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatError {
    /// One-based source line, when an input location is known.
    pub line: Option<usize>,
    /// One-based cue ordinal, or original project segment ordinal during export.
    pub cue: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(cue) = self.cue {
            write!(f, "cue {cue}: ")?;
        }
        if let Some(line) = self.line {
            write!(f, "line {line}: ")?;
        }
        f.write_str(&self.message)
    }
}
impl std::error::Error for FormatError {}

pub trait FormatProvider: std::fmt::Debug + Send + Sync {
    fn descriptor(&self) -> FormatDescriptor;
    fn import(&self, bytes: &[u8]) -> Result<ImportContent, FormatError>;
    fn export(&self, project: &Project, options: &ExportOptions) -> Result<Vec<u8>, FormatError>;
}

#[derive(Debug)]
struct Registration {
    owner: String,
    descriptor: FormatDescriptor,
    provider: Arc<dyn FormatProvider>,
}

/// One registry owned by the shell's contribution registry, not a second activation
/// system. Only the host's enabled-extension registration path may add providers;
/// disabling/removing an owner calls `unregister_extension` in the same transaction.
#[derive(Debug, Default)]
pub struct FormatRegistry {
    providers: BTreeMap<String, Registration>,
}

impl FormatRegistry {
    pub fn register(
        &mut self,
        owner: &str,
        provider: Arc<dyn FormatProvider>,
    ) -> Result<(), String> {
        self.register_many(owner, vec![provider])
    }

    /// Atomic registration: invalid or duplicate contributions leave all existing
    /// entries unchanged, including contributions from other owners.
    pub fn register_many(
        &mut self,
        owner: &str,
        providers: Vec<Arc<dyn FormatProvider>>,
    ) -> Result<(), String> {
        if !crate::manifest::valid_id(owner) {
            return Err(format!("invalid extension owner `{owner}`"));
        }
        let mut ids = BTreeSet::new();
        let mut pending = Vec::with_capacity(providers.len());
        for provider in providers {
            let descriptor = provider.descriptor();
            if !crate::manifest::valid_id(&descriptor.id)
                || !descriptor.id.starts_with(owner)
                || descriptor.id.as_bytes().get(owner.len()) != Some(&b'.')
            {
                return Err(format!(
                    "format `{}` must be namespaced under owner `{owner}`",
                    descriptor.id
                ));
            }
            if descriptor.name.trim().is_empty()
                || descriptor.media_type.trim().is_empty()
                || descriptor.extensions.is_empty()
                || (!descriptor.can_import && !descriptor.can_export)
                || descriptor.extensions.iter().any(|suffix| {
                    suffix.is_empty()
                        || suffix.len() > 32
                        || !suffix
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
            {
                return Err(format!("invalid format descriptor `{}`", descriptor.id));
            }
            if self.providers.contains_key(&descriptor.id) || !ids.insert(descriptor.id.clone()) {
                return Err(format!("format `{}` is already registered", descriptor.id));
            }
            pending.push(Registration {
                owner: owner.into(),
                descriptor,
                provider,
            });
        }
        for registration in pending {
            self.providers
                .insert(registration.descriptor.id.clone(), registration);
        }
        Ok(())
    }

    pub fn unregister_extension(&mut self, owner: &str) {
        self.providers
            .retain(|_, registration| registration.owner != owner);
    }

    /// Stable format-ID ordering. Contains only currently registered contributions.
    pub fn entries(&self) -> Vec<FormatDescriptor> {
        self.providers
            .values()
            .map(|registration| registration.descriptor.clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn FormatProvider>> {
        self.providers
            .get(id)
            .map(|registration| &registration.provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Example(&'static str);
    impl FormatProvider for Example {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor {
                id: self.0.into(),
                name: "Example text".into(),
                extensions: vec!["txt".into()],
                media_type: "text/plain".into(),
                can_import: true,
                can_export: true,
            }
        }
        fn import(&self, bytes: &[u8]) -> Result<ImportContent, FormatError> {
            String::from_utf8(bytes.to_vec())
                .map(ImportContent::Untimed)
                .map_err(|e| FormatError {
                    line: None,
                    cue: None,
                    message: e.to_string(),
                })
        }
        fn export(&self, project: &Project, _: &ExportOptions) -> Result<Vec<u8>, FormatError> {
            Ok(project
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes())
        }
    }

    #[test]
    fn registration_is_atomic_owner_scoped_and_removal_preserves_other_owners() {
        let mut registry = FormatRegistry::default();
        registry
            .register("example.one", Arc::new(Example("example.one.txt")))
            .unwrap();
        registry
            .register("example.two", Arc::new(Example("example.two.txt")))
            .unwrap();
        assert!(
            registry
                .register_many(
                    "example.one",
                    vec![
                        Arc::new(Example("example.one.new")),
                        Arc::new(Example("example.one.txt"))
                    ]
                )
                .is_err()
        );
        assert!(registry.get("example.one.new").is_none());
        assert!(
            registry
                .register("example.one", Arc::new(Example("example.two.stolen")))
                .is_err()
        );
        registry.unregister_extension("example.one");
        assert!(registry.get("example.one.txt").is_none());
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(
            registry
                .get("example.two.txt")
                .unwrap()
                .import(b"text")
                .unwrap(),
            ImportContent::Untimed("text".into())
        );
    }
}
