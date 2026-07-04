#[test]
fn macos_alpha_release_scripts_use_velopack_and_optional_signing() {
    let package_script = include_str!("../../../tools/package-alpha-macos.sh");
    let release_doc = include_str!("../../../docs/release/macos-alpha.md");
    let workflow = include_str!("../../../.github/workflows/macos-alpha.yml");

    assert!(package_script.contains("vpk bundle"));
    assert!(package_script.contains("vpk pack"));
    assert!(package_script.contains("--bundleId"));
    assert!(package_script.contains("MACOSX_DEPLOYMENT_TARGET"));
    assert!(package_script.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(package_script.contains("Developer ID Application"));
    assert!(package_script.contains("notarytool"));
    assert!(workflow.contains("macos-26"));
    assert!(workflow.contains("macos-26-intel"));
    assert!(workflow.contains("macos-14-large"));
    assert!(workflow.contains("latest-arm64"));
    assert!(workflow.contains("latest-amd64"));
    assert!(workflow.contains("sonoma-amd64"));
    assert!(workflow.contains("mac-sonoma-x64"));
    assert!(workflow.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(workflow.contains("remote_base_path"));
    assert!(workflow.contains("./tools/package-alpha-macos.sh"));
    assert!(workflow.contains("APPLE_DEVELOPER_ID_APPLICATION"));
    assert!(workflow.contains("APPLE_NOTARY_PROFILE"));
    assert!(release_doc.contains("Open Anyway"));
    assert!(release_doc.contains("STUDIO_UPDATE_FEED_URL"));
    assert!(release_doc.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(release_doc.contains("mac-sonoma-x64"));
    assert!(release_doc.contains("releases.alpha.json"));
}
