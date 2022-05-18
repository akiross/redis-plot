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
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

mod argparse;
mod utils;

use argparse::parse_args;
use utils::{build_ui, plot_complex, PlotSpec};

// TODO this must change (to a map?) and allow to send data to different targets, so
// we can update different windows and/or images.
//static mut CHAN_TX: Option<glib::Sender<String>> = None;
const COLORS: &[(u8, u8, u8)] = &[(0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0x00, 0x00, 0xff)];

lazy_static! {
    static ref BOUND_L: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    // Maps redis keys to vectors of drawing parameters, which are later used for rendering.
    // TODO the vec could become a list of indices/references into a vector of DrawParams
    // to avoid duplicating the arguments.
    static ref BOUND_KEYS: Mutex<HashMap<String, Vec<DrawParams>>> = Mutex::new(HashMap::new());
    //static ref BOUND_NAMES: Mutex<HashMap<String, DrawParams>> = Mutex::new(HashSet::new());
    static ref WINDOWS_TX: Mutex<HashMap<String, glib::Sender<String>>> = Mutex::new(HashMap::new());
}

#[derive(Clone, Debug)]
struct DrawParams {
    lists: Vec<String>,
    width: usize,
    height: usize,
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
    })
}

fn plot_spec_try_from(ctx: &Context, args: DrawParams) -> Result<PlotSpec, RedisError> {
    let DrawParams {
        lists,
        width: _,
        height: _,
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

    // Aggiungo un clone di args per ogni key che potrebbe essere aggiornata.
    args.lists.iter().for_each(|k| {
        BOUND_KEYS
            .lock()
            .unwrap()
            .entry(k.to_string())
            .or_insert(vec![])
            .push(args.clone());
    });

    Ok(().into())
}

fn on_list_event(ctx: &Context, ev_type: NotifyEvent, event: &str, key: &str) -> RedisResult {
    println!("Some list event! {:?} {} {}", ev_type, event, key);
    // Check if the key that generated the event was bound to something.
    if BOUND_KEYS.lock().unwrap().contains_key(key) {
        println!("This key '{}' was watched, plotting!", key);

        // There might be multiple plots bound to this key
        let binding_args = BOUND_KEYS.lock().unwrap().get(key).unwrap().clone();
        for args in binding_args.into_iter() {
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
                let tx = build_ui(app);
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
                WINDOWS_TX.lock().unwrap().insert("window".to_string(), tx);
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
