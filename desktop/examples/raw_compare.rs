//! Local reference comparison helper; never changes input files or catalogs.
use advanced_explorer_develop as engine;
use std::path::Path;
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 4, "raw_compare INPUT CASES.json OUTPUT_DIR");
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    pool.install(|| -> anyhow::Result<()> {
        let base = engine::prepare(Path::new(&args[1]), 1600)?;
        let destination = Path::new(&args[3]);
        let cases: std::collections::BTreeMap<String, engine::Settings> =
            serde_json::from_slice(&std::fs::read(&args[2])?)?;
        for (name, settings) in &cases {
            anyhow::ensure!(
                !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                "Invalid case name"
            );
            settings.validate()?;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir(destination)?;
        std::fs::write(
            destination.join("base.json"),
            serde_json::to_vec_pretty(&base.info)?,
        )?;
        engine::write_base(&destination.join("base.tif"), &base)?;
        for (name, settings) in cases {
            settings.validate()?;
            let image = engine::render(&base, &settings, &|| false)?;
            image.image.save(destination.join(format!("{name}.png")))?;
        }
        Ok(())
    })
}
