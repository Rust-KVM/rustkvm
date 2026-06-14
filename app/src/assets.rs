use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../client/static"]
pub struct ClientAssets;
#[derive(RustEmbed)]
#[folder = "../assets/images"]
pub struct BuiltinImages;
