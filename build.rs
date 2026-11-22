fn main() {
    embuild::espidf::sysenv::output();
    // Route the locked esp-idf-svc constructor through the SDK defaults shim.
    println!("cargo:rustc-link-arg=-Wl,--wrap=esp_wifi_init");
}
