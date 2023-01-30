fn main() {
    // rcon is required on Windows
    #[cfg(all(windows, not(feature = "rcon")))]
    {
        compile_error!("required feature missing on Windows: rcon");
    }
}
