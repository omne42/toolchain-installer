fn main() {
    let wants_version = std::env::args().any(|arg| arg == "--version" || arg == "-V");
    if wants_version {
        println!("ti-cargo-fixture 0.1.0");
    } else {
        println!("ti-cargo-fixture");
    }
}
