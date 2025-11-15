use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=../build.rs");
    shared_build::run()
}

mod shared_build {
    include!("../build.rs");
}
