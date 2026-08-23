fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.set("ProductName", "IsImeOn");
        res.set("FileDescription", "IME連動キャレットインジケーター");
        res.set("LegalCopyright", "MIT License");
        res.compile().expect("embed Windows resources");
    }
}
