use std::hash::{Hash, Hasher};

use iced::widget::{button, container, lazy, text, tooltip};
use iced::{Alignment, Element, Length};
use studio_app::{LintSeverity, TranscriptRowView};

use super::super::super::Message;
use super::super::super::styles::{dot_style, lint_color, table_plain_button_style};

struct LintCellDependency {
    segment_id: String,
    severity: LintSeverity,
    message: String,
    can_toggle_assertion: bool,
}

impl std::hash::Hash for LintCellDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.segment_id.hash(state);
        hash_lint_severity(&self.severity, state);
        self.message.hash(state);
        self.can_toggle_assertion.hash(state);
    }
}

pub(super) fn view_lint(
    row_view: &TranscriptRowView,
    severity: LintSeverity,
    message: String,
) -> Element<'static, Message> {
    let can_toggle_assertion = (row_view.lint.is_assertable_warning()
        || row_view.has_asserted_warning)
        && row_view.lint.message == message;

    view_lazy_lint_cell(LintCellDependency {
        segment_id: row_view.segment_id.clone(),
        severity,
        message,
        can_toggle_assertion,
    })
}

fn view_lazy_lint_cell(dependency: LintCellDependency) -> Element<'static, Message> {
    lazy(dependency, view_lint_cell_content).into()
}

fn view_lint_cell_content(dependency: &LintCellDependency) -> Element<'static, Message> {
    let color = lint_color(&dependency.severity);
    let dot = view_lint_dot(color);
    let dot: Element<'static, Message> = if dependency.can_toggle_assertion {
        button(dot)
            .on_press(Message::ToggleWarningAssertion(
                dependency.segment_id.clone(),
            ))
            .style(table_plain_button_style)
            .padding(0)
            .into()
    } else {
        dot
    };

    tooltip(
        container(dot)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .width(Length::Fixed(super::LINTER_COLUMN_WIDTH))
            .height(Length::Fixed(20.0)),
        container(text(dependency.message.clone()).size(12)).padding(6),
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}

fn hash_lint_severity<H: Hasher>(severity: &LintSeverity, state: &mut H) {
    let key = match severity {
        LintSeverity::Pass => 1_u8,
        LintSeverity::Warning => 2,
        LintSeverity::Error => 3,
    };
    key.hash(state);
}

fn view_lint_dot(color: iced::Color) -> Element<'static, Message> {
    container(text(""))
        .width(Length::Fixed(12.0))
        .height(Length::Fixed(12.0))
        .style(move |_| dot_style(color))
        .into()
}
