#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod desktop;
mod frontend_probe;
mod integration_smoke;
mod ledger;
mod runtime_profile;
mod window_geometry;

fn main() {
    if let Some(exit_code) = ledger::smoke::handle_cli_mode() {
        std::process::exit(exit_code);
    }
    app::run();
}
