//! Babel tag completions and deterministic text-hygiene commands.
pub mod autofix;
pub mod skaz;
pub mod tags;

use exaudio_extension::document::{
    CommandOutcome, CommandProvider, CommandRequest, Completion, CompletionProvider,
    CompletionRequest, DocumentProviders, DraftReplacement, TextRange,
};
use std::sync::Arc;
use studio_core::Project;

pub const ID: &str = "exaudio.babel-tags";
pub const MANIFEST_JSON: &str = include_str!("../manifest.json");

pub fn manifest() -> Result<exaudio_extension::manifest::ExtensionManifest, serde_json::Error> {
    serde_json::from_str(MANIFEST_JSON)
}

#[derive(Debug)]
pub struct BabelTagProvider;

impl CompletionProvider for BabelTagProvider {
    fn complete(&self, request: &CompletionRequest<'_>) -> Vec<Completion> {
        tags::completions_for_request(request)
    }
}

#[derive(Debug)]
pub struct BabelAutofixProvider;

/// The caller supplies its effective project, including pending drafts, for lane context.
pub fn previous_same_lane_text<'a>(project: &'a Project, segment_id: &str) -> &'a str {
    let Some(index) = project
        .segments
        .iter()
        .position(|segment| segment.id == segment_id)
    else {
        return "";
    };
    let lane = &project.segments[index].lane_id;
    project.segments[..index]
        .iter()
        .rev()
        .find(|segment| &segment.lane_id == lane)
        .map_or("", |segment| segment.text.as_str())
}

impl CommandProvider for BabelAutofixProvider {
    fn command_ids(&self) -> &[&str] {
        &["babel.autofix-row", "babel.autofix-all"]
    }

    fn execute(
        &self,
        command: &str,
        request: &CommandRequest<'_>,
    ) -> Result<CommandOutcome, String> {
        match command {
            "babel.autofix-row" => {
                let id = request.segment_id.ok_or("Select a row to autofix.")?;
                let segment = request
                    .project
                    .segment(id)
                    .ok_or("The selected row no longer exists.")?;
                let before = request.text.unwrap_or(&segment.text);
                let text = autofix::autofix_transcript_text(
                    before,
                    previous_same_lane_text(request.project, id),
                );
                if text == before {
                    return Ok(CommandOutcome::default());
                }
                Ok(CommandOutcome {
                    drafts: vec![DraftReplacement {
                        segment_id: id.into(),
                        text,
                        selection: None,
                    }],
                    ..CommandOutcome::default()
                })
            }
            "babel.autofix-all" => {
                // Keep earlier rewrites visible to later rows, matching sequential draft autofix.
                let mut previous_by_lane = std::collections::HashMap::<&str, usize>::new();
                let mut fixed: Vec<Option<String>> =
                    Vec::with_capacity(request.project.segments.len());
                for (index, segment) in request.project.segments.iter().enumerate() {
                    let previous =
                        previous_by_lane
                            .get(segment.lane_id.as_str())
                            .map_or("", |&index| {
                                fixed[index]
                                    .as_deref()
                                    .unwrap_or(&request.project.segments[index].text)
                            });
                    let text = autofix::autofix_transcript_text(&segment.text, previous);
                    fixed.push((text != segment.text).then_some(text));
                    previous_by_lane.insert(&segment.lane_id, index);
                }
                let drafts = request
                    .project
                    .segments
                    .iter()
                    .zip(fixed)
                    .filter_map(|(segment, text)| {
                        text.map(|text| DraftReplacement {
                            segment_id: segment.id.clone(),
                            text,
                            selection: None,
                        })
                    })
                    .collect();
                Ok(CommandOutcome {
                    drafts,
                    ..CommandOutcome::default()
                })
            }
            _ => Err(format!("Unknown Babel autofix command: {command}")),
        }
    }
}

#[derive(Debug)]
pub struct BabelSkazProvider;

impl CommandProvider for BabelSkazProvider {
    fn command_ids(&self) -> &[&str] {
        &["babel.skaz-selection", "babel.skaz-digit"]
    }

    fn execute(
        &self,
        command: &str,
        request: &CommandRequest<'_>,
    ) -> Result<CommandOutcome, String> {
        let id = request.segment_id.ok_or("Select a row to convert.")?;
        let segment = request
            .project
            .segment(id)
            .ok_or("The selected row no longer exists.")?;
        let text = request.text.unwrap_or(&segment.text);
        let selection = request.selection.ok_or("Select text to convert.")?;
        let selected = text
            .get(selection.start..selection.end)
            .ok_or("Invalid text selection.")?;
        let (replacement, caret) = match command {
            "babel.skaz-selection" => {
                let replacement = skaz::skaz_for_selection(selected)
                    .ok_or("Select a supported number or spoken digit sequence.")?;
                let caret = selection.start + replacement.len();
                (replacement, caret)
            }
            "babel.skaz-digit" => {
                let digit = request
                    .arguments
                    .get("digit")
                    .and_then(|value| value.as_str())
                    .filter(|digit| digit.len() == 1)
                    .and_then(|digit| digit.chars().next())
                    .filter(char::is_ascii_digit)
                    .ok_or("Provide one ASCII digit.")?;
                let replacement =
                    skaz::skaz_with_digit(digit, selected).ok_or("Select text to convert.")?;
                (replacement, selection.start + 1)
            }
            _ => return Err(format!("Unknown Babel SKAZ command: {command}")),
        };
        let mut updated = String::with_capacity(text.len() - selected.len() + replacement.len());
        updated.push_str(&text[..selection.start]);
        updated.push_str(&replacement);
        updated.push_str(&text[selection.end..]);
        Ok(CommandOutcome {
            drafts: vec![DraftReplacement {
                segment_id: id.into(),
                text: updated,
                selection: Some(TextRange {
                    start: caret,
                    end: caret,
                }),
            }],
            ..CommandOutcome::default()
        })
    }
}

pub fn register(providers: &mut DocumentProviders) {
    providers.completions.push(Arc::new(BabelTagProvider));
    providers.commands.push(Arc::new(BabelAutofixProvider));
    providers.commands.push(Arc::new(BabelSkazProvider));
}
