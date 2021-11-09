fn main() {
    // Must enable rcon on Windows
    #[cfg(all(windows, not(feature = "rcon")))]
    {
        println!("cargo:warning=lazymc: you must enable rcon feature on Windows");
    }
}
