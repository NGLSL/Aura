fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../icons/icon.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../icons/icon.ico");
        res.set("ProductName", "EnvBox");
        res.set("FileDescription", "EnvBox process-scoped environment virtualization");
        res.set("CompanyName", "EnvBox");
        res.compile()
            .expect("failed to embed Windows application icon");
    }
}
