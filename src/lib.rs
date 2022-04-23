/// This module gets built and becomes a cdylib .so file to be loaded in redis and can be
/// used to plot data from redis. Can be used from a redis client to plot data on demand
/// or as a watcher, that automatically updates the plot when events change the data.
///
/// The module can plot directly over a buffer as a RGB image - which can be read from the
/// redis client and displayed as preferred - or output to a device (i.e. screen or file).

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
use std::collections::HashSet;
use std::sync::Mutex;

mod utils;

use utils::{build_ui, plot_stuff};

/*
struct MyApplication {
    tx: Option<glib::Sender<String>>,
    rx: Option<glib::Receiver<String>>,
    out_files: Vec<std::path::PathBuf>,
}

impl MyApplication {
    fn new() -> Self {
        MyApplication {
            tx: None,
            rx: None,
            out_files: vec![],
        }
    }
    /// Sets up a communication channel, returning RX.
    fn build_chan(&mut self) -> glib::Receiver<String> {
        let (tx, rx) = glib::MainContext::channel(PRIORITY_DEFAULT);
        rx
    }

    unsafe fn get_tx(&self) -> glib::Sender<String> {
        let (tx, rx) = glib::MainContext::channel(PRIORITY_DEFAULT);
        tx
    }
}

lazy_static! {
    static ref STATE: MyApplication = MyApplication::new();
}
*/

// TODO this must change (to a map?) and allow to send data to different targets, so
// we can update different windows and/or images.
static mut CHAN_TX: Option<glib::Sender<String>> = None;

lazy_static! {
    static ref BOUND_L: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

/// Binds a list name to a plot. Whenever the list gets updated, after the binding,
/// the plot will be updated. A single argument is needed, with the name of the list.
// TODO in future, this will also accept a plot type (scatter, lines, histogram...)
// and a drawing target (window, image).
fn rsp_bind(_: &Context, args: Vec<RedisString>) -> RedisResult {
    if args.len() < 2 {
        return Err(RedisError::WrongArity);
    }

    let list_name = args
        .into_iter()
        .skip(1)
        .next_arg()
        .expect("BUG, list_name arg was not counted properly");

    // Bind the list name so we will watch it
    BOUND_L.lock().unwrap().insert(list_name.to_string());
    println!("List {} bound", list_name);

    Ok(().into())
}

fn on_list_event(ctx: &Context, ev_type: NotifyEvent, event: &str, key: &str) -> RedisResult {
    println!("Some list event! {:?} {} {}", ev_type, event, key);
    if BOUND_L.lock().unwrap().contains(key) {
        // TODO redraw should happen only for the related plot
        println!("A watched list! Triggering redraw!");
        return rsp_draw(ctx, vec![]); // FIXME pass arguments here?
    }
    Ok(().into())
}

/// This is the primitive function for rsp: it takes all the arguments necessary
/// to determine what to plot, where and how. The first argument is mandatory: it's
/// the key name for a list, source of data.
/// TODO The second argument is the kind of plot (e.g. "scatter"), this defaults to "scatter".
/// TODO The third argument specified the draw target: a window name or a string, this defaults to "-" (string).
fn rsp_draw(ctx: &Context, args: Vec<RedisString>) -> RedisResult {
    if args.len() < 2 {
        return Err(RedisError::WrongArity);
    }

    // Parse arguments
    let mut args = args.into_iter().skip(1);
    let key = args.next().expect("BUG!").to_string();
    //let kind = args.next().map_or("scatter".to_string(), |s| s.to_string());
    //let target = args.next().map_or("-".to_string(), |s| s.to_string());

    // Read the entire data (TODO there is likely a better way to do this)
    let els = ctx
        .call("LRANGE", &[key.as_str(), "0", "-1"])
        .expect("Cannot lrange");

    // println!("Collecting RSP {:?}", els);
    if let RedisValue::Array(els) = els {
        let data: Vec<(f32, f32)> = els
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| match v {
                // FIXME this unwrap shall be changed into a None
                RedisValue::SimpleString(v) => Some((i as f32, v.parse::<f32>().unwrap())),
                RedisValue::Integer(v) => Some((i as f32, v as f32)),
                RedisValue::Float(v) => Some((i as f32, v as f32)),
                _ => None,
            })
            .collect();

        // Plot the data on a buffer bitmap
        let (w, h) = (400, 300);
        let mut buf: Vec<u8> = vec![0; w * h * 3];
        let root = BitMapBackend::with_buffer(&mut buf, (w as u32, h as u32)).into_drawing_area();

        plot_stuff(root, data);

        // Send back the data, prepending encoded width and height
        Ok(w.to_be_bytes()
            .into_iter()
            .chain(h.to_be_bytes().into_iter())
            .chain(buf.into_iter())
            .collect::<Vec<u8>>()
            .into())
    } else {
        Err(RedisError::Str("Not an array"))
    }

    /*
    //let tx = STATE.get_tx();
    let tx = unsafe { CHAN_TX.clone() }.expect("TX end was None!");
    dbg!(&tx);
    tx.send(product.to_string()).expect("Cannot send");
    println!("DATA SENT TO CHANNEL!");
    */
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
                let tx = build_ui(app);
                // Save the channel so we can send data to it
                unsafe {
                    CHAN_TX = Some(tx);
                }
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
