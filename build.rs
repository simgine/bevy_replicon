fn main() {
    if std::env::var_os("CARGO_FEATURE_SCENE").is_some() {
        println!(
            "cargo:warning=the `scene` feature is deprecated, use `world_serialization` instead."
        );
    }
}
