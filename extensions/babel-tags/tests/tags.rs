use exaudio_babel_lint::project_lint_tag_config::{
    BabelProjectLintTagConfig, META_BABEL_PROJECT_SLUG, resolve_babel_project_lint_tag_config,
};
use exaudio_babel_tags::tags::{TagSuggestion, TagSuggestionKind, suggestions_for_text};
use studio_core::Project;

fn default_config() -> BabelProjectLintTagConfig {
    BabelProjectLintTagConfig::default()
}
fn find_suggestion(suggestions: Vec<TagSuggestion>, label: &str) -> TagSuggestion {
    suggestions
        .into_iter()
        .find(|suggestion| suggestion.label == label)
        .expect("offered tag")
}
fn labels(suggestions: &[TagSuggestion]) -> Vec<&str> {
    suggestions
        .iter()
        .map(|suggestion| suggestion.label.as_str())
        .collect()
}
fn config_for_project_slug(slug: &str) -> BabelProjectLintTagConfig {
    let project = Project {
        id: "tags".into(),
        name: "Tags".into(),
        speakers: Vec::new(),
        lanes: Vec::new(),
        segments: Vec::new(),
        metadata: [(META_BABEL_PROJECT_SLUG.into(), slug.into())].into(),
    };
    resolve_babel_project_lint_tag_config(&project)
}
#[test]
fn tag_suggestions_match_babel_square_bracket_trigger() {
    let text = "это [не";
    let suggestion = find_suggestion(
        suggestions_for_text(text, text.len(), &default_config()),
        "неразборчиво",
    );

    assert_eq!(suggestion.kind, TagSuggestionKind::Bracket);
    assert_eq!(suggestion.insert_text, "[неразборчиво]");
    assert_eq!(suggestion.replacement_start, "это ".len());
    assert_eq!(suggestion.replacement_end, text.len());
}

#[test]
fn tag_suggestions_match_babel_style_open_and_close_triggers() {
    let open_text = "говорит <нара";
    let open = find_suggestion(
        suggestions_for_text(open_text, open_text.len(), &default_config()),
        "<речь-нараспев>",
    );
    assert_eq!(open.kind, TagSuggestionKind::StyleOpen);
    assert_eq!(open.insert_text, "<речь-нараспев></речь-нараспев>");
    assert_eq!(open.cursor_back_count, "речь-нараспев>".chars().count() + 2);

    let close_text = "говорит </смех";
    let close = find_suggestion(
        suggestions_for_text(close_text, close_text.len(), &default_config()),
        "</смех-в-речи>",
    );
    assert_eq!(close.kind, TagSuggestionKind::StyleClose);
    assert_eq!(close.insert_text, "</смех-в-речи>");
    assert_eq!(close.cursor_back_count, 0);
}

#[test]
fn tag_suggestions_match_babel_curly_mis_and_pro_triggers() {
    let mis_text = "слово {M";
    let mis = find_suggestion(
        suggestions_for_text(mis_text, mis_text.len(), &default_config()),
        "MIS:",
    );
    assert_eq!(mis.kind, TagSuggestionKind::Curly);
    assert_eq!(mis.insert_text, "{MIS: }");
    assert_eq!(mis.replacement_start, "слово ".len());
    assert_eq!(mis.replacement_end, mis_text.len());
    assert_eq!(mis.cursor_back_count, 1);

    let pro_text = "слово {pro";
    let pro = find_suggestion(
        suggestions_for_text(pro_text, pro_text.len(), &default_config()),
        "PRO:",
    );
    assert_eq!(pro.kind, TagSuggestionKind::Curly);
    assert_eq!(pro.insert_text, "{PRO: }");
}

#[test]
fn tag_suggestions_ignore_invalid_non_suffix_partial() {
    let text = "hello [не word";
    assert!(suggestions_for_text(text, text.len(), &default_config()).is_empty());
}

#[test]
fn tag_suggestions_use_contains_matching_like_babel() {
    let suggestions = suggestions_for_text("[раз", "[раз".len(), &default_config());
    assert!(suggestions.iter().any(|suggestion| {
        suggestion.kind == TagSuggestionKind::Bracket && suggestion.label == "неразборчиво"
    }));
}

#[test]
fn tag_suggestions_offer_gold_non_background_tags_for_any_russian_project() {
    // ru-tx-gold: its background-noise tags are not offered, Gold's non-background ones are.
    let config = config_for_project_slug("RU-tx-gold");

    let background = suggestions_for_text("[муз", "[муз".len(), &config);
    let other_noise = suggestions_for_text("[друг", "[друг".len(), &config);
    let style = suggestions_for_text("<под", "<под".len(), &config);

    assert!(background.is_empty());
    assert_eq!(labels(&other_noise), vec!["другой-звук"]);
    assert_eq!(other_noise[0].kind, TagSuggestionKind::Bracket);
    assert_eq!(labels(&style), vec!["<подражание>"]);
    assert_eq!(style[0].kind, TagSuggestionKind::StyleOpen);
}
