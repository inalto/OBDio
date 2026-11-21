// Nelle build di rilascio non apre la finestra della console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    obdio::ui::esegui()
}
