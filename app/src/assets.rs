use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../crates/rkvm-web/dist"]
pub struct ClientAssets;
#[derive(RustEmbed)]
#[folder = "../assets/images"]
pub struct BuiltinImages;
