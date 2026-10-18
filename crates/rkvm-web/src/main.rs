mod app;
mod hid;
mod i18n;
mod net;
mod ocr;
mod rpc;
mod settings;
mod settings_advanced;
mod shell;
mod terminal;
mod theme;
mod ui;
mod util;

fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    leptos::mount::mount_to_body(app::App);
}
