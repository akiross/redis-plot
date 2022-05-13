/// These are the integration tests for redis-plot. They will build the library and start
/// redis-server (which must be in PATH) loading that module.
/// Then, some tests are run: rendering outputs are tested using reference images stored
/// tests/snapshots. You can use the show_rle_snap python utility (in flake.nix) to display
/// the image for inspection.
use anyhow::Result;
use itertools::Itertools;
use redis::Client;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

/// Is a drop guard that will kill the redis-server when dropped.
struct ServerGuard(std::process::Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Err(_) = self.0.kill() {
            println!("Couldn't kill the server");
        }
        if let Err(_) = self.0.wait() {
            println!("Couldn't wait the server");
        }
    }
}

impl ServerGuard {
    /// Opens a new connection to redis-server, using localhost and default port.
    /// It will attempt connection up to 10 times in 10 seconds, then fail.
    fn get_connection() -> Result<redis::Connection> {
        let client = redis::Client::open("redis://127.0.0.1/").expect("Cannot connect to server");
        for _ in 0..10 {
            match client.get_connection() {
                Ok(con) => {
                    // Connected, perform tests
                    return Ok(con);
                }
                Err(e) => {
                    if e.is_connection_refusal() {
                        // Wait for the server to be on line
                        std::thread::sleep_ms(1000);
                    } else {
                        // Cannot connect to server within a reasonable amount of time, fail.
                        return Err(e.into());
                    }
                }
            }
        }
        anyhow::bail!("Cannot connect to redis-server");
    }
}

/// This function builds the cdylib in a temporary directory, runs a callback
/// and returns its value removing the directory. If environment variable
/// REDIS_PLOT_TEST_TARGET_DIR is set, then that directory will be used as cargo target dir (which
/// might help for faster build times).
fn build_cdylib<F: FnOnce(&Path) -> Result<(), anyhow::Error>>(f: F) -> Result<(), anyhow::Error> {
    let temp_path = if let Some(p) = option_env!("REDIS_PLOT_TEST_TARGET_DIR") {
        PathBuf::from(p)
    } else {
        // Use a temporary directory for building the library, this is safer than using a fixed
        // directory, but will slow down build time during development.
        let temp_dir = tempdir().expect("Cannot create temporary directory");
        temp_dir.path().to_path_buf()
    };
    // TODO check building result: was cargo successful?
    std::process::Command::new("cargo")
        .args(["build", "--target-dir"])
        .arg(&temp_path)
        .output()
        .expect("Failed to build the library");
    let path = temp_path.join("debug").join("libredis_plot.so");
    // Ensure library was built and file is present
    assert!(path.exists());
    f(&path)
}

fn start_server(module_path: &Path) -> ServerGuard {
    // This is expected to be the path to the library
    // let module_path = env!("LIB_PATH");
    std::process::Command::new("redis-server")
        .arg("--loadmodule")
        .arg(module_path)
        .arg("--save") // Disable persistence, just noisy in tests.
        .arg("")
        .stdout(std::process::Stdio::null()) // TODO show output on failed tests
        .spawn()
        .map(ServerGuard)
        .expect("Cannot start redis-server")
}

/// RLE encoding of a slice.
fn rle(v: &[u8]) -> Vec<(usize, u8)> {
    // Apply RLE encoding to get something more manageable
    v.iter()
        .peekable()
        .batching(|it| {
            if let Some(first) = it.next() {
                // Count how many times we match the iterator
                let mut c: usize = 1;
                while let Some(n) = it.peek() {
                    if n == &first {
                        it.next();
                        c += 1;
                    } else {
                        // We are done finding this sequence
                        break;
                    }
                }
                Some((c, *first))
            } else {
                // No more elements in the iterator
                None
            }
        })
        .collect::<Vec<_>>()
}

#[test]
fn test_rle() {
    assert_eq!(rle(&vec![]), vec![]);
    assert_eq!(rle(&vec![0x01]), vec![(1, 0x01)]);
    assert_eq!(rle(&vec![0xff]), vec![(1, 0xff)]);
    assert_eq!(rle(&vec![0xff, 0xff, 0xff]), vec![(3, 0xff)]);
    assert_eq!(
        rle(&vec![0xff, 0xff, 0x00, 0x00, 0xff]),
        vec![(2, 0xff), (2, 0x00), (1, 0xff)]
    );
    assert_eq!(
        rle(&vec![0x11, 0x22, 0x33, 0x44, 0x55]),
        vec![(1, 0x11), (1, 0x22), (1, 0x33), (1, 0x44), (1, 0x55)]
    );
}

/// Represents a RLE-encoded RGB image and its shape, it's used by insta for
/// writing the snapshot file.
#[derive(Serialize)]
struct RleImage {
    width: usize,
    height: usize,
    rle_data: Vec<(usize, u8)>,
}

impl From<Vec<u8>> for RleImage {
    fn from(v: Vec<u8>) -> Self {
        Self {
            width: usize::from_be_bytes(v[..8].try_into().unwrap()),
            height: usize::from_be_bytes(v[8..16].try_into().unwrap()),
            rle_data: rle(&v[16..]),
        }
    }
}

/// Integration tests are placed here. There's currently a single test for
/// performance reasons. When developing, might be useful to set
/// REDIS_PLOT_TEST_TARGET_DIR to save some build time.
#[test]
fn test_everything() -> Result<(), anyhow::Error> {
    use redis::Commands;
    build_cdylib(|lib_path| {
        let _server = start_server(lib_path);
        let mut con = ServerGuard::get_connection()?;

        // This is just a smoke test.
        assert_eq!(
            redis::cmd("rsp.echo").arg("foo").arg("bar").query(&mut con),
            Ok("rsp.echo, foo, bar".to_owned())
        );

        // Set a value which is not a list
        let _: () = con.set("nl", 123i32)?;

        // Build a few lists as plot targets
        let la: i32 = con.rpush("la", vec![1u32, 2, 4, 9, 15, 16, 42])?;
        assert_eq!(la, 7);
        let lb: i32 = con.rpush("lb", vec![-1i32, 1, -2, 2, -3, 3, -4, 4])?;
        assert_eq!(lb, 8);
        con.rpush("lc", vec![0.0f32, 0.5, 1.75, -2.125])?;
        con.rpush(
            "ld",
            vec![0.125f32, 0.0, 0.0, 0.0, 125.0, 2.5, 3.0, 3.125, 2.95],
        )?;

        // Test that rsp.draw is requiring the --list argument with 1+ param.
        {
            println!("Test #1");
            assert!(redis::cmd("rsp.draw").query::<()>(&mut con).is_err());
            assert!(redis::cmd("rsp.draw")
                .arg("--list")
                .query::<()>(&mut con)
                .is_err());
        }

        // rsp.draw must accept lists only
        {
            println!("Test #2");
            assert!(redis::cmd("rsp.draw")
                .arg("--list")
                .arg("nl")
                .query::<()>(&mut con)
                .is_err());
        }

        // Test that rsp.draw can plot one list.
        {
            println!("Test #3");
            let res: Vec<u8> = redis::cmd("rsp.draw")
                .arg("--list")
                .arg("la")
                .query(&mut con)?;
            insta::assert_json_snapshot!(RleImage::from(res));
        }

        // Test that rsp.draw can plot more than one list.
        {
            println!("Test #4");
            let res: Vec<u8> = redis::cmd("rsp.draw")
                .arg("--list")
                .arg("la")
                .arg("lb")
                .arg("lc")
                .query(&mut con)?;
            insta::assert_json_snapshot!(RleImage::from(res));
        }

        // TODO plot one list as scatter, one as lines, one as histograms

        // TODO test async plotting: bind a list to a plot and try updating
        {
            println!("Test #5");
            // Bind a list to a plot, plot to key
            redis::cmd("rsp.bind")
                .arg("--list")
                .arg("la")
                .arg("--target")
                .arg("key:la_output")
                .query(&mut con)?;

            // Push a few data on an existing list
            con.rpush("la", vec![4u32, 8, 15, 16, 23, 42])?;

            // Check if output is as expected
            //TODO questo lo devo fare: scrivere la key in uscita con --target key:foo
            //e leggere qui il risultato, che non si sa quando sarà pronto...
            // la lettura async si dovrebbe poter fare usando le redis keyspace notifications
        }

        Ok(())
    })
}
