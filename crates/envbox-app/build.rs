fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../icons/icon.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../icons/icon.ico");
        res.set("ProductName", "Aura");
        res.set("FileDescription", "Aura");
        res.set("CompanyName", "EnvBox");
        res.compile()
            .expect("failed to embed Windows application icon");
    }
}
