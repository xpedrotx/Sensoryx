fn main() {
    // O driver PawnIO e o ETW (FPS) exigem administrador: embute o manifesto só no release,
    // para `cargo run` em debug continuar funcionando sem elevação.
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if release && windows {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
    }
}
