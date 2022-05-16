/// This tool produces a GUI that writes values to redis variables.
/// It reads a .slint file with the GUI and automatically binds the values
/// to functions that write the data on redis (write only!)
use redis::Commands;
use slint::Model;
use slint_interpreter::{ComponentCompiler, ComponentHandle, SharedString, Value};
use std::cell::RefCell;
use std::rc::Rc;

// Wrapper, this wraps Value into something that has TryFrom (missing from slint 0.2.4 but
// a patch has landed: waiting the next release to remove this, see slint issue #1258).
#[derive(Clone)]
struct Wrap(Value);

impl TryFrom<Wrap> for i32 {
    type Error = Value;
    fn try_from(v: Wrap) -> Result<i32, Self::Error> {
        v.0.try_into()
    }
}

impl TryFrom<Wrap> for f32 {
    type Error = Value;
    fn try_from(v: Wrap) -> Result<f32, Self::Error> {
        v.0.try_into()
    }
}

impl TryFrom<Wrap> for bool {
    type Error = Value;
    fn try_from(v: Wrap) -> Result<bool, Self::Error> {
        v.0.try_into()
    }
}

impl TryFrom<Wrap> for SharedString {
    type Error = Value;
    fn try_from(v: Wrap) -> Result<SharedString, Self::Error> {
        v.0.try_into()
    }
}

// TODO Use some macros to build these callbacks

// Returns a callback that fooes the bar.
fn make_set_callback<T>(con_clone: Rc<RefCell<redis::Connection>>) -> impl Fn(&[Value]) -> Value
where
    T: redis::ToRedisArgs,
    T: TryFrom<Wrap> + redis::ToRedisArgs,
    <T as TryFrom<Wrap>>::Error: std::fmt::Debug,
{
    move |args: &[Value]| -> Value {
        let key: SharedString = Wrap(args[0].clone())
            .try_into()
            .expect("First argument is not a string");

        let val: T = Wrap(args[1].clone())
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.set(key.as_str(), val).expect("Cannot set");
        Value::from(())
    }
}

// Returns a callback that handles string arguments.
fn make_set_callback_str(con_clone: Rc<RefCell<redis::Connection>>) -> impl Fn(&[Value]) -> Value {
    move |args: &[Value]| -> Value {
        let key: SharedString = args[0]
            .clone()
            .try_into()
            .expect("First argument is not a string");

        let val: SharedString = args[1]
            .clone()
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.set(key.as_str(), val.as_str()).expect("Cannot set");
        Value::from(())
    }
}

fn make_lpush_callback<T>(con_clone: Rc<RefCell<redis::Connection>>) -> impl Fn(&[Value]) -> Value
where
    T: redis::ToRedisArgs,
    T: TryFrom<Wrap> + redis::ToRedisArgs,
    <T as TryFrom<Wrap>>::Error: std::fmt::Debug,
{
    move |args: &[Value]| -> Value {
        let key: SharedString = Wrap(args[0].clone())
            .try_into()
            .expect("First argument is not a string");

        let val: T = Wrap(args[1].clone())
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.lpush(key.as_str(), val).expect("Cannot set");
        Value::from(())
    }
}

// Returns a callback that handles string arguments.
fn make_lpush_callback_str(
    con_clone: Rc<RefCell<redis::Connection>>,
) -> impl Fn(&[Value]) -> Value {
    move |args: &[Value]| -> Value {
        let key: SharedString = args[0]
            .clone()
            .try_into()
            .expect("First argument is not a string");

        let val: SharedString = args[1]
            .clone()
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.lpush(key.as_str(), val.as_str()).expect("Cannot set");
        Value::from(())
    }
}
fn make_rpush_callback<T>(con_clone: Rc<RefCell<redis::Connection>>) -> impl Fn(&[Value]) -> Value
where
    T: redis::ToRedisArgs,
    T: TryFrom<Wrap> + redis::ToRedisArgs,
    <T as TryFrom<Wrap>>::Error: std::fmt::Debug,
{
    move |args: &[Value]| -> Value {
        let key: SharedString = Wrap(args[0].clone())
            .try_into()
            .expect("First argument is not a string");

        let val: T = Wrap(args[1].clone())
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.rpush(key.as_str(), val).expect("Cannot set");
        Value::from(())
    }
}

// Returns a callback that handles string arguments.
fn make_rpush_callback_str(
    con_clone: Rc<RefCell<redis::Connection>>,
) -> impl Fn(&[Value]) -> Value {
    move |args: &[Value]| -> Value {
        let key: SharedString = args[0]
            .clone()
            .try_into()
            .expect("First argument is not a string");

        let val: SharedString = args[1]
            .clone()
            .try_into()
            .expect("Second argument not an f32");

        let mut con = con_clone.borrow_mut();
        let _: () = con.rpush(key.as_str(), val.as_str()).expect("Cannot set");
        Value::from(())
    }
}

fn main() {
    let mut compiler = ComponentCompiler::default();
    let definition = spin_on::spin_on(compiler.build_from_path("hello.slint"));

    slint_interpreter::print_diagnostics(&compiler.diagnostics());

    if let Some(definition) = definition {
        let instance = definition.create();

        // Prepare redis connection
        let client = redis::Client::open("redis://127.0.0.1/").expect("Cannot connect to server");
        // Redis connection cannot be cloned, so we could create a new connection
        // for each callback execution, but it's cumbersome.
        let con = std::rc::Rc::new(std::cell::RefCell::new(
            client.get_connection().expect("Cannot get connection"),
        ));

        // Set callbacks for possible actions
        let con_clone = con.clone();
        instance
            .set_callback("cmd", move |args: &[Value]| -> Value {
                // The name of the command is a SharedString
                let cmd_name: SharedString = args[0].clone().try_into().expect("Not a string");
                // The arguments is a Model<SharedString>, there is no TryFrom for it, so we
                // have to iterate the model and convert each element.
                let cmd_args: Vec<SharedString> = if let Value::Model(modelrc) = args[1].clone() {
                    modelrc
                        .iter()
                        .map(|x| -> SharedString { x.clone().try_into().expect("Not a shastring") })
                        .collect()
                } else {
                    panic!("not a model!");
                };

                let mut con = con_clone.borrow_mut();
                redis::cmd(cmd_name.as_str())
                    .arg(cmd_args.iter().map(|a| a.as_str()).collect::<Vec<_>>())
                    .query::<()>(&mut *con)
                    .expect("Cannot set");

                Value::from(())
            })
            .unwrap_or_else(|_| println!("No set_i32 callback, ignoring"));

        instance
            .set_callback("set_i32", make_set_callback::<i32>(con.clone()))
            .unwrap_or_else(|_| println!("No set_i32 callback, ignoring"));
        instance
            .set_callback("set_f32", make_set_callback::<f32>(con.clone()))
            .unwrap_or_else(|_| println!("No set_f32 callback, ignoring"));
        instance
            .set_callback("set_bool", make_set_callback::<bool>(con.clone()))
            .unwrap_or_else(|_| println!("No set_bool callback, ignoring"));
        instance
            .set_callback("set_string", make_set_callback_str(con.clone()))
            .unwrap_or_else(|_| println!("No set_string callback, ignoring"));

        instance
            .set_callback("lpush_i32", make_lpush_callback::<i32>(con.clone()))
            .unwrap_or_else(|_| println!("No lpush_i32 callback, ignoring"));
        instance
            .set_callback("lpush_f32", make_lpush_callback::<f32>(con.clone()))
            .unwrap_or_else(|_| println!("No lpush_f32 callback, ignoring"));
        instance
            .set_callback("lpush_bool", make_lpush_callback::<bool>(con.clone()))
            .unwrap_or_else(|_| println!("No lpush_bool callback, ignoring"));
        instance
            .set_callback("lpush_string", make_lpush_callback_str(con.clone()))
            .unwrap_or_else(|_| println!("No lpush_string callback, ignoring"));

        instance
            .set_callback("rpush_i32", make_rpush_callback::<i32>(con.clone()))
            .unwrap_or_else(|_| println!("No rpush_i32 callback, ignoring"));
        instance
            .set_callback("rpush_f32", make_rpush_callback::<f32>(con.clone()))
            .unwrap_or_else(|_| println!("No rpush_f32 callback, ignoring"));
        instance
            .set_callback("rpush_bool", make_rpush_callback::<bool>(con.clone()))
            .unwrap_or_else(|_| println!("No rpush_bool callback, ignoring"));
        instance
            .set_callback("rpush_string", make_rpush_callback_str(con.clone()))
            .unwrap_or_else(|_| println!("No rpush_string callback, ignoring"));

        instance.run();
    }
}
