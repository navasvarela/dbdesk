//! Links the C++ standard library when DuckDB comes from its prebuilt static library.
//!
//! Compiled from source (`bundled`), libduckdb-sys's `cc` build adds it. Linked from
//! `DUCKDB_LIB_DIR`, libduckdb-sys only asks for `duckdb_static`, whose C++ code needs libc++ on
//! macOS and libstdc++ on Linux. Windows links DuckDB's DLL, which carries its own runtime.

fn main() {
    if std::env::var_os("CARGO_CFG_DBROVER_PREBUILT_DUCKDB").is_none() {
        return;
    }
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => println!("cargo:rustc-link-lib=c++"),
        Ok("linux") => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }
}
