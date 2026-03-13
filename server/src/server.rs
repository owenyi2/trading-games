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

use crate::BusBroadcaster;

use protocol::{
    ClientAction, ClientMessage, ExchangeEvent, ExchangeId, ExchangePrivateMessage, ServerMessage,
    SystemMessage,
};

pub struct ConnectionHandler {
    id: AccountId,
    ws_tx: mpsc::Sender<ServerMessage>,
    ws_rx: mpsc::Receiver<ClientMessage>,

    private_rx: Option<mpsc::Receiver<(ExchangeId, ExchangePrivateMessage)>>,
    bus_readers: HashMap<ExchangeId, bus::BusReader<ExchangeEvent>>,
    msg_senders: HashMap<ExchangeId, mpsc::Sender<Message>>,
}

impl ConnectionHandler {
    fn new(
        id: AccountId,
        ws_tx: mpsc::Sender<ServerMessage>,
        ws_rx: mpsc::Receiver<ClientMessage>,
    ) -> Self {
        ConnectionHandler {
            id,
            ws_tx,
            ws_rx,
            private_rx: None,
            bus_readers: HashMap::new(),
            msg_senders: HashMap::new(),
        }
    }
    pub fn run(&mut self, stop_receiver: mpsc::Receiver<SystemMessage>) {
        let server_message = ServerMessage::System(SystemMessage::AccountId {
            account_id: self.id,
        });
        let _ = self.ws_tx.send(server_message);
        for exchange_id in self.msg_senders.keys() {
            let server_message = ServerMessage::System(SystemMessage::ExchangeAdded {
                exchange_id: *exchange_id,
            });
            let _ = self.ws_tx.send(server_message);
        }
        self.ws_tx.send(ServerMessage::System(SystemMessage::Start));

        loop {
            // 0. Check if server wants to stop us
            if let Ok(msg) = stop_receiver.try_recv() {
                match msg {
                    SystemMessage::End => break,
                    _ => unreachable!(),
                }
            }
            // 1. Check each exchange event
            for (exchange_id, reader) in &mut self.bus_readers {
                while let Ok(event) = reader.try_recv() {
                    let server_message = ServerMessage::Event(*exchange_id, event);
                    let _ = self.ws_tx.send(server_message);
                }
            }
            // 2. Check for private exchange messages
            if let Some(rx) = &self.private_rx {
                // self.private_rx is wrapped in an option because the rx is added later in the
                // objects lifetime by Server.sync
                while let Ok((exchange_id, msg)) = rx.try_recv() {
                    let server_message = ServerMessage::Private(exchange_id, msg);
                    let _ = self.ws_tx.send(server_message);
                }
            }
            // 3. Check the websocket for incoming messages
            while let Ok(msg) = self.ws_rx.try_recv() {
                if let Some(msg_sender) = self.msg_senders.get_mut(&msg.exchange_id) {
                    let account_id = self.id;
                    let order = match msg.action {
                        ClientAction::CancelOrder { order_id } => Message::CancelOrder {
                            order_id,
                            account_id,
                        },
                        ClientAction::InsertOrder {
                            side,
                            price,
                            qty,
                            client_order_id,
                        } => Message::InsertOrder {
                            account_id,
                            price,
                            qty,
                            client_order_id,
                            side: match side {
                                true => Side::Bid,
                                false => Side::Ask,
                            },
                        },
                        ClientAction::End => {
                            todo!()
                        }
                    };
                    msg_sender.send(order);
                }
            }
        }
    }
}

type Connections = HashMap<AccountId, ConnectionHandler>;

pub struct ExchangeHandler {
    exchange: Exchange<BusBroadcaster>,
    msg_sender: mpsc::Sender<Message>,
}

#[derive(Clone, Copy)]
pub enum Command {
    Stop,
    Play,
    Pause,
    AddExchange,
}

fn parse_command(input: &str) -> Result<Command, String> {
    let parts: Vec<&str> = input.trim().split_whitespace().collect();

    match parts.as_slice() {
        ["play"] => Ok(Command::Play),
        ["pause"] => Ok(Command::Pause),
        ["add_exchange"] => Ok(Command::AddExchange),
        ["stop"] => Ok(Command::Stop),
        _ => Err("Unknown command".into()),
    }
}

pub struct RunningServer {
    io_tx: mpsc::Sender<String>,
    io_rx: mpsc::Receiver<Command>,

    connections: HashMap<AccountId, JoinHandle<ConnectionHandler>>,
    exchanges: HashMap<ExchangeId, JoinHandle<ExchangeHandler>>,

    connection_stoppers: HashMap<AccountId, mpsc::Sender<SystemMessage>>,
    exchange_stoppers: HashMap<ExchangeId, mpsc::Sender<Message>>,

    ws_key: String,
}

impl RunningServer {
    pub fn run(self) -> PausedServer {
        loop {
            while let Ok(command) = self.io_rx.try_recv() {
                match command {
                    Command::Pause => {
                        return self.create_paused_server();
                    }
                    _ => {
                        self.io_tx
                            .send("Command not available. Server is Running".to_string());
                    }
                }
            }
        }
    }
    fn create_paused_server(self) -> PausedServer {
        let mut paused_connections = HashMap::new();

        for (_, stopper) in self.connection_stoppers {
            stopper.send(SystemMessage::End);
        }
        for (connection_id, connection) in self.connections {
            paused_connections.insert(connection_id, connection.join().unwrap());
        }

        let mut paused_exchanges = HashMap::new();
        for (_, stopper) in self.exchange_stoppers {
            stopper.send(Message::End);
        }
        for (exchange_id, exchange) in self.exchanges {
            paused_exchanges.insert(exchange_id, exchange.join().unwrap());
        }

        PausedServer::new(
            paused_connections,
            paused_exchanges,
            self.ws_key,
            self.io_tx,
            self.io_rx,
        )
    }
}

fn run_websocket(
    server_rx: mpsc::Receiver<ServerMessage>,
    client_tx: mpsc::Sender<ClientMessage>,
    mut websocket: tungstenite::WebSocket<TcpStream>,
) -> tungstenite::WebSocket<TcpStream> {
    websocket
        .get_mut()
        .set_nonblocking(true)
        .expect("Cannot set non-blocking");
    loop {
        if let Ok(msg) = server_rx.try_recv() {
            if matches!(msg, ServerMessage::System(SystemMessage::End)) {
                break;
            }
            let encoded: Vec<u8> = rmp_serde::to_vec(&msg).unwrap();
            let _ = websocket.send(tungstenite::Message::Binary(encoded.into()));
        }
        if let Ok(msg) = websocket.read() {
            if msg.is_binary() {
                if let Ok(parsed) = rmp_serde::from_slice::<ClientMessage>(&msg.into_data()) {
                    let _ = client_tx.send(parsed);
                }
            }
        }
    }
    websocket
}

pub struct PausedServer {
    io_tx: mpsc::Sender<String>,
    io_rx: mpsc::Receiver<Command>,

    stop: Arc<atomic::AtomicBool>,
    connections: Arc<Mutex<Connections>>,
    exchanges: HashMap<ExchangeId, ExchangeHandler>,
    accept_handle: Option<JoinHandle<()>>,
    ws_key: String,
}

impl PausedServer {
    pub fn new(
        connections: Connections,
        exchanges: HashMap<ExchangeId, ExchangeHandler>,
        ws_key: String,
        io_tx: mpsc::Sender<String>,
        io_rx: mpsc::Receiver<Command>,
    ) -> Self {
        let stop = Arc::new(atomic::AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(connections));

        let stop_clone = stop.clone();
        let connections_clone = connections.clone();

        let ws_key_clone = ws_key.clone();
        let accept_handle = thread::spawn(move || {
            Self::accept_connections(connections_clone, stop_clone, ws_key_clone);
        });

        PausedServer {
            stop,
            connections,
            exchanges,
            accept_handle: Some(accept_handle),
            ws_key,
            io_tx,
            io_rx,
        }
    }
    fn create_running_server(self) -> RunningServer {
        let Self {
            exchanges,
            connections,
            ws_key,
            io_tx,
            io_rx,
            ..
        } = self;

        let mut running_exchanges = HashMap::new();
        let mut running_connections = HashMap::new();

        let mut exchange_stoppers = HashMap::new();
        let mut connection_stoppers = HashMap::new();

        for (exchange_id, mut exchange_handler) in exchanges {
            exchange_stoppers.insert(exchange_id, exchange_handler.msg_sender.clone());
            running_exchanges.insert(
                exchange_id,
                thread::spawn(move || {
                    exchange_handler.exchange.run();
                    exchange_handler
                }),
            );
        }
        let mut connections_inner = connections.lock().unwrap();
        for (connection_id, mut connection_handler) in connections_inner.drain() {
            let (stop_tx, stop_rx) = mpsc::channel();
            connection_stoppers.insert(connection_id, stop_tx);
            running_connections.insert(
                connection_id,
                thread::spawn(move || {
                    connection_handler.run(stop_rx);
                    connection_handler
                }),
            );
        }
        RunningServer {
            exchanges: running_exchanges,
            connections: running_connections,
            exchange_stoppers,
            connection_stoppers,
            ws_key,
            io_tx,
            io_rx,
        }
    }
    pub fn run(mut self) -> Option<RunningServer> {
        loop {
            while let Ok(command) = self.io_rx.try_recv() {
                match command {
                    Command::Stop => {
                        self.stop.store(true, atomic::Ordering::Relaxed);
                        let _ = self.accept_handle.take().unwrap().join();
                        break;
                    }
                    Command::AddExchange => {
                        self.add_exchange();
                    }
                    Command::Play => {
                        self.stop.store(true, atomic::Ordering::Relaxed);
                        let _ = self.accept_handle.take().unwrap().join();
                        self.sync();
                        return Some(self.create_running_server());
                    }
                    _ => {
                        self.io_tx
                            .send("Command not Available. Server is Paused".to_string());
                    }
                }
            }
        }
    }
    fn add_exchange(&mut self) {
        let exchange_id: u64 = self.exchanges.len().try_into().unwrap();

        let (msg_sender, msg_receiver) = mpsc::channel();
        let bus = bus::Bus::<ExchangeEvent>::new(256);

        let broadcaster = BusBroadcaster::new(exchange_id, bus);
        let mut exchange: Exchange<BusBroadcaster> = Exchange::new(broadcaster, msg_receiver);
        self.exchanges.insert(
            exchange_id,
            ExchangeHandler {
                exchange,
                msg_sender,
            },
        );
    }

    fn accept_connections(
        connections: Arc<Mutex<Connections>>,
        stop_accepting: Arc<atomic::AtomicBool>,
        ws_key: String,
    ) {
        use tungstenite::handshake::server::{Request, Response};
        static COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(0);

        let addr = "127.0.0.1:8080";
        let listener = TcpListener::bind(addr).expect("Failed to bind");

        listener
            .set_nonblocking(true)
            .expect("Cannot set non-blocking");

        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    s.set_nonblocking(false).expect("Cannot set non-blocking");

                    let callback = |request: &Request, mut response: Response| {
                        if let Some(key) = request.headers().get("ws_secret_key") {
                            if *key == *ws_key {
                                Ok(response)
                            } else {
                                let resp = tungstenite::http::Response::builder()
                                    .status(401)
                                    .body(Some("Unauthorized".into()))
                                    .unwrap();
                                Err(resp)
                            }
                        } else {
                            let resp = tungstenite::http::Response::builder()
                                .status(401)
                                .body(Some("Unauthorized".into()))
                                .unwrap();
                            Err(resp)
                        }
                    };
                    let mut websocket = match tungstenite::accept_hdr(s, callback) {
                        Ok(ws) => ws,
                        Err(e) => {
                            // TODO: check if err because we rejected connection or some other
                            // failure
                            continue;
                        }
                    };

                    let connection_id = COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
                    let (server_tx, server_rx) = mpsc::channel();
                    let (client_tx, client_rx) = mpsc::channel();

                    let ws_thread =
                        thread::spawn(move || run_websocket(server_rx, client_tx, websocket));
                    connections.lock().unwrap().insert(
                        connection_id,
                        ConnectionHandler::new(connection_id, server_tx, client_rx),
                    );
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("Encountered IO error: {e}"),
            }
            if stop_accepting.load(atomic::Ordering::Relaxed) {
                break;
            }
        }
    }
    fn sync(&mut self) {
        let mut connections_lock = self.connections.lock().unwrap();
        for (connection_id, connection) in connections_lock.iter_mut() {
            let (private_tx, private_rx) = mpsc::channel();
            for (exchange_id, exchange) in self.exchanges.iter_mut() {
                // Each Connection needs to subscribe a bus reader to each Exchange
                connection
                    .bus_readers
                    .insert(*exchange_id, exchange.exchange.broadcaster.tx.add_rx());
                // Each Connection needs to clone a msg_sender to each Exchange
                connection
                    .msg_senders
                    .insert(*exchange_id, exchange.msg_sender.clone());
                // Each Exchange.broadcaster needs to populate its private_txs

                exchange
                    .exchange
                    .broadcaster
                    .private_txs
                    .insert(*connection_id, private_tx.clone());
            }
            // Each connection needs to have private_rx
            connection.private_rx = Some(private_rx);
        }
    }
}

pub fn shell(tx: mpsc::Sender<Command>, rx: mpsc::Receiver<String>) {
    loop {
        print!(">> ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {
                let input = input.trim();

                // skip empty input
                if input.is_empty() {
                    continue;
                }
                if let Ok(cmd) = parse_command(&input) {
                    tx.send(cmd);
                    // todo: exit gracefully if cmd says so
                } else {
                    eprintln!("Unrecognised Command");
                }
            }
            Err(error) => {
                eprintln!("Error reading input: {}", error);
            }
        }
        thread::sleep(time::Duration::from_millis(10)); // give some time to receive response 
        while let Ok(msg) = rx.try_recv() {
            println!("<< {}", msg);
        }
    }
}
