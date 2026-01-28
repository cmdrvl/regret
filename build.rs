fn main() {
    // Link advapi32 on Windows for security functions used by libgit2
    #[cfg(windows)]
    println!("cargo:rustc-link-lib=advapi32");
}
