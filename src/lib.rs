/// This module gets built and becomes a cdylib .so file to be loaded in redis and can be
/// used to plot data from redis. Can be used from a redis client to plot data on demand
/// or as a watcher, that automatically updates the plot when events change the data.
///
/// The module can plot directly over a buffer as a RGB image - which can be read from the
/// redis client and displayed as preferred - or output to a device (i.e. screen or file).

/*
sono in una situazione del cavolo: in nuova_lib.rs c'era l'implementazione che avevo fatto
in cui si prendono gli args e li si manda nel chan, che costruisce la finestra secondo
quelle specifiche e si registrano le chiavi associate...
però non funziona. d'altro canto, tornando indietro nelle modifiche, mi sono accorto
che questo file funziona, il motivo è che qui c'é la finestra che viene aperta dall'inizio:
se la tolgo, non se ne aprono più. sembra quasi che sia un problema di app che viene
distrutta, ma non è così (o almeno non sembra) perché comunque app::run viene eseguito
e in teoria l'app rimane sempre in scope, quindi il weak pointer dovrebbe sempre
essere valido - solo che non lo sappiamo perché - se tolgo la finestra magica -
quando invio i dati al canale, essi non arrivano mai
// TODO build a map plot-type -> series-name -> data
// Get all keys supported by this
// TODO vedere https://github.com/RedisLabsModules/redismodule-rs/blob/master/examples/lists.rs
//   per un esempio di come aprire le chiavi e vederne il tipo e agire direttamente
//   su di esse
// if let Ok(RedisValue::Array(keys)) = ctx.call("KEYS", &["rsp-plot*"]) {
//     for key in keys.iter() {
//         match key {
//             RedisValue::SimpleString(s) => println!("Key: {}", s),
//             RedisValue::SimpleStringStatic(s) => println!("Key: {}", s),
//             _ => {}
//         }
//     }
// }
*/

#[macro_use]
extern crate redis_module;

#[macro_use]
extern crate lazy_static;

use redis_module::{
    Context, LogLevel, NextArg, NotifyEvent, RedisError, RedisResult, RedisString, RedisValue,
    Status,
};

use gtk4::prelude::*;

use plotters::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

mod argparse;
mod utils;

use argparse::parse_args;
use utils::{build_ui, plot_complex, plot_stuff, PlotSpec};

const COLORS: &[(u8, u8, u8)] = &[(0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0x00, 0x00, 0xff)];

lazy_static! {
    // This is the channel to send drawing arguments to the dispatcher when a binding
    // command arrives, the dispatcher will set up the window and build a channel to
    // notify when keys are updated. That channel will be added in BOUND_KEYS.
    static ref DISPATCHER_TX: Mutex<Option<glib::Sender<DrawParams>>> = Mutex::new(None);

    // Maps redis keys to channels that are used to notify the dispatcher when new data
    // is available. When an event happens for a given key, the key name shall be sent
    // over its channel: the dispatcher will read that key and update the plot.
    static ref BOUND_KEYS: Mutex<HashMap<String, Vec<glib::Sender<String>>>> = Mutex::new(HashMap::new());
}

#[derive(Clone, Debug)]
struct DrawParams {
    lists: Vec<String>,
    width: usize,
    height: usize,
    target: String,
    /// Shall the window be opened upon binding
    open: bool,
}

// TODO implement TryFrom for DrawParams, possibly dropping parse_args.
/// Parse the arguments and return DrawParams.
fn draw_params_try_from(args: Vec<RedisString>) -> Result<DrawParams, RedisError> {
    // Parse the arguments into a map. Checks on arity will be performed later.
    let args = parse_args(args.into_iter().map(|s| s.to_string()).collect());

    // This command accept lists where to get data
    let lists = args
        .get("--list")
        .ok_or(RedisError::Str("Missing mandatory --list argument"))?
        .clone();

    if lists.is_empty() {
        return Err(RedisError::WrongArity);
    }

    Ok(DrawParams {
        lists,
        width: 400,
        height: 300,
        target: "out_win".to_string(),
        open: false,
    })
}

fn plot_spec_try_from(ctx: &Context, args: DrawParams) -> Result<PlotSpec, RedisError> {
    let DrawParams {
        lists,
        width: _,
        height: _,
        target: _,
        open: _,
    } = args;

    // Read all the data for all the lists
    let mut ld = vec![];
    for l in lists.into_iter() {
        // Get the data for this list
        let d = ctx.call("LRANGE", &[&l, "0", "-1"])?;

        // Ensure it's an array and extract data from it.
        if let RedisValue::Array(els) = d {
            let data: Vec<(f32, f32)> = els
                .into_iter()
                .enumerate()
                .filter_map(|(i, v)| match v {
                    // Try to parse the string, return None (ignore value) on fail.
                    RedisValue::SimpleString(v) => Some((i as f32, v.parse::<f32>().ok()?)),
                    // Note we are using f32 for points, there might be loss of precision.
                    RedisValue::Integer(v) => Some((i as f32, v as f32)),
                    RedisValue::Float(v) => Some((i as f32, v as f32)),
                    _ => None,
                })
                .collect();

            // Get the data
            ld.push(data);
        } else {
            return Err(RedisError::String(format!("{} is not an array", l)));
        }
    }

    Ok(PlotSpec {
        color: (0..ld.len()).map(|i| COLORS[i % COLORS.len()]).collect(),
        data: ld,
        bg_color: (0xff, 0xff, 0xff),
    })
}

/// This is the primitive function for rsp: it takes all the arguments necessary
/// to determine what to plot, where and how. Returns a (redis) string with the binary
/// image with the plot.
/// Arguments are passed using dashed notation, for example `--foo 1 2 3 --bar` is
/// valid under this notation.
/// Currently accepts a `--list key+` argument, followed by one or more redis keys
/// containing a list of integers or floats.
/// Color is currently fixed to cyclic-RGB for curves, same as output size of the image,
/// grid, ticks and background colors.
fn rsp_draw(ctx: &Context, args: Vec<RedisString>) -> RedisResult {
    // Parse arguments
    let args = draw_params_try_from(args)?;
    let w = args.width;
    let h = args.height;

    // Prepare the data for plotting
    let sp = plot_spec_try_from(ctx, args)?;

    // Plot the data on a buffer bitmap
    // TODO read size from args.
    let mut buf: Vec<u8> = vec![0; w * h * 3];
    let root = BitMapBackend::with_buffer(&mut buf, (w as u32, h as u32)).into_drawing_area();

    plot_complex(root, sp);

    // Send back the data, prepending encoded width and height
    Ok(w.to_be_bytes()
        .into_iter()
        .chain(h.to_be_bytes().into_iter())
        .chain(buf.into_iter())
        .collect::<Vec<u8>>()
        .into())
}

/// Binds one or more list keys to a new plot window. Whenever the keys get updated
/// after the binding, the plot will be updated as well. This accepts the same arguments
/// as rsp_draw.
fn rsp_bind(_ctx: &Context, args: Vec<RedisString>) -> RedisResult {
    let args = draw_params_try_from(args)?;

    // Whenever a bind happens, send the specification to the dispatcher.
    if let Some(tx) = DISPATCHER_TX
        .lock()
        .expect("Cannot lock dispatcher")
        .as_ref()
    {
        tx.send(args.clone()).expect("Cannot send to dispatcher");
    }

    Ok(().into())
}

fn on_list_event(_ctx: &Context, ev_type: NotifyEvent, event: &str, key: &str) -> RedisResult {
    println!("Some list event! {:?} {} {}", ev_type, event, key);
    // Check if the key that generated the event was bound to something.
    if BOUND_KEYS.lock().unwrap().contains_key(key) {
        println!("This key '{}' was watched, plotting!", key);

        // There might be multiple plots bound to this key
        let binding_args = BOUND_KEYS.lock().unwrap().get(key).unwrap().clone();
        for tx in binding_args.into_iter() {
            tx.send(key.to_string()).expect("Cannot send key");
        }
    }
    Ok(().into())
}

/// This is an echo function used for testing
fn rsp_echo(_: &Context, args: Vec<RedisString>) -> RedisResult {
    println!("Called rsp.echo");
    if args.len() < 2 {
        return Err(RedisError::WrongArity);
    }

    Ok(RedisValue::from(
        args.into_iter()
            .map(|rs| rs.to_string())
            .collect::<Vec<String>>()
            .join(", "),
    ))
}

fn init_rsp(ctx: &Context, args: &[RedisString]) -> Status {
    if args.len() == 1 {
        ctx.log(LogLevel::Notice, "Plotting to file");
    } else {
        // println!("LOADING MODULE WITH ARGS {:?}", _args);
        std::thread::spawn(|| {
            let app = gtk4::Application::builder()
                .application_id("re.ale.RedisPlot")
                .build();

            // TODO support async render on files, not just windows.

            // Activation might be called multiple times, e.g. after being hidden
            app.connect_activate(|app| {
                let bind_tx = {
                    use glib::{source::PRIORITY_DEFAULT, MainContext};
                    // Setup communication channel, this is unique for the UI being built.
                    let (tx, rx) = MainContext::channel(PRIORITY_DEFAULT);
                    rx.attach(None, {
                        let app_clone = app.downgrade();
                        move |args: DrawParams| {
                            if let Some(app) = app_clone.upgrade() {
                                use std::cell::RefCell;
                                use std::rc::Rc;

                                // This is the plot model, it contains all the
                                // data necessary to plot something
                                let plot_model: Rc<RefCell<Vec<(f32, f32)>>> =
                                    Rc::new(RefCell::new(vec![]));

                                let drawing_area = gtk4::DrawingArea::new();

                                let win = gtk4::Window::builder()
                                    .application(&app)
                                    .default_width(args.width as i32)
                                    .default_height(args.height as i32)
                                    .title(args.target.as_str())
                                    .child(&drawing_area)
                                    .build();

                                // We use the plot model as source of truth when plotting.
                                // It is the only dependency for the drawing function.
                                let plot_model_clone = plot_model.clone();
                                drawing_area.set_draw_func(move |_, cr, w, h| {
                                    use plotters::prelude::*;
                                    let root =
                                        plotters_cairo::CairoBackend::new(cr, (w as u32, h as u32))
                                            .into_drawing_area();

                                    let data = plot_model_clone.borrow().to_vec();
                                    plot_stuff(root, data);
                                });

                                // Setup communication channel, this is unique for the UI being built.
                                let (plot_tx, plot_rx) = MainContext::channel(PRIORITY_DEFAULT);
                                plot_rx.attach(None, {
                                    let drawing_area = drawing_area.downgrade();
                                    let win = win.downgrade();
                                    move |list_key: String| {
                                        println!("LIST KEY received: {}", list_key);

                                        // Get access to redis data
                                        // TODO there should be a more efficient access method.
                                        use redis_module::ThreadSafeContext;
                                        let thread_ctx = ThreadSafeContext::new();

                                        let mut data = {
                                            let ctx = thread_ctx.lock();

                                            let els = ctx
                                                .call("LRANGE", &[&list_key, "0", "-1"])
                                                .expect("Cannot lrange");
                                            if let RedisValue::Array(els) = els {
                                                let data: Vec<(f32, f32)> = els
                                                    .into_iter()
                                                    .enumerate()
                                                    .filter_map(|(i, v)| match v {
                                                        // FIXME this unwrap shall be changed into a None
                                                        RedisValue::SimpleString(v) => Some((
                                                            i as f32,
                                                            v.parse::<f32>().unwrap(),
                                                        )),
                                                        RedisValue::Integer(v) => {
                                                            Some((i as f32, v as f32))
                                                        }
                                                        RedisValue::Float(v) => {
                                                            Some((i as f32, v as f32))
                                                        }
                                                        _ => None,
                                                    })
                                                    .collect();
                                                data
                                            } else {
                                                // list_key was not found!
                                                vec![]
                                            }
                                        };

                                        plot_model.borrow_mut().clear();
                                        plot_model.borrow_mut().append(&mut data);

                                        if let Some(drawing_area) = drawing_area.upgrade() {
                                            drawing_area.queue_draw();
                                        } else {
                                            println!("Drawing area cannot be upgraded!");
                                        }
                                        // Present upon plot update
                                        if !args.open {
                                            if let Some(win) = win.upgrade() {
                                                println!("Presenting window!");
                                                win.present();
                                            } else {
                                                println!("Window cannot be upgraded!");
                                            }
                                        }
                                        Continue(true)
                                    }
                                });

                                // When presenting immediately
                                if args.open {
                                    win.present();
                                }

                                args.lists.iter().for_each(|k| {
                                    BOUND_KEYS
                                        .lock()
                                        .unwrap()
                                        .entry(k.to_string())
                                        .or_insert(vec![])
                                        .push(plot_tx.clone());
                                });
                            } else {
                                println!("Application cannot be upgraded!");
                            }

                            Continue(true)
                        }
                    });
                    tx
                };

                // Salvalo da qualche parte
                let _ = DISPATCHER_TX.lock().expect("LOCK FALLITO").insert(bind_tx);

                // FIXME if this win gets commented out, no windows will appear at all.
                // This suggests me that references get lost and weak pointers cannot
                // upgrade.
                let win = gtk4::Window::builder()
                    .application(app)
                    .default_width(400)
                    .default_height(300)
                    .title("Boh")
                    .build();
                win.present();
            });

            app.run_with_args::<&str>(&[]);
        });
    }

    ctx.log(LogLevel::Warning, "Initializing rsp!");
    Status::Ok
}

fn deinit_rsp(ctx: &Context) -> Status {
    ctx.log(LogLevel::Warning, "DE-initializing rsp!");
    Status::Ok
}

redis_module! {
    name: "redis_plot",
    version: 1,
    data_types: [],
    init: init_rsp,
    deinit: deinit_rsp,
    commands: [
        // x are assumed to be natural numbers, but a flag might be used to load them from a list.
        ["rsp.draw", rsp_draw, "", 0, 0, 0],
        ["rsp.bind", rsp_bind, "", 0, 0, 0],
        ["rsp.echo", rsp_echo, "", 0, 0, 0],
        // rsp.line
        // rsp.dots
        // rsp.bars a bar for each y-value. more keys will produce side-to-side bars, flag to stack
        // rsp.hist
    ],
    event_handlers: [
        [@LIST: on_list_event]
    ]
}
