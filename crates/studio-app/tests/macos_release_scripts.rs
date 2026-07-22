#[test]
fn macos_alpha_release_scripts_publish_only_atomic_current_payloads() {
    let osxcross_script = include_str!("../../../tools/build-alpha-macos-osxcross.sh");
    let package_script = include_str!("../../../tools/package-alpha-macos.sh");
    let release_doc = include_str!("../../../docs/release/macos-alpha.md");
    let workflow = include_str!("../../../.github/workflows/macos-alpha.yml");
    let tool_manifest = include_str!("../../../.config/dotnet-tools.json");

    assert!(osxcross_script.contains("osx-arm64)\n    rust_target=\"aarch64-apple-darwin\""));
    assert!(osxcross_script.contains("osx-x64)\n    rust_target=\"x86_64-apple-darwin\""));
    assert!(osxcross_script.contains(r#"eval "$(osxcross-conf)""#));
    assert!(osxcross_script.contains(
        r#"compiler_prefix="$OSXCROSS_TARGET_DIR/bin/$osxcross_arch-apple-$OSXCROSS_TARGET""#
    ));
    assert!(osxcross_script.contains(r#"export SDKROOT="$OSXCROSS_SDK""#));
    assert!(osxcross_script.contains(r#"cc_target_env="CC_${rust_target//-/_}""#));
    assert!(osxcross_script.contains(r#"cxx_target_env="CXX_${rust_target//-/_}""#));
    assert!(osxcross_script.contains(r#"ar_target_env="AR_${rust_target//-/_}""#));
    assert!(
        osxcross_script.contains(r#"linker_target_env="CARGO_TARGET_${target_env_suffix}_LINKER""#)
    );
    assert!(osxcross_script.contains(r#"export "$cc_target_env=$cc_path""#));
    assert!(osxcross_script.contains(r#"export "$cxx_target_env=$cxx_path""#));
    assert!(osxcross_script.contains(r#"export "$ar_target_env=$ar_path""#));
    assert!(osxcross_script.contains(r#"export "$linker_target_env=$cc_path""#));
    assert!(osxcross_script.contains(r#"rustup target add "$rust_target""#));
    assert!(osxcross_script.contains(
        r#"cargo build \
  -p studio-app \
  -p studio-relay \
  --release \
  --locked \
  --target "$rust_target""#
    ));

    assert!(!package_script.contains("vpk bundle"));
    assert!(package_script.contains("run_vpk pack"));
    assert!(package_script.contains(r#"skip_build="${SKIP_BUILD:-0}""#));
    assert!(package_script.contains(r#"source_dir="${SOURCE_DIR:-$repo_root/target/release}""#));
    assert!(package_script.contains(r#"if [[ "$skip_build" != "1" && "$skip_build" != "true" ]]"#));
    assert!(
        package_script.contains(r#"cp "$source_dir/studio-app" "$stage_dir/TranscriptionStudio""#)
    );
    assert!(package_script.contains(r#"cp "$source_dir/studio-relay" "$stage_dir/studio-relay""#));
    assert!(package_script.contains("studio-relay"));
    assert!(package_script.contains("--bundleId"));
    assert!(package_script.contains("publish-manifest.json"));
    assert!(package_script.contains("RelativeFileName"));
    assert!(package_script.contains("PUBLISH_DIR"));
    assert!(package_script.contains("MACOSX_DEPLOYMENT_TARGET"));
    assert!(package_script.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(package_script.contains("Developer ID Application"));
    assert!(package_script.contains("notarytool"));

    let compile_job = workflow
        .split_once("\n  compile:\n")
        .unwrap()
        .1
        .split_once("\n  package:\n")
        .unwrap()
        .0;
    let package_job = workflow.split_once("\n  package:\n").unwrap().1;

    assert!(workflow.contains("macos-26"));
    assert!(workflow.contains("macos-26-intel"));
    assert!(workflow.contains("macos-14-large"));
    assert!(workflow.contains("latest-arm64"));
    assert!(workflow.contains("latest-amd64"));
    assert!(workflow.contains("sonoma-amd64"));
    assert!(workflow.contains("mac-sonoma-x64"));
    assert!(workflow.contains(r#""runtime":"osx-arm64","rust_target":"aarch64-apple-darwin""#));
    assert!(workflow.contains(r#""runtime":"osx-x64","rust_target":"x86_64-apple-darwin""#));
    assert!(workflow.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(workflow.contains("STUDIO_BUILD_GIT_COMMIT: ${{ github.sha }}"));
    assert!(!workflow.contains("Swatinem/rust-cache@v2"));

    assert!(compile_job.contains("runs-on: [self-hosted, Linux, homeserver]"));
    assert!(compile_job.contains("CARGO_TARGET_DIR: target/osxcross/${{ matrix.target }}"));
    assert!(compile_job.contains("./tools/build-alpha-macos-osxcross.sh"));
    assert!(compile_job.contains("uses: actions/upload-artifact@v5"));
    assert!(compile_job.contains("name: macos-binaries-${{ matrix.target }}"));
    assert!(compile_job.contains(
        "target/osxcross/${{ matrix.target }}/${{ matrix.rust_target }}/release/studio-app"
    ));
    assert!(compile_job.contains(
        "target/osxcross/${{ matrix.target }}/${{ matrix.rust_target }}/release/studio-relay"
    ));

    assert!(package_job.contains("needs: [select-targets, compile]"));
    assert!(package_job.contains("runs-on: ${{ matrix.runner }}"));
    assert!(package_job.contains(
        "SOURCE_DIR: ${{ github.workspace }}/target/osxcross/${{ matrix.target }}/${{ matrix.rust_target }}/release"
    ));
    assert!(package_job.contains("SKIP_BUILD: \"1\""));
    assert!(package_job.contains("uses: actions/download-artifact@v5"));
    assert!(package_job.contains("name: macos-binaries-${{ matrix.target }}"));
    assert!(package_job.contains("path: ${{ env.SOURCE_DIR }}"));
    assert!(!package_job.contains("Swatinem/rust-cache"));
    assert!(!package_job.contains("setup-rust"));
    assert!(!package_job.contains("rust-toolchain"));
    assert!(!package_job.contains("rustup target add"));
    assert!(!package_job.contains("cargo build"));

    assert!(package_job.contains("dotnet tool restore"));
    assert!(package_job.contains("rm -rf target/velopack/Releases target/velopack/Publish"));
    assert!(package_job.contains("remote_base_path"));
    assert!(package_job.contains("./tools/package-alpha-macos.sh"));
    assert!(package_job.contains("APPLE_DEVELOPER_ID_APPLICATION"));
    assert!(package_job.contains("APPLE_NOTARY_PROFILE"));
    assert!(package_job.contains("scp \"${scp_args[@]}\" -r \"$PUBLISH_DIR\""));
    assert!(package_job.contains("flock 9"));
    assert!(package_job.contains("Remote live and incoming paths must not overlap"));
    assert!(package_job.contains("Refusing to replace existing versioned package"));
    assert!(package_job.contains("path: target/velopack/Publish/"));
    assert!(!package_job.contains("path: target/velopack/Releases/"));
    assert!(package_job.contains("compression-level: 0"));
    assert!(package_job.contains("retention-days: 3"));
    let publish_step = package_job
        .find("- name: Publish artifacts to VPS")
        .unwrap();
    let artifact_step = package_job
        .find("- name: Upload Velopack publish payload")
        .unwrap();
    assert!(publish_step < artifact_step);

    assert!(tool_manifest.contains("\"version\": \"1.2.0\""));
    assert!(release_doc.contains("Open Anyway"));
    assert!(release_doc.contains("STUDIO_UPDATE_FEED_URL"));
    assert!(release_doc.contains("STUDIO_DEFAULT_UPDATE_FEED_URL"));
    assert!(release_doc.contains("mac-sonoma-x64"));
    assert!(release_doc.contains("feed metadata"));
}

#[test]
fn windows_alpha_package_stages_relay_binary() {
    let package_script = include_str!("../../../tools/package-alpha-windows.ps1");

    assert!(package_script.contains("[string]$SourceRelayExe"));
    assert!(package_script.contains("\"-p\", \"studio-relay\""));
    assert!(package_script.contains(
        r#"$releaseExeDir = if ([string]::IsNullOrWhiteSpace($Target)) { "target/release" } else { "target/$Target/release" }"#
    ));
    assert!(
        package_script
            .contains(r#"$SourceRelayExe = Join-Path $repoRoot "$releaseExeDir/studio-relay.exe""#)
    );
    assert!(package_script.contains("studio-relay.exe"));
}
