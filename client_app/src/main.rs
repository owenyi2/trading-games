use client_app::StateMachineApp;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Trading Client",
        options,
        Box::new(|_cc| Ok(Box::new(StateMachineApp::default()))),
    )
}
