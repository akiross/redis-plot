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

use redis_module::{
    Context, NotifyEvent, RedisError, RedisResult, RedisString, RedisValue, Status,
};

use plotters::prelude::*;
use tracing::{debug, info, warn};

mod argparse;
mod launcher;
mod utils;

use launcher::{build_gtk_gui, BindParams, APP_QUIT, BOUND_KEYS, DISPATCHER_TX};

use argparse::parse_args;
use utils::{plot_complex, PlotSpec};

const COLORS: &[(u8, u8, u8)] = &[(0xff, 0x00, 0x00), (0x00, 0xff, 0x00), (0x00, 0x00, 0xff)];

// TODO implement TryFrom for DrawParams, possibly dropping parse_args.
/// Parse the arguments and return DrawParams.
fn draw_params_try_from(args: Vec<RedisString>) -> Result<BindParams, RedisError> {
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

    // --width and --height accept a single integer argument
    let width = match args.get("--width") {
        None => Ok(400),
        Some(a) => {
            if a.len() != 1 {
                Err(RedisError::WrongArity)
            } else {
                a[0].parse()
                    .map_err(|_| RedisError::Str("Width cannot be parsed as unsigned int"))
            }
        }
    }?;

    let height = match args.get("--height") {
        None => Ok(300),
        Some(a) => {
            if a.len() != 1 {
                Err(RedisError::WrongArity)
            } else {
                a[0].parse()
                    .map_err(|_| RedisError::Str("Height cannot be parsed as unsigned int"))
            }
        }
    }?;

    let target = match args.get("--target") {
        None => Ok("out_win".to_string()),
        Some(a) => {
            if a.len() != 1 {
                Err(RedisError::WrongArity)
            } else {
                Ok(a[0].clone())
            }
        }
    }?;

    let index = match args.get("--index") {
        None => Ok("natural".to_string()),
        Some(a) => {
            if a.len() != 1 {
                Err(RedisError::WrongArity)
            } else {
                Ok(a[0].clone())
            }
        }
    }?;

    if index == "zip" && lists.len() != 2 {
        return Err(RedisError::Str(
            "zip index needs exactly 2 lists as argument",
        ));
    }

    Ok(BindParams {
        lists,
        width,
        height,
        target,
        index,
        open: false,
    })
}

fn plot_spec_try_from(ctx: &Context, args: &BindParams) -> Result<PlotSpec, RedisError> {
    let BindParams {
        lists,
        width: _,
        height: _,
        target: _,
        index,
        open: _,
    } = args;

    // Depending on the "index", we interpret the source of data in different ways
    let ld = match index.as_str() {
        // When user "zip"s, the indices and the values are taken from 2 lists
        "zip" => {
            let mut ld = vec![];
            let lx = &lists[0];
            let ly = &lists[1];
            let dx = ctx.call("LRANGE", &[&lx, "0", "-1"])?;
            let dy = ctx.call("LRANGE", &[&ly, "0", "-1"])?;

            match (dx, dy) {
                (RedisValue::Array(dx), RedisValue::Array(dy)) => {
                    let data: Vec<(f32, f32)> = std::iter::zip(dx.into_iter(), dy.into_iter())
                        .filter_map(|v| match v {
                            (RedisValue::SimpleString(x), RedisValue::SimpleString(y)) => {
                                Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?))
                            }
                            (RedisValue::Integer(x), RedisValue::Float(y)) => {
                                Some((x as f32, y as f32))
                            }
                            (RedisValue::Float(x), RedisValue::Integer(y)) => {
                                Some((x as f32, y as f32))
                            }
                            (RedisValue::Integer(x), RedisValue::Integer(y)) => {
                                Some((x as f32, y as f32))
                            }
                            (RedisValue::Float(x), RedisValue::Float(y)) => {
                                Some((x as f32, y as f32))
                            }
                            _ => None,
                        })
                        .collect();
                    ld.push(data);
                }
                _ => {
                    return Err(RedisError::String(format!(
                        "{} or {} is not an array",
                        lx, ly
                    )));
                }
            }

            ld
        }
        // When user "xy"s, the indices and the values are interleaved in a list
        // e.g. powers_of_2 = 0 1  1 2  2 4  3 8  4 16  5 32...
        "xy" => {
            todo!("Not done yet!")
        }
        _ => {
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
            ld
        }
    };

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
    let sp = plot_spec_try_from(ctx, &args)?;

    // Plot the data on a buffer bitmap
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

fn init_rsp(_ctx: &Context, args: &[RedisString]) -> Status {
    warn!("Initializing rsp....");

    if args.len() == 1 {
        info!("Plotting to file");
    } else {
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
        ["rsp.draw", rsp_draw, "", 0, 0, 0],
        ["rsp.bind", rsp_bind, "", 0, 0, 0],
        ["rsp.echo", rsp_echo, "", 0, 0, 0],
    ],
    event_handlers: [
        [@LIST: on_list_event]
    ]
}
