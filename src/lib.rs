/// This module gets built and becomes a cdylib .so file to be loaded in redis and can be
/// used to plot data from redis. Can be used from a redis client to plot data on demand
/// or as a watcher, that automatically updates the plot when events change the data.
///
/// The module can plot directly over a buffer as a RGB image - which can be read from the
/// redis client and displayed as preferred - or output to a device (i.e. screen or file).
//
// TODO look at https://github.com/RedisLabsModules/redismodule-rs/blob/master/examples/lists.rs
// for a more efficient access to key data.

#[macro_use]
extern crate redis_module;

#[macro_use]
extern crate lazy_static;

use redis_module::ThreadSafeContext;
use redis_module::{
    Context, LogLevel, NotifyEvent, RedisError, RedisResult, RedisString, RedisValue, Status,
};

use gtk4::prelude::*;

use plotters::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

mod argparse;
mod launcher;
mod utils;

use launcher::{build_gtk_gui, DrawParams, APP_QUIT, BOUND_KEYS, DISPATCHER_TX};

use argparse::parse_args;
use utils::{plot_complex, PlotSpec};

const COLORS: &[(u8, u8, u8)] = &[(0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0x00, 0x00, 0xff)];

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

    if let Some(tx) = DISPATCHER_TX.lock()?.as_ref() {
        tx.send(args.clone())?;
    }

    Ok(().into())
}

fn on_list_event(_ctx: &Context, ev_type: NotifyEvent, event: &str, key: &str) -> RedisResult {
    println!("Some list event! {:?} {} {}", ev_type, event, key);

    // Check if the key that generated the event was bound to something.
    if let Ok(guard) = BOUND_KEYS.lock() {
        if let Some(binding_args) = guard.get(key) {
            println!("This key '{}' was watched, plotting!", key);

            // There might be multiple plots bound to this key
            let binding_args = binding_args.clone();
            for tx in binding_args.into_iter() {
                tx.send(key.to_string())?;
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
    warn!("Initializing rsp....");

    if args.len() == 1 {
        info!("Plotting to file");
    } else {
        // println!("LOADING MODULE WITH ARGS {:?}", _args);
        std::thread::spawn(|| {
            build_gtk_gui();
        });
    }

    info!("redis_plot initialized");
    Status::Ok
}

fn deinit_rsp(ctx: &Context) -> Status {
    info!("De-initializing redis_plot");

    if let Ok(guard) = APP_QUIT.lock() {
        if let Some(tx) = guard.as_ref() {
            debug!("Sending term signal...");
            if let Err(_e) = tx.send(()) {
                //warn!("Cannot send termination signal");
                ctx.reply_error_string("Cannot send termination signal");
                return Status::Err;
            }
        } else {
            warn!("APP_QUIT has no value, initialization might have failed.");
        }
    } else {
        ctx.reply_error_string("Cannot lock APP_QUIT");
        return Status::Err;
    }

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
