use std::{collections::HashMap, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=icon.ico");
    println!("cargo:rerun-if-changed=src/ui");

    let library = HashMap::from([(
        "lucide".to_string(),
        PathBuf::from(lucide_slint::lib()),
    )]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library);

    slint_build::compile_with_config("src/ui/main.slint", config).expect("Slint build failed");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}
