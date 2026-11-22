fn main() {
    println!("cargo:rerun-if-env-changed=HUBBYPDF_ICON_RES");
    if let Some(resource) = std::env::var_os("HUBBYPDF_ICON_RES") {
        let resource = std::path::Path::new(&resource).display();
        println!("cargo:rerun-if-changed={resource}");
        println!("cargo:rustc-link-arg-bin=hubbypdf={resource}");
    }
}
