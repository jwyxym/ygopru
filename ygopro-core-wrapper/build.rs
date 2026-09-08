use glob::glob;
use std::env;
use std::path::Path;
use std::path::PathBuf;

/// The ocgcore and lua source pair this build compiles.
#[derive(Debug)]
enum Core {
    Official,
    Koishi,
    Custom { ocgcore_dir: PathBuf, lua_dir: PathBuf },
}

impl Core {
    fn from_enabled_features() -> Self {
        let official_enabled = env::var_os("CARGO_FEATURE_OFFICIAL").is_some();
        let koishi_enabled = env::var_os("CARGO_FEATURE_KOISHI").is_some();
        let custom_enabled = env::var_os("CARGO_FEATURE_CUSTOM").is_some();
        let enabled_core_count = [official_enabled, koishi_enabled, custom_enabled]
            .iter()
            .filter(|enabled| **enabled)
            .count();
        // custom wins over koishi, koishi wins over official.
        let core = if custom_enabled {
            let ocgcore_dir = PathBuf::from(env::var("YGOPRO_OCGCORE_DIR").unwrap_or_else(|_| {
                panic!("feature `custom` requires env YGOPRO_OCGCORE_DIR to point to an ocgcore source tree")
            }));
            let lua_dir = PathBuf::from(env::var("YGOPRO_LUA_DIR").unwrap_or_else(|_| {
                panic!("feature `custom` requires env YGOPRO_LUA_DIR to point to a lua source tree")
            }));
            Core::Custom { ocgcore_dir, lua_dir }
        } else if koishi_enabled {
            Core::Koishi
        } else {
            // No core feature selected falls back to official, so dependents that
            // disable default features still get the official core.
            Core::Official
        };
        if enabled_core_count > 1 {
            println!("cargo:warning=enabling more than one of `official`, `koishi`, `custom`; using `{:?}` by priority", core);
        }
        core
    }

    fn source_dirs(&self, root: &Path) -> (PathBuf, PathBuf) {
        match self {
            Core::Official => (
                root.join("official").join("ocgcore"),
                root.join("official").join("lua"),
            ),
            Core::Koishi => (
                root.join("koishi").join("ocgcore"),
                root.join("koishi").join("lua"),
            ),
            Core::Custom { ocgcore_dir, lua_dir } => (ocgcore_dir.clone(), lua_dir.clone()),
        }
    }

    fn library_name(&self) -> &'static str {
        match self {
            Core::Official => "ygopro-core-official",
            Core::Koishi => "ygopro-core-koishi",
            Core::Custom { .. } => "ygopro-core-custom",
        }
    }
}

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let core = Core::from_enabled_features();
    let (ocgcore_dir, lua_dir) = core.source_dirs(&root);
    let lib_name = core.library_name();

    println!("cargo:rerun-if-changed={}", ocgcore_dir.display());
    println!("cargo:rerun-if-changed={}", lua_dir.display());
    println!("cargo:rerun-if-changed=src/random.cpp");

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
        let filename = path.file_name().unwrap().to_str().unwrap();
        // koishi ships an emscripten-only translation unit that is empty otherwise.
        if filename == "emscripten_shutdown.cpp" {
            continue;
        }
        build.file(&path);
    }

    build.file(root.join("src").join("random.cpp"));

    build.compile(lib_name);
}
