mod build_support;

fn main() {
    println!("cargo:rerun-if-env-changed=AUUMAIL_GOOGLE_CLIENT_SECRET");
    println!("cargo:rerun-if-changed=google-oauth-secret.local");
    println!("cargo:rerun-if-changed=build_support.rs");
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let secret = build_support::load_secret(
        std::env::var_os("AUUMAIL_GOOGLE_CLIENT_SECRET"),
        &root.join("google-oauth-secret.local"),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if secret.is_none() && std::env::var("PROFILE").as_deref() == Ok("release") {
        panic!("Release builds require AUUMAIL_GOOGLE_CLIENT_SECRET or google-oauth-secret.local");
    }
    // Do not print the value through cargo:rustc-env (Cargo persists build output).
    let generated = build_support::secret_source(secret.as_deref());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
        .join("google_oauth_secret.rs");
    if std::fs::read_to_string(&output).ok().as_deref() != Some(&generated) {
        std::fs::write(output, generated).expect("Could not write OAuth build configuration");
    }
    tauri_build::build();
}
