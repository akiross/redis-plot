use redis_module::{
    //Context, KeyType, LogLevel, RedisError, RedisResult, RedisString,
    RedisValue,
    //Status,
    ThreadSafeContext,
};

use glib::{source::PRIORITY_DEFAULT, MainContext};
//use gtk4::prelude::*;
//use gtk4::Application;
use glib::clone;
//use glib::Continue;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};

//static mut CHAN_TX: Option<glib::Sender<String>> = None;
//static mut OUT_FILE: Option<std::path::PathBuf> = None;

// This is the function that performs the plotting for different backends.
pub fn plot_stuff<DB>(
    root: plotters::drawing::DrawingArea<DB, plotters::coord::Shift>,
    data: Vec<(f32, f32)>,
) where
    DB: plotters_backend::DrawingBackend,
{
    use plotters::prelude::*;
    root.fill(&WHITE).unwrap();
    //let root = root.margin(25, 25, 25, 25);

    let (x_range, y_range) = {
        let mut x_min: Option<f32> = None;
        let mut x_max: Option<f32> = None;
        let mut y_min: Option<f32> = None;
        let mut y_max: Option<f32> = None;

        for (x, y) in data.iter() {
            if x < x_min.get_or_insert(*x) {
                x_min.replace(*x);
            }
            if x > x_max.get_or_insert(*x) {
                x_max.replace(*x);
            }
            if y < y_min.get_or_insert(*y) {
                y_min.replace(*y);
            }
            if y > y_max.get_or_insert(*y) {
                y_max.replace(*y);
            }
        }
        if x_min.is_none() || x_max.is_none() || y_min.is_none() || y_max.is_none() {
            (0.0..0.0, 0.0..0.0)
        } else {
            (
                x_min.unwrap()..x_max.unwrap(),
                y_min.unwrap()..y_max.unwrap(),
            )
        }
    };

    let mut chart = ChartBuilder::on(&root)
        .margin(25i32)
        .x_label_area_size(30)
        .y_label_area_size(30)
        .caption("RSPlotters", ("sans-serif", 20u32))
        .build_cartesian_2d(x_range, y_range)
        .unwrap();

    chart.configure_mesh().draw().unwrap();

    chart.draw_series(LineSeries::new(data, &RED)).unwrap();
}

// The received MUST be passed by value.
pub fn build_ui(app: &gtk4::Application) -> glib::Sender<String> {
    // Setup drawing area
    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.set_draw_func(|_, cr, w, h| {
        use plotters::prelude::*;
        let root = plotters_cairo::CairoBackend::new(cr, (w as u32, h as u32)).into_drawing_area();

        // Get access to redis data
        let thread_ctx = ThreadSafeContext::new();

        let data = {
            let ctx = thread_ctx.lock();
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

            let els = ctx
                .call("LRANGE", &["rsp", "0", "-1"])
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
                data
            } else {
                // println!("No rsp list found");
                vec![]
            }
        };
        // println!("Collected data {:?}", data);

        plot_stuff(root, data);
    });

    // let list_box = gtk4::ListBox::new();
    // list_box.append(&gtk4::Label::new(Some("999".into())));

    // Setup communication channel, this is unique for the UI being built.
    let (tx, rx) = MainContext::channel(PRIORITY_DEFAULT);
    rx.attach(
        None,
        clone!(@weak drawing_area => @default-return Continue(false),
        move |_message| {
            println!("DATA RECEIVED: {}", _message);
            // let label = gtk4::Label::new(Some(&message));
            // list_box.append(&label);

            drawing_area.queue_draw();
            Continue(true)
        }),
    );

    // let scrolled = gtk4::ScrolledWindow::builder()
    //     .hscrollbar_policy(gtk4::PolicyType::Never)
    //     .child(&list_box)
    //     .build();

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

    // Return the transmission end, so it can be used to send stuff to this UI.
    tx
}

/*
pub fn start_ui() {
    let app = Application::builder()
        .application_id("re.ale.RedisPlot")
        .build();

    //app.connect_activate(build_ui);

    app.run_with_args::<&str>(&[]);
}
*/

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
