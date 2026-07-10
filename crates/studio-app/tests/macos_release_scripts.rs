#[test]
fn macos_alpha_release_scripts_publish_only_atomic_current_payloads() {
    let package_script = include_str!("../../../tools/package-alpha-macos.sh");
    let release_doc = include_str!("../../../docs/release/macos-alpha.md");
    let workflow = include_str!("../../../.github/workflows/macos-alpha.yml");
    let tool_manifest = include_str!("../../../.config/dotnet-tools.json");

    assert!(!package_script.contains("vpk bundle"));
    assert!(package_script.contains("run_vpk pack"));
    assert!(package_script.contains("--bundleId"));
    assert!(package_script.contains("publish-manifest.json"));
    assert!(package_script.contains("RelativeFileName"));
    assert!(package_script.contains("PUBLISH_DIR"));
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
    assert!(workflow.contains("STUDIO_BUILD_GIT_COMMIT: ${{ github.sha }}"));
    assert!(workflow.contains("Swatinem/rust-cache@v2"));
    assert!(workflow.contains("shared-key: alpha-${{ matrix.runtime }}"));
    assert!(workflow.contains("dotnet tool restore"));
    assert!(workflow.contains("rm -rf target/velopack/Releases target/velopack/Publish"));
    assert!(workflow.contains("remote_base_path"));
    assert!(workflow.contains("./tools/package-alpha-macos.sh"));
    assert!(workflow.contains("APPLE_DEVELOPER_ID_APPLICATION"));
    assert!(workflow.contains("APPLE_NOTARY_PROFILE"));
    assert!(workflow.contains("scp \"${scp_args[@]}\" -r \"$PUBLISH_DIR\""));
    assert!(workflow.contains("flock 9"));
    assert!(workflow.contains("Remote live and incoming paths must not overlap"));
    assert!(workflow.contains("Refusing to replace existing versioned package"));
    assert!(workflow.contains("path: target/velopack/Publish/"));
    assert!(!workflow.contains("path: target/velopack/Releases/"));
    assert!(workflow.contains("compression-level: 0"));
    assert!(workflow.contains("retention-days: 3"));
    let publish_step = workflow.find("- name: Publish artifacts to VPS").unwrap();
    let artifact_step = workflow
        .find("- name: Upload Velopack publish payload")
        .unwrap();
    assert!(publish_step < artifact_step);

    assert!(tool_manifest.contains("\"version\": \"1.2.0\""));
    assert!(release_doc.contains("Open Anyway"));
    assert!(release_doc.contains("STUDIO_UPDATE_FEED_URL"));
    assert!(release_doc.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(release_doc.contains("mac-sonoma-x64"));
    assert!(release_doc.contains("releases.alpha.json"));
}
