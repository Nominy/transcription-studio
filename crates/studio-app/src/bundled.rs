//! Distribution composition, not editor behavior. A no-default-features build contains the
//! same shell without any first-party AI, checking, format or platform implementation.

use gpui_kit::App;
#[cfg(any(feature = "formats", feature = "basic-checks", feature = "babel"))]
use gpui_kit::BorrowAppContext;

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    // Dependency extensions register before the modes that require them. Native activators
    // are retained by the shell so a disabled bundle can be enabled again without restart.
    #[cfg(feature = "formats")]
    exaudio_shell::contributions::register_native_extension(
        exaudio_formats::manifest(),
        register_formats,
        cx,
    )?;
    #[cfg(feature = "basic-checks")]
    exaudio_shell::contributions::register_native_extension(
        exaudio_lint_basic::manifest()?,
        register_basic_checks,
        cx,
    )?;
    #[cfg(feature = "gold")]
    exaudio_shell::contributions::register_native_extension(
        exaudio_gold_ui::manifest()?,
        exaudio_gold_ui::register,
        cx,
    )?;
    #[cfg(feature = "babel")]
    {
        exaudio_shell::contributions::register_native_extension(
            exaudio_babel_lint::manifest()?,
            register_babel_checks,
            cx,
        )?;
        exaudio_shell::contributions::register_native_extension(
            exaudio_babel_tags::manifest()?,
            register_babel_tags,
            cx,
        )?;
        exaudio_shell::contributions::register_native_extension(
            exaudio_babel_ui::manifest()?,
            exaudio_babel_ui::register,
            cx,
        )?;
    }
    #[cfg(feature = "local")]
    exaudio_shell::contributions::register_native_extension(
        exaudio_local::manifest()?,
        exaudio_local::register,
        cx,
    )?;
    let _ = cx; // The intentionally extension-free build still runs the editor shell.
    Ok(())
}

#[cfg(feature = "formats")]
fn register_formats(cx: &mut App) -> anyhow::Result<()> {
    cx.update_global::<exaudio_shell::ContributionRegistry, _>(|registry, _| {
        exaudio_formats::register(&mut registry.formats).map_err(anyhow::Error::msg)
    })
}

#[cfg(feature = "basic-checks")]
fn register_basic_checks(cx: &mut App) -> anyhow::Result<()> {
    let language = if exaudio_shell::screens::ui::localized(cx, "en", "ru") == "en" {
        exaudio_lint_basic::Language::En
    } else {
        exaudio_lint_basic::Language::Ru
    };
    cx.update_global::<exaudio_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            exaudio_lint_basic::ID,
            "exaudio.lint-basic.checks",
            vec!["exaudio.local".into()],
            exaudio_lint_basic::providers(language),
        )
    })
}

#[cfg(feature = "babel")]
fn register_babel_checks(cx: &mut App) -> anyhow::Result<()> {
    let mut providers = exaudio_extension::document::DocumentProviders::default();
    exaudio_babel_lint::register(&mut providers);
    cx.update_global::<exaudio_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            exaudio_babel_lint::ID,
            "exaudio.babel-lint.checks",
            vec![
                exaudio_babel_ui::ID.into(),
                exaudio_babel_ui::ERRORS_ID.into(),
                exaudio_babel_ui::FILE_ID.into(),
            ],
            providers,
        )
    })
}

#[cfg(feature = "babel")]
fn register_babel_tags(cx: &mut App) -> anyhow::Result<()> {
    let mut providers = exaudio_extension::document::DocumentProviders::default();
    exaudio_babel_tags::register(&mut providers);
    cx.update_global::<exaudio_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            exaudio_babel_tags::ID,
            "exaudio.babel-tags.editing",
            vec![
                exaudio_babel_ui::ID.into(),
                exaudio_babel_ui::ERRORS_ID.into(),
                exaudio_babel_ui::FILE_ID.into(),
            ],
            providers,
        )
    })
}
