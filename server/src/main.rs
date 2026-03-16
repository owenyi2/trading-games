mod broadcaster;
mod server;

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, atomic, mpsc};
use std::thread::{self, JoinHandle};
use std::time;

use bus;
use dotenv::dotenv;
use rmp_serde;
use tungstenite;

use exchange::exchange::Exchange;
use exchange::types::*;

use broadcaster::BusBroadcaster;

use protocol::{
    ClientAction, ClientMessage, ExchangeEvent, ExchangeId, ExchangePrivateMessage, ServerMessage,
    SystemMessage,
};

use server::*;

fn main() {
    dotenv().ok();
    let secret_key = env::var("WS_SECRET_KEY").expect("WS_SECRET_KEY must be set in .env");

    let (io_tx, rx) = mpsc::channel::<String>();
    let (tx, io_rx) = mpsc::channel::<Command>();

    let _ = thread::spawn(move || {
        shell(tx, rx);
    });

    let mut paused_server =
        PausedServer::new(HashMap::new(), HashMap::new(), secret_key, io_tx, io_rx);

    loop {
        let Some(mut running_server) = paused_server.run() else {
            // state machine cycles between Paused Server or Running Server
            break;
        };
        paused_server = running_server.run();
    }
}
