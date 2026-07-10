use iced::widget::{button, container, slider, text_editor, text_input};
use std::sync::LazyLock;

use iced::{Background, Color, Shadow, Theme, Vector, border, theme::Palette};
use studio_app::LintSeverity;
const PERCENT_PORTION_SCALE: f32 = 100.0;

static STUDIO_THEME: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "Transcription Studio".to_string(),
        Palette {
            background: Color::from_rgb8(248, 250, 252),
            text: Color::from_rgb8(15, 23, 42),
            primary: Color::from_rgb8(15, 23, 42),
            success: Color::from_rgb8(5, 150, 105),
            danger: Color::from_rgb8(220, 38, 38),
        },
    )
});

pub(crate) fn studio_theme() -> Theme {
    STUDIO_THEME.clone()
}

fn surface_shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(15, 23, 42, 0.07),
        offset: Vector::new(0.0, 1.0),
        blur_radius: 8.0,
    }
}

pub(crate) fn portion(percent: f32) -> u16 {
    if !percent.is_finite() || percent <= 0.0 {
        return 0;
    }

    (percent * PERCENT_PORTION_SCALE)
        .round()
        .clamp(1.0, 10_000.0) as u16
}

pub(crate) fn color_for_token(token: &str) -> Color {
    match token {
        "purple" => Color::from_rgb8(176, 131, 255),
        _ => Color::from_rgb8(100, 181, 246),
    }
}

pub(crate) fn region_border_color_for_token(token: &str) -> Color {
    match token {
        "purple" => Color::from_rgb8(176, 131, 255),
        _ => Color::from_rgb8(49, 160, 252),
    }
}

fn region_tooltip_background_for_token(token: &str) -> Color {
    match token {
        "purple" => Color::from_rgb8(248, 244, 255),
        _ => Color::from_rgb8(242, 248, 255),
    }
}

fn region_tooltip_border_color_for_token(token: &str) -> Color {
    let color = match token {
        "purple" => Color::from_rgb8(176, 131, 255),
        _ => Color::from_rgb8(100, 181, 246),
    };
    Color { a: 0.50, ..color }
}

pub(crate) fn lint_color(severity: &LintSeverity) -> Color {
    match severity {
        LintSeverity::Pass => Color::from_rgb8(28, 190, 105),
        LintSeverity::Warning => Color::from_rgb8(220, 155, 30),
        LintSeverity::Error => Color::from_rgb8(218, 67, 74),
    }
}

pub(crate) fn app_background_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(248, 250, 252))
        .color(Color::from_rgb8(15, 23, 42))
}

pub(crate) fn panel_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::WHITE)
        .border(
            border::rounded(12)
                .width(1)
                .color(Color::from_rgb8(226, 232, 240)),
        )
        .shadow(surface_shadow())
}

pub(crate) fn editor_band_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE)
}

pub(crate) fn babel_editor_card_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(8)
            .width(1)
            .color(Color::from_rgb8(226, 232, 240)),
    )
}

pub(crate) fn babel_transcript_card_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(8)
            .width(1)
            .color(Color::from_rgb8(226, 232, 240)),
    )
}

pub(crate) fn subtle_panel_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(8)
            .width(1)
            .color(Color::from_rgb8(226, 232, 240)),
    )
}

pub(crate) fn modal_scrim_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color {
        a: 0.48,
        ..Color::from_rgb8(15, 23, 42)
    })
}

pub(crate) fn modal_surface_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::WHITE)
        .border(
            border::rounded(12)
                .width(1)
                .color(Color::from_rgb8(226, 232, 240)),
        )
        .shadow(Shadow {
            color: Color::from_rgba8(15, 23, 42, 0.18),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 28.0,
        })
}

pub(crate) fn dashboard_sidebar_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(15, 23, 42))
        .color(Color::from_rgb8(248, 250, 252))
}

pub(crate) fn brand_mark_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(51, 65, 85))
        .color(Color::WHITE)
        .border(border::rounded(9))
}

pub(crate) fn dashboard_header_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgba8(255, 255, 255, 0.96))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(226, 232, 240)),
        )
}

pub(crate) fn project_card_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::WHITE)
        .border(
            border::rounded(10)
                .width(1)
                .color(Color::from_rgb8(226, 232, 240)),
        )
        .shadow(Shadow {
            color: Color::from_rgba8(15, 23, 42, 0.05),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 5.0,
        })
}

pub(crate) fn status_bar_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(248, 250, 252))
        .color(Color::from_rgb8(71, 85, 105))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(226, 232, 240)),
        )
}

pub(crate) fn success_pill_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(236, 253, 245))
        .color(Color::from_rgb8(6, 95, 70))
        .border(
            border::rounded(999)
                .width(1)
                .color(Color::from_rgb8(167, 243, 208)),
        )
}

pub(crate) fn success_banner_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(236, 253, 245))
        .color(Color::from_rgb8(6, 95, 70))
        .border(
            border::rounded(8)
                .width(1)
                .color(Color::from_rgb8(167, 243, 208)),
        )
}

pub(crate) fn warning_pill_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(255, 251, 235))
        .color(Color::from_rgb8(146, 64, 14))
        .border(
            border::rounded(999)
                .width(1)
                .color(Color::from_rgb8(253, 230, 138)),
        )
}

pub(crate) fn warning_banner_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(255, 251, 235))
        .color(Color::from_rgb8(120, 53, 15))
        .border(
            border::rounded(8)
                .width(1)
                .color(Color::from_rgb8(253, 230, 138)),
        )
}

pub(crate) fn error_pill_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(254, 242, 242))
        .color(Color::from_rgb8(153, 27, 27))
        .border(
            border::rounded(999)
                .width(1)
                .color(Color::from_rgb8(254, 202, 202)),
        )
}

pub(crate) fn primary_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Active => Color::from_rgb8(15, 23, 42),
        button::Status::Hovered => Color::from_rgb8(30, 41, 59),
        button::Status::Pressed => Color::from_rgb8(51, 65, 85),
        button::Status::Disabled => Color::from_rgb8(226, 232, 240),
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(148, 163, 184),
        _ => Color::WHITE,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(7).width(1).color(background),
        shadow: Default::default(),
    }
}

pub(crate) fn secondary_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(248, 250, 252),
        button::Status::Pressed => Color::from_rgb8(241, 245, 249),
        button::Status::Disabled => Color::from_rgb8(248, 250, 252),
        button::Status::Active => Color::WHITE,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(148, 163, 184),
        _ => Color::from_rgb8(51, 65, 85),
    };
    let border_color = match status {
        button::Status::Disabled => Color::from_rgb8(226, 232, 240),
        _ => Color::from_rgb8(203, 213, 225),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(7).width(1).color(border_color),
        shadow: Default::default(),
    }
}

pub(crate) fn dashboard_nav_button_style(
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    move |_theme, status| {
        let background = match (selected, status) {
            (true, _) => Some(Color::from_rgb8(30, 41, 59)),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(Color::from_rgb8(30, 41, 59))
            }
            _ => None,
        };
        let text_color = if selected {
            Color::WHITE
        } else {
            Color::from_rgb8(203, 213, 225)
        };

        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: border::rounded(7).width(0).color(Color::TRANSPARENT),
            shadow: Default::default(),
        }
    }
}

pub(crate) fn diff_switch_style(is_checked: bool) -> container::Style {
    let background = if is_checked {
        Color::from_rgb8(35, 137, 221)
    } else {
        Color::from_rgb8(229, 231, 235)
    };
    container::Style::default()
        .background(background)
        .border(border::rounded(999))
}

pub(crate) fn diff_switch_knob_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::WHITE)
        .border(border::rounded(999))
}

pub(crate) fn kbd_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(244, 246, 250))
        .border(
            border::rounded(4)
                .width(1)
                .color(Color::from_rgb8(218, 224, 234)),
        )
}

pub(crate) fn outline_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(238, 241, 246),
        button::Status::Pressed => Color::from_rgb8(226, 231, 238),
        button::Status::Disabled => Color::from_rgb8(245, 247, 250),
        button::Status::Active => Color::WHITE,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(156, 163, 175),
        _ => Color::from_rgb8(73, 80, 93),
    };
    let border_color = match status {
        button::Status::Disabled => Color::from_rgb8(229, 233, 240),
        _ => Color::from_rgb8(210, 216, 226),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(4).width(1).color(border_color),
        shadow: Default::default(),
    }
}

pub(crate) fn table_plain_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Some(Color::from_rgb8(245, 245, 245)),
        button::Status::Pressed => Some(Color::from_rgb8(238, 238, 238)),
        button::Status::Active | button::Status::Disabled => None,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(156, 163, 175),
        _ => Color::from_rgb8(33, 37, 41),
    };

    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: border::rounded(2).width(0).color(Color::TRANSPARENT),
        shadow: Default::default(),
    }
}

pub(crate) fn tag_suggestion_button_style(
    highlighted: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + Copy {
    move |_theme, status| {
        let background = match (highlighted, status) {
            (_, button::Status::Pressed) => Some(Color::from_rgb8(219, 234, 254)),
            (true, _) => Some(Color::from_rgb8(239, 246, 255)),
            (false, button::Status::Hovered) => Some(Color::from_rgb8(245, 245, 245)),
            (false, _) => None,
        };

        button::Style {
            background: background.map(Background::Color),
            text_color: Color::from_rgb8(33, 37, 41),
            border: border::rounded(2).width(0).color(Color::TRANSPARENT),
            shadow: Default::default(),
        }
    }
}

pub(crate) fn action_menu_item_button_style(
    _theme: &Theme,
    status: button::Status,
) -> button::Style {
    let background = match status {
        button::Status::Hovered => Some(Color::from_rgb8(245, 245, 245)),
        button::Status::Pressed => Some(Color::from_rgb8(238, 238, 238)),
        button::Status::Active | button::Status::Disabled => None,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(163, 163, 163),
        _ => Color::from_rgb8(82, 82, 82),
    };

    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: border::rounded(2).width(0).color(Color::TRANSPARENT),
        shadow: Default::default(),
    }
}

pub(crate) fn action_menu_delete_button_style(
    _theme: &Theme,
    status: button::Status,
) -> button::Style {
    let background = match status {
        button::Status::Hovered => Some(Color::from_rgb8(254, 242, 242)),
        button::Status::Pressed => Some(Color::from_rgb8(254, 226, 226)),
        button::Status::Active | button::Status::Disabled => None,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(252, 165, 165),
        _ => Color::from_rgb8(220, 38, 38),
    };

    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: border::rounded(2).width(0).color(Color::TRANSPARENT),
        shadow: Default::default(),
    }
}

pub(crate) fn low_confidence_ack_button_style(
    _theme: &Theme,
    status: button::Status,
) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(217, 119, 6),
        button::Status::Pressed => Color::from_rgb8(180, 83, 9),
        button::Status::Disabled => Color::from_rgb8(245, 158, 11),
        button::Status::Active => Color::from_rgb8(245, 158, 11),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: border::rounded(4).width(0).color(Color::TRANSPARENT),
        shadow: Default::default(),
    }
}

pub(crate) fn table_plain_text_input_style(
    _theme: &Theme,
    _status: text_input::Status,
) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::WHITE),
        border: border::rounded(0).width(0).color(Color::TRANSPARENT),
        icon: Color::from_rgb8(33, 37, 41),
        placeholder: Color::from_rgb8(156, 163, 175),
        value: Color::from_rgb8(33, 37, 41),
        selection: Color::from_rgb8(191, 219, 254),
    }
}

pub(crate) fn table_plain_text_editor_style(
    _theme: &Theme,
    status: text_editor::Status,
) -> text_editor::Style {
    let border_color = match status {
        text_editor::Status::Hovered | text_editor::Status::Focused => {
            Color::from_rgb8(209, 213, 219)
        }
        text_editor::Status::Disabled | text_editor::Status::Active => Color::TRANSPARENT,
    };

    text_editor::Style {
        background: Background::Color(Color::WHITE),
        border: border::rounded(3).width(1).color(border_color),
        icon: Color::from_rgb8(33, 37, 41),
        placeholder: Color::from_rgb8(156, 163, 175),
        value: Color::from_rgb8(33, 37, 41),
        selection: Color::from_rgb8(191, 219, 254),
    }
}

pub(crate) fn zoom_slider_style(_theme: &Theme, status: slider::Status) -> slider::Style {
    let active = match status {
        slider::Status::Active => Color::from_rgb8(124, 124, 124),
        slider::Status::Hovered => Color::from_rgb8(105, 105, 105),
        slider::Status::Dragged => Color::from_rgb8(86, 86, 86),
    };

    slider::Style {
        rail: slider::Rail {
            backgrounds: (active.into(), Color::from_rgb8(196, 196, 196).into()),
            width: 4.0,
            border: border::rounded(2),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 7.0 },
            background: active.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub(crate) fn lane_label_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(253, 253, 254))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(224, 229, 236)),
        )
}

pub(crate) fn lane_vertical_zoom_slider_rail_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(253, 253, 254))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(224, 229, 236)),
        )
}

pub(crate) fn lane_vertical_zoom_slider_thumb_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(116, 116, 116))
        .border(border::rounded(99))
}

pub(crate) fn waveform_backdrop_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(0)
            .width(1)
            .color(Color::from_rgb8(224, 229, 236)),
    )
}

pub(crate) fn loop_strip_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(252, 253, 255))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(224, 229, 236)),
        )
}

pub(crate) fn loop_range_style(is_enabled: bool) -> container::Style {
    let alpha = if is_enabled { 0.22 } else { 0.08 };
    let border_color = if is_enabled {
        Color::from_rgb8(217, 119, 6)
    } else {
        Color {
            a: 0.35,
            ..Color::from_rgb8(217, 119, 6)
        }
    };
    container::Style::default()
        .background(Color {
            a: alpha,
            ..Color::from_rgb8(245, 158, 11)
        })
        .border(border::rounded(2).width(1).color(border_color))
}

pub(crate) fn loop_handle_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(217, 119, 6))
}

pub(crate) fn overlay_range_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color {
        a: 0.34,
        ..Color::from_rgb8(100, 181, 246)
    })
}

pub(crate) fn transcript_row_style(
    is_selected: bool,
    is_playing: bool,
    is_hovered: bool,
    is_find_match: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let background = if is_selected && is_find_match {
            Color::from_rgb8(254, 243, 199)
        } else if is_find_match {
            Color::from_rgb8(255, 251, 235)
        } else if is_selected && is_playing {
            Color::from_rgb8(240, 253, 244)
        } else if is_selected {
            Color::from_rgb8(245, 245, 245)
        } else if is_hovered {
            Color::from_rgb8(250, 250, 250)
        } else {
            Color::WHITE
        };
        let border_color = if is_selected && is_find_match {
            Color::from_rgb8(245, 158, 11)
        } else if is_find_match {
            Color::from_rgb8(252, 211, 77)
        } else if is_selected && is_playing {
            Color::from_rgb8(134, 239, 172)
        } else if is_selected {
            Color::from_rgb8(212, 212, 212)
        } else {
            Color::from_rgb8(234, 237, 243)
        };

        container::Style::default()
            .background(background)
            .border(border::rounded(0).width(1).color(border_color))
    }
}

pub(crate) fn transcript_modified_row_style(is_added: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let background = if is_added {
            Color::from_rgb8(240, 253, 244)
        } else {
            Color::from_rgb8(254, 242, 242)
        };
        let border_color = if is_added {
            Color::from_rgb8(134, 239, 172)
        } else {
            Color::from_rgb8(252, 165, 165)
        };

        container::Style::default()
            .background(background)
            .border(border::rounded(0).width(1).color(border_color))
    }
}

pub(crate) fn diff_change_badge_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(254, 252, 232))
        .color(Color::from_rgb8(113, 63, 18))
        .border(
            border::rounded(3)
                .width(1)
                .color(Color::from_rgb8(253, 224, 71)),
        )
}

pub(crate) fn diff_no_change_badge_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(240, 253, 244))
        .color(Color::from_rgb8(22, 101, 52))
        .border(
            border::rounded(3)
                .width(1)
                .color(Color::from_rgb8(187, 247, 208)),
        )
}

pub(crate) fn diff_new_badge_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(219, 234, 254))
        .color(Color::from_rgb8(30, 64, 175))
        .border(
            border::rounded(3)
                .width(1)
                .color(Color::from_rgb8(147, 197, 253)),
        )
}

pub(crate) fn diff_added_badge_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(240, 253, 244))
        .color(Color::from_rgb8(22, 101, 52))
        .border(
            border::rounded(3)
                .width(1)
                .color(Color::from_rgb8(187, 247, 208)),
        )
}

pub(crate) fn diff_deleted_badge_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(254, 242, 242))
        .color(Color::from_rgb8(185, 28, 28))
        .border(
            border::rounded(3)
                .width(1)
                .color(Color::from_rgb8(254, 202, 202)),
        )
}

pub(crate) fn diff_word_added_chip_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(220, 252, 231))
        .border(border::rounded(3))
}

pub(crate) fn diff_word_removed_chip_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(254, 226, 226))
        .border(border::rounded(3))
}

pub(crate) fn low_confidence_text_shell_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::WHITE)
        .border(border::rounded(3).width(1).color(Color::TRANSPARENT))
}

pub(crate) fn low_confidence_word_dotted_style(_theme: &Theme) -> container::Style {
    container::Style::default().border(
        border::rounded(1)
            .width(2)
            .color(Color::from_rgb8(245, 158, 11)),
    )
}

pub(crate) fn action_menu_popup_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(2)
            .width(1)
            .color(Color::from_rgba8(115, 115, 115, 0.20)),
    )
}

pub(crate) fn action_menu_separator_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(229, 229, 229))
}

pub(crate) fn waveform_region_timestamp_label_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color {
            a: 0.90,
            ..Color::from_rgb8(15, 23, 42)
        })
        .color(Color::from_rgb8(248, 250, 252))
        .border(border::rounded(2))
}

pub(crate) fn waveform_magnifier_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color {
            a: 0.96,
            ..Color::WHITE
        })
        .color(Color::from_rgb8(15, 23, 42))
        .border(
            border::rounded(4)
                .width(1)
                .color(Color::from_rgba8(100, 116, 139, 0.45)),
        )
}

pub(crate) fn waveform_magnifier_center_line_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(37, 99, 235))
}

pub(crate) fn waveform_region_label_edit_tooltip_style(token: &str) -> container::Style {
    container::Style::default()
        .background(region_tooltip_background_for_token(token))
        .color(Color::BLACK)
        .border(
            border::rounded(6)
                .width(1)
                .color(region_tooltip_border_color_for_token(token)),
        )
}

pub(crate) fn hover_cursor_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(36, 42, 54))
}

pub(crate) fn hover_label_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(36, 42, 54))
        .color(Color::WHITE)
        .border(border::rounded(2))
}

pub(crate) fn dot_style(color: Color) -> container::Style {
    container::Style::default()
        .background(color)
        .border(border::rounded(20))
}

#[cfg(test)]
mod tests {
    use iced::widget::button;
    use iced::{Background, Color, Theme};

    use super::{outline_button_style, portion, table_plain_button_style, transcript_row_style};

    #[test]
    fn portions_preserve_centipercent_steps_and_allow_zero_width() {
        assert_eq!(portion(0.0), 0);
        assert_eq!(portion(0.25), 25);
        assert_eq!(portion(0.26), 26);
    }

    #[test]
    fn table_plain_buttons_keep_browser_like_hover_pressed_and_disabled_feedback() {
        let active = table_plain_button_style(&Theme::Light, button::Status::Active);
        let hovered = table_plain_button_style(&Theme::Light, button::Status::Hovered);
        let pressed = table_plain_button_style(&Theme::Light, button::Status::Pressed);
        let disabled = table_plain_button_style(&Theme::Light, button::Status::Disabled);

        assert_eq!(background_color(active), None);
        assert_ne!(background_color(hovered), None);
        assert_ne!(background_color(pressed), background_color(hovered));
        assert_eq!(background_color(disabled), None);
        assert_ne!(disabled.text_color, active.text_color);
    }

    #[test]
    fn outline_buttons_expose_distinct_active_hover_pressed_and_disabled_states() {
        let active = outline_button_style(&Theme::Light, button::Status::Active);
        let hovered = outline_button_style(&Theme::Light, button::Status::Hovered);
        let pressed = outline_button_style(&Theme::Light, button::Status::Pressed);
        let disabled = outline_button_style(&Theme::Light, button::Status::Disabled);

        assert_ne!(background_color(active), background_color(hovered));
        assert_ne!(background_color(hovered), background_color(pressed));
        assert_ne!(active.text_color, disabled.text_color);
        assert_ne!(active.border.color, disabled.border.color);
    }

    #[test]
    fn transcript_rows_expose_distinct_browser_find_match_highlight() {
        let plain = transcript_row_style(false, false, false, false)(&Theme::Light);
        let hovered = transcript_row_style(false, false, true, false)(&Theme::Light);
        let selected = transcript_row_style(true, false, false, false)(&Theme::Light);
        let playing = transcript_row_style(true, true, false, false)(&Theme::Light);
        let find_match = transcript_row_style(false, false, false, true)(&Theme::Light);
        let selected_find_match = transcript_row_style(true, false, false, true)(&Theme::Light);

        assert_ne!(
            container_background_color(&find_match),
            container_background_color(&plain)
        );
        assert_ne!(
            container_background_color(&find_match),
            container_background_color(&hovered)
        );
        assert_ne!(
            container_background_color(&selected_find_match),
            container_background_color(&selected)
        );
        assert_ne!(selected_find_match.border.color, selected.border.color);
        assert_ne!(
            container_background_color(&selected_find_match),
            container_background_color(&playing)
        );
    }

    fn background_color(style: button::Style) -> Option<Color> {
        match style.background {
            Some(Background::Color(color)) => Some(color),
            _ => None,
        }
    }

    fn container_background_color(style: &iced::widget::container::Style) -> Option<Color> {
        match style.background {
            Some(Background::Color(color)) => Some(color),
            _ => None,
        }
    }
}
