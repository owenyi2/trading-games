pub mod app;
pub use app::StateMachineApp;

// TODO: Figure out how to port to wasm
// Apparently shit like threads don't work on WASM

// #[cfg(target_arch = "wasm32")]
// use wasm_bindgen::prelude::*;
//
// #[cfg(target_arch = "wasm32")]
// #[wasm_bindgen]
// pub async fn start(canvas_id: &str) -> Result<(), eframe::WebError> {
//     let web_options = eframe::WebOptions::default();
//
//     eframe::WebRunner::new()
//         .start(
//             canvas_id,
//             web_options,
//             Box::new(|_cc| Ok(Box::new(StateMachineApp::default()))),
//         )
//         .await
// }
