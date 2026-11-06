//! Distribution composition, not editor behavior. A no-default-features build contains the
//! same shell without any first-party AI, checking, format or platform implementation.

use gpui_kit::App;
#[cfg(any(feature = "formats", feature = "basic-checks", feature = "babel"))]
use gpui_kit::BorrowAppContext;

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    // Dependency extensions register before the modes that require them. Native activators
    // are retained by the shell so a disabled bundle can be enabled again without restart.
    #[cfg(feature = "formats")]
    sonus_shell::contributions::register_native_extension(
        sonus_formats::manifest(),
        register_formats,
        cx,
    )?;
    #[cfg(feature = "basic-checks")]
    sonus_shell::contributions::register_native_extension(
        sonus_lint_basic::manifest()?,
        register_basic_checks,
        cx,
    )?;
    #[cfg(feature = "gold")]
    sonus_shell::contributions::register_native_extension(
        sonus_gold_ui::manifest()?,
        sonus_gold_ui::register,
        cx,
    )?;
    #[cfg(feature = "babel")]
    {
        sonus_shell::contributions::register_native_extension(
            sonus_babel_lint::manifest()?,
            register_babel_checks,
            cx,
        )?;
        sonus_shell::contributions::register_native_extension(
            sonus_babel_tags::manifest()?,
            register_babel_tags,
            cx,
        )?;
        sonus_shell::contributions::register_native_extension(
            sonus_babel_ui::manifest()?,
            sonus_babel_ui::register,
            cx,
        )?;
    }
    #[cfg(feature = "local")]
    sonus_shell::contributions::register_native_extension(
        sonus_local::manifest()?,
        sonus_local::register,
        cx,
    )?;
    let _ = cx; // The intentionally extension-free build still runs the editor shell.
    Ok(())
}

#[cfg(feature = "formats")]
fn register_formats(cx: &mut App) -> anyhow::Result<()> {
    cx.update_global::<sonus_shell::ContributionRegistry, _>(|registry, _| {
        sonus_formats::register(&mut registry.formats).map_err(anyhow::Error::msg)
    })
}

#[cfg(feature = "basic-checks")]
fn register_basic_checks(cx: &mut App) -> anyhow::Result<()> {
    cx.update_global::<sonus_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            sonus_lint_basic::ID,
            "sonus.lint-basic.checks",
            vec!["sonus.local".into()],
            sonus_lint_basic::providers(),
        )
    })
}

#[cfg(feature = "babel")]
fn register_babel_checks(cx: &mut App) -> anyhow::Result<()> {
    let mut providers = sonus_extension::document::DocumentProviders::default();
    sonus_babel_lint::register(&mut providers);
    cx.update_global::<sonus_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            sonus_babel_lint::ID,
            "sonus.babel-lint.checks",
            vec![
                sonus_babel_ui::ID.into(),
                sonus_babel_ui::ERRORS_ID.into(),
                sonus_babel_ui::FILE_ID.into(),
            ],
            providers,
        )
    })
}

#[cfg(feature = "babel")]
fn register_babel_tags(cx: &mut App) -> anyhow::Result<()> {
    let mut providers = sonus_extension::document::DocumentProviders::default();
    sonus_babel_tags::register(&mut providers);
    cx.update_global::<sonus_shell::ContributionRegistry, _>(|registry, _| {
        registry.register_document_providers(
            sonus_babel_tags::ID,
            "sonus.babel-tags.editing",
            vec![
                sonus_babel_ui::ID.into(),
                sonus_babel_ui::ERRORS_ID.into(),
                sonus_babel_ui::FILE_ID.into(),
            ],
            providers,
        )
    })
}
