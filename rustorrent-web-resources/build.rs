use actix_web_static_files;
use std::{env, path::Path};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();

    let generated_files = Path::new(&out_dir).join("generated_files.rs");
    actix_web_static_files::NpmBuild::new("./static/files")
        .install().unwrap()
        .run("build").unwrap()
        .target("./static/files/target/classes/static")
        .to_resource_dir()
        .with_generated_filename(&generated_files)
        .with_generated_fn("generate_files")
        .build()
        .unwrap();

    let generated_css = Path::new(&out_dir).join("generated_css.rs");
    actix_web_static_files::generate_resources(
        "./static/css",
        None,
        &generated_css,
        "generate_css",
    )
    .unwrap();
}
