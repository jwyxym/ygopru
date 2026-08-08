use glob::glob;
use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=ocgcore/");
    println!("cargo:rerun-if-changed=lua/");
    println!("cargo:rerun-if-changed=src/random.cpp");

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ocgcore_dir = root.join("ocgcore");
    let lua_dir = root.join("lua");

    if !ocgcore_dir.exists() {
        panic!(
            "ocgcore not found at {}. Please run: git submodule update --init",
            ocgcore_dir.display()
        );
    }

    let mut build = cc::Build::new();

    // Suppress all C/C++ warnings (GCC/Clang -w, MSVC /w), errors still fail the build
    build.warnings(false);

    // Compile Lua C files
    for entry in glob(lua_dir.join("*.c").to_str().unwrap()).unwrap() {
        let path = entry.unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        if filename != "lua.c" && filename != "luac.c" && filename != "onelua.c" {
            build.file(&path);
        }
    }

    // Compile ocgcore C++ files
    build.cpp(true);
    build.flag_if_supported("-std=c++14");
    build.include(&ocgcore_dir);
    build.include(&lua_dir);

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "msvc" {
        build.flag("/TP");
    } else {
        build.flag("-Wno-deprecated-declarations");
    }

    for entry in glob(ocgcore_dir.join("*.cpp").to_str().unwrap()).unwrap() {
        let path = entry.unwrap();
        build.file(&path);
    }

    build.file(root.join("src").join("random.cpp"));

    build.compile("ygopro-core");
}
