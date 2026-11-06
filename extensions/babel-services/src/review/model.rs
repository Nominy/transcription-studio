//! Typed Reviewer/Drafting backend responses and their parsers.

use anyhow::{Context, Result, anyhow, bail};
use exaudio_babel_domain::i18n::localized;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewerTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub score: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSuggestion {
    pub proposal_id: String,
    pub operation: String,
    pub category: String,
    pub target_template_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub report_texts: Vec<String>,
    pub reason: String,
    pub source_card_ids: Vec<String>,
    /// `pending` when absent.
    pub decision: String,
}

/// Exact change evidence of a session card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceDetail {
    TextDiff {
        before: String,
        after: String,
        inline_diff: Option<String>,
    },
    Raw(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewerCard {
    pub id: String,
    pub change_index: i64,
    pub kind: String,
    pub summary: String,
    pub evidence: String,
    pub evidence_detail: Option<EvidenceDetail>,
    pub categories: Vec<String>,
    pub matched_template_id: Option<String>,
    pub template_title: Option<String>,
    pub template_description: Option<String>,
    pub initial_matched_template_id: Option<String>,
    pub initial_template_title: Option<String>,
    pub match_source: Option<String>,
    pub opinion_text: Option<String>,
    pub rationale: Option<String>,
    /// Reviewer's comment for this card (from `comments.cardComments`).
    pub comment: String,
}

impl ReviewerCard {
    /// `evidenceDetail`, else the raw evidence text.
    pub fn evidence_view(&self) -> Option<EvidenceDetail> {
        self.evidence_detail.clone().or_else(|| {
            let raw = self.evidence.trim();
            (!raw.is_empty()).then(|| EvidenceDetail::Raw(raw.to_string()))
        })
    }

    /// Header summary: before → after, else raw evidence, else the summary.
    pub fn display_summary(&self) -> String {
        match self.evidence_view() {
            Some(EvidenceDetail::TextDiff { before, after, .. })
                if !before.trim().is_empty() || !after.trim().is_empty() =>
            {
                let side = |text: &str| {
                    let text = text.trim();
                    if text.is_empty() {
                        localized("(empty)", "(пусто)").to_owned()
                    } else {
                        text.to_owned()
                    }
                };
                format!("{} → {}", side(&before), side(&after))
            }
            Some(EvidenceDetail::Raw(text)) => text,
            _ if !self.summary.is_empty() => self.summary.clone(),
            _ => localized("Change", "Изменение").to_owned(),
        }
    }

    /// How the card's template was matched.
    pub fn match_source_label(&self) -> &'static str {
        match self.match_source.as_deref() {
            Some("manual")
                if self.initial_matched_template_id.is_some()
                    && self.initial_matched_template_id != self.matched_template_id =>
            {
                localized("Reassigned by hand", "Переназначено вручную")
            }
            Some("manual") => localized("Matched by hand", "Подобрано вручную"),
            Some("manual_cleared") => localized("Template removed", "Шаблон убран"),
            Some("model") => localized("Matched by AI", "Подобрано ИИ"),
            _ if self.matched_template_id.is_some() => localized("Matched", "Подобрано"),
            _ => localized("Unmatched", "Без шаблона"),
        }
    }

    /// Originally matched template, shown when the match changed.
    pub fn previous_template(&self) -> Option<&str> {
        let initial = self.initial_matched_template_id.as_deref()?;
        (self.matched_template_id.as_deref() != Some(initial))
            .then(|| self.initial_template_title.as_deref().unwrap_or(initial))
    }
}

/// Grader rubric categories in display order (the server's category names).
pub const GRADE_CATEGORIES: [&str; 5] = [
    "Word Accuracy",
    "Timestamp Accuracy",
    "Punctuation & Formatting",
    "Tags & Emphasis",
    "Segmentation",
];

/// Display title of a grader category in the current UI language.
pub fn grade_category_title(category: &str) -> &str {
    let (en, ru) = match category {
        "Word Accuracy" => ("Word accuracy", "Точность слов"),
        "Timestamp Accuracy" => ("Timestamp accuracy", "Точность таймкодов"),
        "Punctuation & Formatting" => ("Punctuation and formatting", "Пунктуация и форматирование"),
        "Tags & Emphasis" => ("Tags and emphasis", "Теги и акценты"),
        "Segmentation" => ("Segmentation", "Сегментация"),
        _ => return category,
    };
    localized(en, ru)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeSample {
    pub kind: String,
    pub severity: String,
    pub annotation_id: String,
    pub note: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryGrade {
    pub category: &'static str,
    /// 1 (best) ..= 3; editable before applying.
    pub score: u8,
    pub score_cap: u8,
    /// Rubric explanation generated by the backend.
    pub note: String,
    pub evidence_count: usize,
    pub samples: Vec<GradeSample>,
}

/// `/api/review/grade` result; `grades` holds all five categories in [`GRADE_CATEGORIES`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeResult {
    pub review_action_id: String,
    pub original_action_id: String,
    pub version: String,
    pub model: Option<String>,
    pub grades: Vec<CategoryGrade>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSession {
    pub session_id: String,
    pub review_action_id: String,
    pub backend_service: Option<String>,
    pub backend_release: Option<String>,
    pub session_comment: String,
    pub cards: Vec<ReviewerCard>,
    pub suggestions: Vec<ReviewerSuggestion>,
}

impl ReviewerSuggestion {
    pub fn is_pending(&self) -> bool {
        self.decision == "pending"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerFeedback {
    /// `Uncategorized` when absent.
    pub category: String,
    pub score: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedReview {
    pub model: Option<String>,
    pub received_at: Option<String>,
    pub template_registry_version: Option<String>,
    pub matched_template_ids: Vec<String>,
    pub feedback: Vec<ReviewerFeedback>,
    /// Backend cards, else classification cards (`change-<n>`), else one card per feedback item
    /// (`feedback-<i>`, kind `CATEGORY FEEDBACK`).
    pub cards: Vec<ReviewerCard>,
    /// Echoed into analytics as `prepared`.
    pub prepared: Value,
    /// Echoed into analytics as `aiReview`.
    pub ai_review: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerFinalize {
    pub session_id: String,
    pub mode: String,
    pub applied_at: Option<String>,
    pub backend_service: Option<String>,
    pub backend_release: Option<String>,
    pub feedback: Vec<ReviewerFeedback>,
}

/// Finalize response: the finalize summary plus the generated-review view of the same payload.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalizeOutcome {
    pub finalize: ReviewerFinalize,
    pub generated: GeneratedReview,
}

pub fn parse_reviewer_session(payload: &str) -> Result<ReviewerSession> {
    let value = parse_json(payload)?;
    let session_id = required_string(&value, "sessionId")?;
    let review_action_id = required_string(&value, "reviewActionId")?;
    let comments = value.get("comments").unwrap_or(&Value::Null);
    let card_comments = comments.get("cardComments").unwrap_or(&Value::Null);
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    let cards = value
        .get("cards")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("The server's reply has no cards."))?
        .iter()
        .map(|card| reviewer_card_from_value(card, card_comments))
        .collect();
    let suggestions = value
        .get("suggestions")
        .or_else(|| value.get("proposals"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|suggestion| ReviewerSuggestion {
                    proposal_id: optional_string(suggestion, "proposalId").unwrap_or_default(),
                    operation: optional_string(suggestion, "operation").unwrap_or_default(),
                    category: optional_string(suggestion, "category").unwrap_or_default(),
                    target_template_id: optional_string(suggestion, "targetTemplateId"),
                    title: optional_string(suggestion, "title").unwrap_or_default(),
                    description: optional_string(suggestion, "description"),
                    report_texts: string_array(suggestion.get("reportTexts")),
                    reason: optional_string(suggestion, "reason").unwrap_or_default(),
                    source_card_ids: string_array(suggestion.get("sourceCardIds")),
                    decision: optional_string(suggestion, "decision")
                        .unwrap_or_else(|| "pending".to_string()),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ReviewerSession {
        session_id,
        review_action_id,
        backend_service: backend_version
            .get("service")
            .and_then(Value::as_str)
            .map(str::to_string),
        backend_release: backend_version
            .get("release")
            .and_then(Value::as_str)
            .map(str::to_string),
        session_comment: comments
            .get("sessionComment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        cards,
        suggestions,
    })
}

pub fn parse_generated_review(payload: &str) -> Result<GeneratedReview> {
    let value = parse_json(payload)?;
    let ai_review = value
        .get("llm")
        .filter(|value| value.is_object())
        .or_else(|| value.get("aiReview").filter(|value| value.is_object()))
        .unwrap_or(&value);
    let feedback_value = ai_review
        .get("feedback")
        .or_else(|| value.get("categoryFeedback"))
        .or_else(|| value.get("feedback"))
        .unwrap_or(&Value::Null);
    let feedback = feedback_value
        .as_array()
        .map(|items| items.iter().map(reviewer_feedback_from_value).collect())
        .unwrap_or_else(Vec::new);
    let mut cards: Vec<ReviewerCard> = value
        .get("cards")
        .or_else(|| ai_review.get("cards"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|card| reviewer_card_from_value(card, &Value::Null))
                .collect()
        })
        .unwrap_or_default();
    if cards.is_empty() {
        cards = ai_review
            .get("classifications")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(index, classification)| classification_card(index, classification))
                    .collect()
            })
            .unwrap_or_default();
    }
    if cards.is_empty() {
        cards = feedback
            .iter()
            .enumerate()
            .map(|(index, feedback)| ReviewerCard {
                id: format!("feedback-{index}"),
                change_index: index as i64,
                kind: "CATEGORY FEEDBACK".to_string(),
                summary: feedback.category.clone(),
                evidence: feedback.note.clone(),
                evidence_detail: None,
                categories: vec![feedback.category.clone()],
                matched_template_id: None,
                template_title: None,
                template_description: None,
                initial_matched_template_id: None,
                initial_template_title: None,
                match_source: None,
                opinion_text: Some(feedback.note.clone()),
                rationale: None,
                comment: String::new(),
            })
            .collect();
    }
    if cards.is_empty() && feedback.is_empty() {
        bail!("The server's reply has no feedback.");
    }
    Ok(GeneratedReview {
        model: optional_string(ai_review, "model"),
        received_at: optional_string(ai_review, "receivedAt"),
        template_registry_version: optional_string(ai_review, "templateRegistryVersion"),
        matched_template_ids: string_array(
            ai_review
                .get("matchedTemplateIds")
                .or_else(|| value.get("matchedTemplateIds")),
        ),
        prepared: value.get("prepared").cloned().unwrap_or(Value::Null),
        ai_review: if ai_review.is_object() {
            ai_review.clone()
        } else {
            json!({ "feedback": feedback_value })
        },
        feedback,
        cards,
    })
}

/// Parses a finalize response for the request (`session_id`, `{ "mode": ... }`).
pub fn parse_finalize_outcome(
    request_session_id: &str,
    request_body: &Value,
    payload: &str,
) -> Result<FinalizeOutcome> {
    let value = parse_json(payload)?;
    let generated = parse_generated_review(payload)?;
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    let finalize = ReviewerFinalize {
        session_id: optional_string(&value, "sessionId")
            .unwrap_or_else(|| request_session_id.to_string()),
        mode: optional_string(&value, "mode")
            .or_else(|| optional_string(request_body, "mode"))
            .unwrap_or_else(|| "apply".to_string()),
        applied_at: optional_string(&value, "appliedAt"),
        backend_service: backend_version
            .get("service")
            .and_then(value_to_compact_string),
        backend_release: backend_version
            .get("release")
            .and_then(value_to_compact_string)
            .or_else(|| value_to_compact_string(backend_version)),
        feedback: generated.feedback.clone(),
    };
    Ok(FinalizeOutcome {
        finalize,
        generated,
    })
}

pub fn parse_template_search(payload: &str) -> Result<Vec<ReviewerTemplate>> {
    let value = parse_json(payload)?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("The server's reply has no results."))?;
    Ok(results
        .iter()
        .map(|result| ReviewerTemplate {
            id: optional_string(result, "id").unwrap_or_default(),
            title: optional_string(result, "title").unwrap_or_default(),
            description: optional_string(result, "description").unwrap_or_default(),
            category: optional_string(result, "category").unwrap_or_default(),
            score: optional_string(result, "score").unwrap_or_default(),
        })
        .collect())
}

/// Parses a grade result; the five categories must each appear once with scores 1..=3.
pub fn parse_grade_result(payload: &str) -> Result<GradeResult> {
    let value = parse_json(payload)?;
    let items = value
        .get("grades")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("The server's reply has no grades."))?;
    if items.len() != GRADE_CATEGORIES.len() {
        bail!("The server's grades don't cover exactly five categories.");
    }
    let mut grades = Vec::with_capacity(GRADE_CATEGORIES.len());
    for category in GRADE_CATEGORIES {
        let mut matches = items
            .iter()
            .filter(|item| item.get("category").and_then(Value::as_str) == Some(category));
        let item = matches
            .next()
            .ok_or_else(|| anyhow!("The server's grades are missing {category}."))?;
        if matches.next().is_some() {
            bail!("The server's grades repeat {category}.");
        }
        let score_of = |key: &str| {
            item.get(key)
                .and_then(Value::as_u64)
                .filter(|score| (1..=3).contains(score))
                .map(|score| score as u8)
        };
        let score =
            score_of("score").ok_or_else(|| anyhow!("The {category} grade must be 1, 2 or 3."))?;
        let evidence = item.get("evidence").unwrap_or(&Value::Null);
        let raw_text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        grades.push(CategoryGrade {
            category,
            score,
            score_cap: score_of("scoreCap").unwrap_or(3),
            note: raw_text(item, "note").unwrap_or_default(),
            evidence_count: evidence
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            samples: evidence
                .get("samples")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|sample| GradeSample {
                    kind: raw_text(sample, "kind").unwrap_or_default(),
                    severity: raw_text(sample, "severity").unwrap_or_default(),
                    annotation_id: raw_text(sample, "annotationId").unwrap_or_default(),
                    note: raw_text(sample, "note").unwrap_or_default(),
                    before: raw_text(sample, "before"),
                    after: raw_text(sample, "after"),
                })
                .collect(),
        });
    }
    Ok(GradeResult {
        review_action_id: required_string(&value, "reviewActionId")?,
        original_action_id: optional_string(&value, "originalActionId").unwrap_or_default(),
        version: optional_string(&value, "version").unwrap_or_default(),
        model: optional_string(&value, "model"),
        grades,
    })
}

fn classification_card(index: usize, classification: &Value) -> ReviewerCard {
    let change_index = classification
        .get("change")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(index as i64);
    let template_id = optional_string(classification, "templateId");
    ReviewerCard {
        id: format!("change-{change_index}"),
        change_index,
        kind: "TEMPLATE MATCH".to_string(),
        summary: if template_id.is_some() {
            localized("Matched a template", "Шаблон подобран").to_owned()
        } else {
            localized(
                "Classified by Magic Review",
                "Классифицировано Magic Review",
            )
            .to_owned()
        },
        evidence: classification.to_string(),
        evidence_detail: None,
        categories: Vec::new(),
        matched_template_id: template_id,
        template_title: None,
        template_description: None,
        initial_matched_template_id: None,
        initial_template_title: None,
        match_source: Some("model".to_string()),
        opinion_text: None,
        rationale: None,
        comment: String::new(),
    }
}

fn reviewer_feedback_from_value(value: &Value) -> ReviewerFeedback {
    ReviewerFeedback {
        category: optional_string(value, "category").unwrap_or_else(|| "Uncategorized".to_string()),
        score: optional_string(value, "score").unwrap_or_default(),
        note: optional_string(value, "note")
            .or_else(|| optional_string(value, "feedback"))
            .or_else(|| optional_string(value, "text"))
            .unwrap_or_default(),
    }
}

fn reviewer_card_from_value(card: &Value, card_comments: &Value) -> ReviewerCard {
    let change_index = card
        .get("changeIndex")
        .and_then(Value::as_i64)
        .or_else(|| card.get("change").and_then(Value::as_i64))
        .unwrap_or_default();
    let id = card
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| change_index.to_string());
    let comment = card_comments
        .get(&id)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let raw = |key: &str| card.get(key).and_then(Value::as_str).map(str::to_string);
    let evidence_detail = card.get("evidenceDetail").and_then(|detail| {
        let text = |key: &str| detail.get(key).and_then(Value::as_str).map(str::to_string);
        match detail.get("kind").and_then(Value::as_str)? {
            "text-diff" => Some(EvidenceDetail::TextDiff {
                before: text("before").unwrap_or_default(),
                after: text("after").unwrap_or_default(),
                inline_diff: text("inlineDiff").filter(|diff| !diff.trim().is_empty()),
            }),
            "raw" => Some(EvidenceDetail::Raw(text("text")?)),
            _ => None,
        }
    });
    ReviewerCard {
        change_index,
        kind: optional_string(card, "type")
            .or_else(|| optional_string(card, "kind"))
            .unwrap_or_default(),
        summary: optional_string(card, "summary")
            .or_else(|| optional_string(card, "description"))
            .unwrap_or_default(),
        evidence: raw("evidence")
            .or_else(|| raw("description"))
            .unwrap_or_default(),
        evidence_detail,
        categories: string_array(card.get("categories")),
        matched_template_id: optional_string(card, "matchedTemplateId")
            .or_else(|| optional_string(card, "templateId")),
        template_title: optional_string(card, "templateTitle"),
        template_description: optional_string(card, "templateDescription"),
        initial_matched_template_id: optional_string(card, "initialMatchedTemplateId"),
        initial_template_title: optional_string(card, "initialTemplateTitle"),
        match_source: optional_string(card, "matchSource"),
        opinion_text: optional_string(card, "opinionText"),
        rationale: optional_string(card, "rationale"),
        comment,
        id,
    }
}

fn parse_json(payload: &str) -> Result<Value> {
    serde_json::from_str::<Value>(payload).context("The server sent an unreadable reply.")
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    optional_string(value, key).ok_or_else(|| anyhow!("The server's reply has no {key}."))
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(value_to_compact_string)
}

/// Trimmed non-empty strings, numbers and bools as text.
pub(super) fn value_to_compact_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(value_to_compact_string).collect())
        .unwrap_or_default()
}
