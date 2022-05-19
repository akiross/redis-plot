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

// TODO this must change (to a map?) and allow to send data to different targets, so
// we can update different windows and/or images.
//static mut CHAN_TX: Option<glib::Sender<String>> = None;
const COLORS: &[(u8, u8, u8)] = &[(0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0x00, 0x00, 0xff)];

lazy_static! {
    static ref BOUND_L: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    // Maps redis keys to vectors of drawing parameters, which are later used for rendering.
    // TODO the vec could become a list of indices/references into a vector of DrawParams
    // to avoid duplicating the arguments.
    static ref BOUND_KEYS: Mutex<HashMap<String, Vec<glib::Sender<String>>>> = Mutex::new(HashMap::new());
    //static ref BOUND_NAMES: Mutex<HashMap<String, DrawParams>> = Mutex::new(HashSet::new());
    // static ref WINDOWS_TX: Mutex<HashMap<String, glib::Sender<String>>> = Mutex::new(HashMap::new());


    // Questo è il canale tramite cui si mandano le cose al dispatcher. Viene usato
    // da rsp.bind per istruire la GUI che vogliamo un nuovo target di rendering.
    static ref DISPATCHER_TX: Mutex<Option<glib::Sender<DrawParams>>> = Mutex::new(None);
}

#[derive(Clone, Debug)]
struct DrawParams {
    lists: Vec<String>,
    width: usize,
    height: usize,
    target: String,
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
    })
}

fn plot_spec_try_from(ctx: &Context, args: DrawParams) -> Result<PlotSpec, RedisError> {
    let DrawParams {
        lists,
        width: _,
        height: _,
        target: _,
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

/// Binds a list name to a plot. Whenever the list gets updated, after the binding,
/// the plot will be updated. This accepts the same argument as rsp_draw.
// The new rsp_bind parses the args immediately, stores the parsed struct.
// On list event, the parsed struct will be used to read the data from redis
// and plot it.
fn rsp_bind(_: &Context, args: Vec<RedisString>) -> RedisResult {
    let args = draw_params_try_from(args)?;

    // Quando avviene il bind, mandiamo un messaggio con le specifiche al
    // dispatcher
    if let Some(tx) = DISPATCHER_TX.lock().expect("LOCK FALLITO").as_ref() {
        tx.send(args.clone()).expect("SEND FALLITOH");
    }

    // Aggiungo un clone di args per ogni key che potrebbe essere aggiornata.
    /*
    args.lists.iter().for_each(|k| {
        BOUND_KEYS
            .lock()
            .unwrap()
            .entry(k.to_string())
            .or_insert(vec![])
            .push(args.clone());
    });
    */

    Ok(().into())
}

fn on_list_event(ctx: &Context, ev_type: NotifyEvent, event: &str, key: &str) -> RedisResult {
    println!("Some list event! {:?} {} {}", ev_type, event, key);
    // Check if the key that generated the event was bound to something.
    if BOUND_KEYS.lock().unwrap().contains_key(key) {
        println!("This key '{}' was watched, plotting!", key);

        // There might be multiple plots bound to this key
        let binding_args = BOUND_KEYS.lock().unwrap().get(key).unwrap().clone();
        for tx in binding_args.into_iter() {
            tx.send(key.to_string()).expect("Cannot send key");
            /*
            if false {
                // Draw on image
                let w = args.width;
                let h = args.height;
                let mut buf: Vec<u8> = vec![0; w * h * 3];
                let root =
                    BitMapBackend::with_buffer(&mut buf, (w as u32, h as u32)).into_drawing_area();

                println!("Ok, root is ready, drawing...");

                let sp = plot_spec_try_from(ctx, args)?;
                plot_complex(root, sp);
            } else {
                // Draw on window
                // TODO use the args to select the target window
                // FIXME all the block is unsafe, not great...
                WINDOWS_TX
                    .lock()
                    .unwrap()
                    .get("window")
                    .unwrap()
                    .send(key.to_string());
                //unsafe {
                //    CHAN_TX.as_ref().map(|ch| ch.send(key.to_string()));
                //}
            }
            */
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

            // TODO GTK might render to a window or to a file.
            // TODO open the window only when needed.
            // TODO multiple windows are necessary.

            // Activation might be called multiple times, e.g. after being hidden
            app.connect_activate(|app| {
                // costruisci il dispatcher
                let bind_tx = {
                    use glib::clone;
                    use glib::{source::PRIORITY_DEFAULT, MainContext};
                    // Setup communication channel, this is unique for the UI being built.
                    let (tx, rx) = MainContext::channel(PRIORITY_DEFAULT);
                    rx.attach(None, {
                        let app_clone = app.downgrade();
                        move |args: DrawParams| {
                            println!("RICEVUTI DATI: {:?}", args);

                            if let Some(app) = app_clone.upgrade() {
                                use std::cell::RefCell;
                                use std::rc::Rc;

                                let plot_spec: Rc<RefCell<Vec<(f32, f32)>>> =
                                    Rc::new(RefCell::new(vec![]));

                                let plot_spec_clone = plot_spec.clone();
                                let drawing_area = gtk4::DrawingArea::new();

                                drawing_area.set_draw_func(move |_, cr, w, h| {
                                    use plotters::prelude::*;
                                    //use redis_module::ThreadSafeContext;

                                    let root =
                                        plotters_cairo::CairoBackend::new(cr, (w as u32, h as u32))
                                            .into_drawing_area();

                                    let data = plot_spec_clone.borrow().to_vec();
                                    println!("Ora plotto!");
                                    plot_stuff(root, data);
                                });

                                // let list_box = gtk4::ListBox::new();
                                // list_box.append(&gtk4::Label::new(Some("999".into())));

                                // Setup communication channel, this is unique for the UI being built.
                                let (plot_tx, plot_rx) = MainContext::channel(PRIORITY_DEFAULT);
                                plot_rx.attach(None, {
                                    let drawing_area = drawing_area.downgrade();
                                    move |list_key: String| {
                                        println!("LIST KEY received: {}", list_key);

                                        // Get access to redis data
                                        use redis_module::ThreadSafeContext;
                                        let thread_ctx = ThreadSafeContext::new();

                                        let mut data = {
                                            let ctx = thread_ctx.lock();

                                            // if let Some(spec) = plot_spec_clone.borrow().as_deref() {
                                            println!("Disegno usando la spec {}", list_key);
                                            let els = ctx
                                                .call("LRANGE", &[&list_key, "0", "-1"])
                                                .expect("Cannot lrange");
                                            // println!("Collecting RSP {:?}", els);
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

                                        plot_spec.borrow_mut().clear();
                                        plot_spec.borrow_mut().append(&mut data);

                                        if let Some(drawing_area) = drawing_area.upgrade() {
                                            drawing_area.queue_draw();
                                        }
                                        Continue(true)
                                    }
                                });

                                let win = gtk4::Window::builder()
                                    .application(&app)
                                    .default_width(400)
                                    .default_height(300)
                                    .title(args.target.as_str())
                                    .child(&drawing_area)
                                    .build();

                                // Forse conviene gestire in qualche modo queste
                                // finestre in modo da fare present solo quando arrivano
                                // i dati ed eventualmente aprirle a comando
                                win.present();

                                args.lists.iter().for_each(|k| {
                                    BOUND_KEYS
                                        .lock()
                                        .unwrap()
                                        .entry(k.to_string())
                                        .or_insert(vec![])
                                        .push(plot_tx.clone());
                                });

                                // Salva plot_tx da qualche parte per quando
                                // arrivano i dati da plottare
                                // TODO questa potrebbe essere come su nuova_lib.rs
                            }

                            // let label = gtk4::Label::new(Some(&message));
                            // list_box.append(&label);
                            //
                            // Quando arriva una nuova spec (stringa per ora) la si salva per l'uso
                            // plot_spec.replace(Some(message));
                            //
                            /*
                                 * TODO deve essere messa qui la creazione della finestra
                            // We create the main window.
                            let win = ApplicationWindow::builder()
                                .application(app)
                                .default_width(320)
                                .default_height(200)
                                .title("RSP")
                                .child(&drawing_area)
                                .build();
                                // Don't forget to make all widgets visible.
                                win.present();
                                */

                            // drawing_area.queue_draw();
                            Continue(true)
                        }
                    });
                    tx
                };

                // Salvalo da qualche parte
                let _ = DISPATCHER_TX.lock().expect("LOCK FALLITO").insert(bind_tx);

                // questa cosa è assurda: se tolgo questa finestra, allora non riesco
                // più ad aprire quelle successive
                let win = gtk4::Window::builder()
                    .application(app)
                    .default_width(400)
                    .default_height(300)
                    .title("Boh")
                    .build();
                win.present();

                // let tx = build_ui(app);
                // Save the channel so we can send data to it
                // SAFETY: FIXME, this *should* happen once, but it might not, since
                // activate is a Fn; so, CHAN_TX might be set multiple times and
                // the on_list_event might be writing on this. A mutex might be
                // appropriate
                /*
                unsafe {
                    CHAN_TX = Some(tx);
                }
                */

                // let binding_args = BOUND_KEYS.lock().unwrap().get(key).unwrap().clone();
                // WINDOWS_TX.lock().unwrap().insert("window".to_string(), tx);
                //: Mutex<HashMap<String, glib::Sender<String>>> = Mutex::new(HashMap::new());
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
