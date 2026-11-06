//! Exports a vault to a static site: `cargo run -p jiyue-export --example export_demo -- <vault> <out>`.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(root), Some(out)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: export_demo <vault-dir> <out-dir>");
        std::process::exit(2);
    };
    let opts = jiyue_export::SiteOptions {
        title: "Jiyue 示例笔记".to_string(),
        toc: true,
        ..Default::default()
    };
    match jiyue_export::export_vault_site(Path::new(root), Path::new(out), &opts) {
        Ok(report) => {
            println!(
                "pages={} assets={} unresolved={} missing_images={} search={} skipped={}",
                report.pages,
                report.assets_copied,
                report.unresolved_links,
                report.missing_images,
                report.search_entries,
                report.skipped_unpublished
            );
            for w in &report.warnings {
                println!("warning: {w}");
            }
        }
        Err(e) => {
            eprintln!("export failed: {e}");
            std::process::exit(1);
        }
    }
}
