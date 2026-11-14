fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../packaging/shadowpainel.ico");
        resource.set("ProductName", "ShadowPainel");
        resource.set("FileDescription", "Painel local para servidores Minecraft");
        resource.set("CompanyName", "NoteDarck");
        resource.set("LegalCopyright", "Copyright (c) NoteDarck");
        resource
            .compile()
            .expect("falha ao embutir recursos do Windows");
    }
}
