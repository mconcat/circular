
fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, text) in [
        (
            engine::closed_tables::MODULE,
            engine::closed_tables::module(),
        ),
        (engine::closed_tables::TYPES, engine::closed_tables::types()),
    ] {
        std::fs::write(root.join(path), text)?;
        println!("{path}");
    }
    Ok(())
}
