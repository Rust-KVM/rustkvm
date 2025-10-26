use rust_embed::RustEmbed;

#[derive(RustEmbed)]
// #[folder = "../client/dist"]
#[folder = "../client/static"]
pub struct ClientAssets;

// Built-in disk images embedded into the binary.
#[derive(RustEmbed)]
#[folder = "../assets/images"]
pub struct BuiltinImages;
