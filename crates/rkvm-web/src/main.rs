mod app;
mod hid;
mod net;
mod ocr;
mod rpc;
mod settings;
mod settings_advanced;
mod terminal;
mod util;

fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    leptos::mount::mount_to_body(app::App);
}
