use redis_module::ThreadSafeContext;
use redis_module::{
    Context, LogLevel, NotifyEvent, RedisError, RedisResult, RedisString, RedisValue, Status,
};

use gtk4::prelude::*;

use glib::{source::PRIORITY_DEFAULT, MainContext};
use plotters::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

use tracing::{debug, info, warn};

use std::cell::RefCell;
use std::rc::Rc;
//mod argparse;
//mod utils;

//use argparse::parse_args;
use crate::utils::plot_stuff;

lazy_static::lazy_static! {
    // This is the channel to send drawing arguments to the dispatcher when a binding
    // command arrives, the dispatcher will set up the window and build a channel to
    // notify when keys are updated. That channel will be added in BOUND_KEYS.
    pub static ref DISPATCHER_TX: Mutex<Option<glib::Sender<DrawParams>>> = Mutex::new(None);

    // Maps redis keys to channels that are used to notify the dispatcher when new data
    // is available. When an event happens for a given key, the key name shall be sent
    // over its channel: the dispatcher will read that key and update the plot.
    pub static ref BOUND_KEYS: Mutex<HashMap<String, Vec<glib::Sender<String>>>> = Mutex::new(HashMap::new());

    // This is a channel that is used to signal when the app shall terminate.
    pub static ref APP_QUIT: Mutex<Option<glib::Sender<()>>> = Mutex::new(None);
}

#[derive(Clone, Debug)]
pub struct DrawParams {
    pub lists: Vec<String>,
    pub width: usize,
    pub height: usize,
    pub target: String,
    /// Shall the window be opened upon binding
    // FIXME pick a better name: the choice is either present it once or on update
    // FIXME it actually might be two separated options...
    pub open: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum Errors {
    #[error("Unable to call LRANGE to read key data")]
    CannotCallLrange,
    #[error("Unable to lock mutex")]
    CannotLockMutex,
}

fn get_key_data(list_key: String) -> Result<Vec<(f32, f32)>, Errors> {
    let thread_ctx = ThreadSafeContext::new();
    println!("DATA-CTX locking...");
    let ctx = thread_ctx.lock();
    println!("DATA-CTX locked!");

    let els = ctx
        .call("LRANGE", &[&list_key, "0", "-1"])
        .map_err(|_| Errors::CannotCallLrange)?;

    if let RedisValue::Array(els) = els {
        let data: Vec<(f32, f32)> = els
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| match v {
                // Assuming that if a string cannot be converted to f32, then it's ok to ignore it
                RedisValue::SimpleString(v) => v.parse::<f32>().ok().map(|v| (i as f32, v)),
                RedisValue::Integer(v) => Some((i as f32, v as f32)),
                RedisValue::Float(v) => Some((i as f32, v as f32)),
                _ => None,
            })
            .collect();
        Ok(data)
    } else {
        // list_key was not found!
        Ok(vec![])
    }
}

fn on_connect_activate(app: &gtk4::Application) {
    // Setup communication channel, this is unique for the UI being built.
    let (bind_tx, bind_rx) = MainContext::channel(PRIORITY_DEFAULT);
    bind_rx.attach(None, {
        let app_clone = app.downgrade();
        move |args: DrawParams| {
            if let Some(app) = app_clone.upgrade() {
                // This is the plot model, it contains all the
                // data necessary to plot something
                let plot_model: Rc<RefCell<Vec<(f32, f32)>>> = Rc::new(RefCell::new(vec![]));

                let drawing_area = gtk4::DrawingArea::new();

                let win = gtk4::Window::builder()
                    .application(&app)
                    .default_width(args.width as i32)
                    .default_height(args.height as i32)
                    .hide_on_close(true) // To open it on render
                    .title(args.target.as_str())
                    .child(&drawing_area)
                    .build();

                // We use the plot model as source of truth when plotting.
                // It is the only dependency for the drawing function.
                let plot_model_clone = plot_model.clone();
                drawing_area.set_draw_func(move |_, cr, w, h| {
                    use plotters::prelude::*;
                    let root = plotters_cairo::CairoBackend::new(cr, (w as u32, h as u32))
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
                        let mut data = match get_key_data(list_key) {
                            Ok(data) => data,
                            Err(_) => {
                                warn!("Cannot get data, skipping");
                                return Continue(true);
                            }
                        };
                        println!("DATA-CTX unlocked");

                        plot_model.borrow_mut().clear();
                        plot_model.borrow_mut().append(&mut data);

                        if let Some(drawing_area) = drawing_area.upgrade() {
                            drawing_area.queue_draw();
                        } else {
                            warn!("Drawing area cannot be upgraded!");
                        }
                        // Present upon plot update
                        if !args.open {
                            if let Some(win) = win.upgrade() {
                                println!("Presenting window!");
                                win.present();
                            } else {
                                warn!("Window cannot be upgraded!");
                            }
                        }
                        Continue(true)
                    }
                });

                // When presenting immediately
                if args.open {
                    win.present();
                }

                if let Err(e) = args
                    .lists
                    .iter()
                    .map(|k| match BOUND_KEYS.lock() {
                        Ok(mut guard) => {
                            guard
                                .entry(k.to_string())
                                .or_insert(vec![])
                                .push(plot_tx.clone());
                            Ok(())
                        }
                        _ => Err(Errors::CannotLockMutex),
                    })
                    .collect::<Result<Vec<()>, Errors>>()
                {
                    warn!("ERRORE!");
                }
            } else {
                println!("Application cannot be upgraded!");
            }

            Continue(true)
        }
    });

    // Save the trasmitter somewhere
    if let Ok(mut guard) = DISPATCHER_TX.lock() {
        let _ = guard.insert(bind_tx);
    } else {
        let ctx = ThreadSafeContext::new().lock();
        ctx.reply_error_string("Cannot lock DISPATCHER_TX");
    }
}

/// Builds a GTK GUI that relies on
pub fn build_gtk_gui() {
    let app = gtk4::Application::builder()
        .application_id("re.ale.RedisPlot")
        .build();

    // The app will be held until a termination signal arrives, so we can have it
    // running even when there are no windows. This requires a release.
    app.hold();
    let (app_tx, app_rx) = MainContext::channel(PRIORITY_DEFAULT);
    app_rx.attach(None, {
        let app = app.downgrade();
        move |_| {
            if let Some(app) = app.upgrade() {
                // Stop holding the app
                app.release();
                // FIXME the lock() method below seems to stall.
                // Send message
                // let thread_ctx = ThreadSafeContext::new();
                // println!("Locking...");
                // let ctx = thread_ctx.lock();
                // println!("Locked!");
                // warn!("Application released");
                // println!("Done writing");
            } else {
                warn!("Cannot upgrade application pointer!");
            }
            Continue(true)
        }
    });

    if let Ok(mut guard) = APP_QUIT.lock() {
        let _ = guard.insert(app_tx);
    } else {
        let ctx = ThreadSafeContext::new().lock();
        ctx.reply_error_string("Cannot lock APP_QUIT");
    }

    // Debug some events
    app.connect_window_added(|_, _| {
        debug!("Window added to application");
    });
    app.connect_window_removed(|_, _| {
        debug!("Window removed from application");
    });

    // TODO support async render on files, not just windows.

    // Activation might be called multiple times, e.g. after being hidden
    app.connect_activate(on_connect_activate);

    app.run_with_args::<&str>(&[]);
}
