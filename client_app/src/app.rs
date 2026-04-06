use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe;
use eframe::egui;
use rmp_serde;
use tungstenite;
use tungstenite::stream::MaybeTlsStream;

use client_lib::Client;
use exchange::book::OrderBook;
use exchange::types::*;

use protocol::{
    ClientAction, ClientMessage, ExchangeEvent, ExchangeId, ExchangePrivateMessage, ServerMessage,
    SystemMessage,
};

pub struct StateMachineApp {
    state: App,
}

impl Default for StateMachineApp {
    fn default() -> Self {
        Self {
            state: App::MainMenu(MainMenu::new()),
        }
    }
}

impl eframe::App for StateMachineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(16)); // ~60 FPS

        egui::CentralPanel::default().show(ctx, |ui| {
            self.state = match std::mem::replace(&mut self.state, App::Empty) {
                App::MainMenu(state) => state.update(ui),
                App::Running(state) => state.update(ui),
                App::Empty => unreachable!(),
            };
        });
    }
}

struct MainMenu {
    addr_str: String,
    // port_str: String,
    game_key_str: String,
    valid_addr_port_key: bool,
}

impl MainMenu {
    fn new() -> Self {
        Self {
            addr_str: String::from("ws://127.0.0.1:8080"),
            // port_str: String::from(""),
            game_key_str: String::new(),
            valid_addr_port_key: false,
        }
    }
}

struct Running {
    client: Client,
    command_strs: HashMap<ExchangeId, String>,
    volume_preset: HashMap<ExchangeId, Quantity>,
}
impl Running {
    fn try_from_main_menu(state: &mut MainMenu) -> Option<Self> {
        dbg!("HELLO");
        use tungstenite::{ClientRequestBuilder, connect, http::Uri};

        // let uri: tungstenite::http::Uri = format!("ws://{}:{}", state.addr_str, state.port_str).parse().ok()?;
        let uri: Uri = state.addr_str.parse().ok()?;
        let builder =
            ClientRequestBuilder::new(uri).with_header("ws_secret_key", state.game_key_str.clone());

        let (ws, response) = match tungstenite::connect(builder) {
            Ok(x) => x,
            Err(e) => {
                panic!("{}", e);
            }
        };

        let client = Client::connect(ws);
        Some(Running {
            client,
            command_strs: HashMap::new(),
            volume_preset: HashMap::new(),
        })
    }
}

enum App {
    MainMenu(MainMenu),
    Running(Running),
    Empty,
}

impl MainMenu {
    fn update(mut self, ui: &mut egui::Ui) -> App {
        ui.heading("Main Menu");
        self.valid_addr_port_key = true;
        ui.heading("Server IP Addres:");
        ui.text_edit_singleline(&mut self.addr_str);
        let addr = self.addr_str.trim();
        ui.heading("Game Key:");
        ui.text_edit_singleline(&mut self.game_key_str);

        if ui.button("Connect").clicked() && self.valid_addr_port_key {
            if let Some(state) = Running::try_from_main_menu(&mut self) {
                return App::Running(state);
            }
        }
        App::MainMenu(self)
    }
}

enum InternalAction {
    InsertOrder {
        side: Side,
        price: Price,
        qty: Quantity,
    },
    Hit,
    Lift,
    CancelLevel {
        price: Price,
    },
    CancelAll,
    SetVolume(Quantity),
}

impl Running {
    fn draw_orderbook(
        ui: &mut egui::Ui,
        id: impl std::hash::Hash,
        bids: Vec<(u64, u64)>,
        asks: Vec<(u64, u64)>,
    ) {
        use std::collections::BTreeSet;

        egui::Grid::new(id).striped(true).show(ui, |ui| {
            ui.label("Bid Vol");
            ui.label("Price");
            ui.label("Ask Vol");
            ui.end_row();

            // Collect all price levels
            let mut prices = BTreeSet::new();
            for (p, _) in &bids {
                prices.insert(*p);
            }
            for (p, _) in &asks {
                prices.insert(*p);
            }

            // Iterate highest → lowest price
            for price in prices.iter().rev() {
                let bid_vol = bids.iter().find(|(p, _)| p == price).map(|(_, v)| *v);

                let ask_vol = asks.iter().find(|(p, _)| p == price).map(|(_, v)| *v);

                if let Some(v) = bid_vol {
                    ui.label(format!("{:>6}", v));
                } else {
                    ui.label("");
                }

                ui.label(format!("{:>6}", price));

                if let Some(v) = ask_vol {
                    ui.label(format!("{:>6}", v));
                } else {
                    ui.label("");
                }

                ui.end_row();
            }
        });
    }
    fn draw_command_input(&mut self, exchange_id: ExchangeId, ui: &mut egui::Ui) {
        ui.label("Command: ");

        let user_command = self
            .command_strs
            .entry(exchange_id)
            .or_insert_with(|| String::new());
        let response = ui.add(egui::TextEdit::singleline(user_command));
        let command = user_command.clone();
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(command) = self.parse_command(&command, exchange_id) {
                match command {
                    InternalAction::InsertOrder { side, price, qty } => {
                        self.client.limit_order(exchange_id, side, price, qty);
                    }
                    InternalAction::Hit => {
                        self.client.hit_best_bid(exchange_id);
                    }
                    InternalAction::Lift => {
                        self.client.lift_best_ask(exchange_id);
                    }
                    InternalAction::CancelAll => {
                        self.client.cancel_all(exchange_id);
                    }
                    InternalAction::CancelLevel { price } => {
                        self.client.cancel_level(exchange_id, price);
                    }
                    InternalAction::SetVolume(qty) => {
                        *self.volume_preset.entry(exchange_id).or_insert(1) = qty;
                    }
                }
            }

            let user_command = self
                .command_strs
                .entry(exchange_id)
                .or_insert_with(|| String::new());
            user_command.clear();
        }
    }

    fn draw_messages(&mut self, ui: &mut egui::Ui) {
        fn format_message_row(
            msg: &(ExchangeId, ExchangePrivateMessage),
        ) -> (String, String, String, String, String) {
            let (exchange_id, msg) = msg;
            match msg {
                ExchangePrivateMessage::InsertConfirm {
                    client_order_id,
                    order_id,
                    id,
                } => (
                    exchange_id.to_string(),                        // Column 1: Exhg. Id
                    "Insert".to_string(),                           // Column 2: Act.
                    id.to_string(),                                 // Column 3: Event Id
                    order_id.to_string(),                           // Column 4: Order Id
                    format!("client_order_id={}", client_order_id), // Column 5: Info
                ),
                ExchangePrivateMessage::CancelConfirm { order_id, id } => (
                    exchange_id.to_string(),
                    "Cancel".to_string(),
                    id.to_string(),
                    order_id.to_string(),
                    "".to_string(), // No extra info
                ),
                ExchangePrivateMessage::TradeConfirm {
                    order_id,
                    trade_price,
                    trade_volume,
                    side,
                    id,
                } => (
                    exchange_id.to_string(),
                    "Trade".to_string(),
                    id.to_string(),
                    order_id.to_string(),
                    format!(
                        "price={} volume={} side={}",
                        trade_price, trade_volume, side
                    ),
                ),
            }
        }
        fn format_event_row(
            event: &(ExchangeId, ExchangeEvent),
        ) -> (String, String, String, String, String) {
            let (exchange_id, event) = event;
            match event {
                ExchangeEvent::Cancel { order_id, id } => (
                    exchange_id.to_string(),
                    "Cancel".to_string(),
                    id.to_string(),
                    order_id.to_string(),
                    "".to_string(),
                ),
                ExchangeEvent::Insert {
                    price,
                    qty,
                    side,
                    order_id,
                    id,
                } => (
                    exchange_id.to_string(),
                    "Insert".to_string(),
                    id.to_string(),
                    order_id.to_string(),
                    format!("price={}\tvolume={}\tside={}", price, qty, side),
                ),
                ExchangeEvent::Trade {
                    ask_id,
                    bid_id,
                    trade_price,
                    trade_volume,
                    id,
                } => (
                    exchange_id.to_string(),
                    "Trade".to_string(),
                    id.to_string(),
                    format!("bid={}\task={}", bid_id, ask_id),
                    format!("price={}\tvolume={}", trade_price, trade_volume),
                ),
            }
        }

        let height = 200.0;
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            ui.horizontal(|ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.vertical(|ui| {
                        let messages = self.client.msg_history();
                        egui_extras::TableBuilder::new(ui)
                            .id_source("message history")
                            .stick_to_bottom(true)
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(200.))
                            .header(28.0, |mut header| {
                                header.col(|ui| {
                                    ui.label("Exchange Id");
                                });
                                header.col(|ui| {
                                    ui.label("Action");
                                });
                                header.col(|ui| {
                                    ui.label("Event Id");
                                });
                                header.col(|ui| {
                                    ui.label("Order Id");
                                });
                                header.col(|ui| {
                                    ui.label("Info");
                                });
                            })
                            .body(|mut body| {
                                body.rows(18.0, messages.len(), |mut row| {
                                    let i = row.index();
                                    let (exhg_id, act, event_id, order_id, info) =
                                        format_message_row(&messages[i]);

                                    row.col(|ui| {
                                        ui.label(exhg_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(act);
                                    });
                                    row.col(|ui| {
                                        ui.label(event_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(order_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(info);
                                    });
                                });
                            });
                    });
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.vertical(|ui| {
                        let events = self.client.event_history();
                        egui_extras::TableBuilder::new(ui)
                            .id_source("event history")
                            .stick_to_bottom(true)
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(80.))
                            .column(egui_extras::Column::exact(200.))
                            .header(28.0, |mut header| {
                                header.col(|ui| {
                                    ui.label("Exchange Id");
                                });
                                header.col(|ui| {
                                    ui.label("Action");
                                });
                                header.col(|ui| {
                                    ui.label("Event Id");
                                });
                                header.col(|ui| {
                                    ui.label("Order Id");
                                });
                                header.col(|ui| {
                                    ui.label("Info");
                                });
                            })
                            .body(|mut body| {
                                body.rows(18.0, events.len(), |mut row| {
                                    let i = row.index();
                                    let (exhg_id, act, event_id, order_id, info) =
                                        format_event_row(&events[i]);

                                    row.col(|ui| {
                                        ui.label(exhg_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(act);
                                    });
                                    row.col(|ui| {
                                        ui.label(event_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(order_id);
                                    });
                                    row.col(|ui| {
                                        ui.label(info);
                                    });
                                });
                            });
                    });
                });
            });
        });
    }
    fn draw_position(
        &mut self,
        best_bid: Option<u64>,
        best_ask: Option<u64>,
        cash: i64,
        position: i64,
        ui: &mut egui::Ui,
    ) {
        let net = match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => {
                let midprice = (bid + ask) as f32 / 2.0;
                (cash as f32 + position as f32 * midprice).to_string()
            }
            _ => "N/A".to_string(),
        };

        ui.label(format!(
            "Cash: {} | Position: {} | Mark to Market: {}",
            cash, position, net
        ));
    }
    fn draw_our_orders(&self, exchange_id: ExchangeId, ui: &mut egui::Ui) {
        let height = 150.0;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(height);
            ui.label("Our Orders");
            let row_height = ui.text_style_height(&egui::TextStyle::Body);

            let our_orders = self.client.our_orders(exchange_id).unwrap_or(Vec::new());
            // Consider sorting our orders because otherwise the order is whatever arbitrary order the HashSet returns
            egui::ScrollArea::vertical()
                .id_source(exchange_id)
                .stick_to_bottom(true)
                .max_height(height)
                .show_rows(ui, row_height, our_orders.len(), |ui, row_range| {
                    for i in row_range {
                        let order = &our_orders[i];
                        ui.horizontal(|ui| {
                            ui.label(format!("{:?}", order));
                        });

                        ui.allocate_space(egui::vec2(
                            0.0,
                            row_height - ui.text_style_height(&egui::TextStyle::Body),
                        ));
                    }
                });
        });
    }

    fn parse_command(&self, input: &str, exchange_id: ExchangeId) -> Option<InternalAction> {
        let parts: Vec<&str> = input.trim().split_whitespace().collect();
        match parts.as_slice() {
            ["hit"] | ["yours"] => Some(InternalAction::Hit),
            ["lift"] | ["mine"] => Some(InternalAction::Lift),
            ["bid", price, qty] => Some(InternalAction::InsertOrder {
                side: Side::Bid,
                price: price.parse().ok()?,
                qty: qty.parse().ok()?,
            }),
            ["bid", price] => Some(InternalAction::InsertOrder {
                side: Side::Bid,
                price: price.parse().ok()?,
                qty: *self.volume_preset.get(&exchange_id).unwrap_or(&1),
            }),
            ["ask", price, qty] => Some(InternalAction::InsertOrder {
                side: Side::Ask,
                price: price.parse().ok()?,
                qty: qty.parse().ok()?,
            }),
            ["ask", price] => Some(InternalAction::InsertOrder {
                side: Side::Ask,
                price: price.parse().ok()?,
                qty: *self.volume_preset.get(&exchange_id).unwrap_or(&1),
            }),
            ["volume", qty] => Some(InternalAction::SetVolume(qty.parse().ok()?)),
            ["cancel", price] => Some(InternalAction::CancelLevel {
                price: price.parse().ok()?,
            }),
            ["cancel_all"] => Some(InternalAction::CancelAll),
            _ => None,
        }
    }

    fn update(mut self, ui: &mut egui::Ui) -> App {
        self.client.update();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        ui.label(format!("Unix time: {}", now));

        if let Some(id) = self.client.account_id() {
            ui.label(format!("Client Id: {}", id));
        }

        let mut books: Vec<_> = self
            .client
            .books()
            .into_iter()
            .map(|(exchange_id, book)| {
                let (bids, asks) = book.order_book.summarise();
                let (cash, position) = (book.cash, book.position);
                (*exchange_id, bids, asks, cash, position)
            })
            .collect();
        books.sort_by(|a, b| a.0.cmp(&b.0));

        ui.horizontal(|ui| {
            for (exchange_id, bids, asks, cash, position) in books {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(format!("Exchange {}", exchange_id));
                        let best_bid = bids.first().cloned().map(|x| x.0);
                        let best_ask = asks.first().cloned().map(|x| x.0);

                        Self::draw_orderbook(ui, format!("orderbook_{}", exchange_id), bids, asks);
                        self.draw_position(best_bid, best_ask, cash, position, ui);
                        self.draw_command_input(exchange_id, ui);
                        self.draw_our_orders(exchange_id, ui);
                    })
                });
            }
        });

        self.draw_messages(ui);
        // let debug = egui::Label::new(format!("{:?}", self.client).to_string()).wrap();
        // ui.add(debug);

        App::Running(self)
    }
}
