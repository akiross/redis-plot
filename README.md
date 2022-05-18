# redis-plot module

This is a redis module that collect data from keys and plot them on screen.

For now it's simple, stupid and dirty (as in: the code is a mess): it allows
to plot one or more lists as lines onto a window (or a buffer), but it's usable
for simple tasks.

There are a few automated tests, but they only check the routines plotting to
string. There is no automated tests involving the windowing part.

## Usage

```
$ cargo build
$ redis-server --loadmodule path/to/libredis_plot.so
```

starting redis-server with this module will open a window where the plotting
happens. You can close it, but there's currently no way to open it again.

Then, you can plot like this

```
$ redis-cli
> rsp.bind --list my_data
> rpush my_data 1 4 9 16 25
```

`rsp.bind` binds `my_data` to the plotting window: whenever the `my_data` list
is updated, the plot is updated as well. Only lists are supported right now.

If you have your backend and want to get the plotting somewhere different from
that simple window, you can use

```
> rpush my_data 1 2 3 4 5
> rsp.draw --list my_data
```

this will return a RGB buffer with the plot; the resolution is fixed right now.

## Testing

To perform integration testing it is necessary to build the cdylib somewhere and
start redis-server using that module: this is done automatically by the tests in
tests/integration.rs, but building will take some time to build both the library
and the tests.
During development, it might be useful to use the `REDIS_PLOT_TEST_TARGET_DIR`
to specify where the cdylib shall be built: if unset, a safe, random temporary
directory will be used for the build, but that variable can be set to force a
directory to use, resulting in faster subsequent builts - thus more useful if
needed to run the test several times.

Use `cargo insta test` to perform the acceptance tests, possibly reviewing the
outcome as instructed. Use `show_rle_snap` to inspect the snapshots.

## Future

The tool is not meant to become super duper complex, use gnuplot for that.

 - One plot kind/type for each window.
 - One or more series for each plot (window).
 - Using lists (or streams) as data structures.
 - Commands to plot in immediate mode or to automatically update the plot.
 - Output targets will be windows, files and binary strings.
   - A web client and/or jupyter integration is a cool idea, but not planned
     right now - with the same features OFC.

API will be something like this

```
# We need keyspace notification on lists (this could be done automatically)
> config set notify-keyspace-events Kl

# When my_data is updated, draw a scatter  plot
> rsp.scatter my_data

# When my_data is updated, draw a line plot
> rsp.lines my_data

# Finally data gets updated somehow, plot windows will change
> rpush my_data 1 2 3 4 5
```

### Command syntax

Command syntax currently adopted looks like this:

```
> rsp.command --foo foo1 foo2 --bar --baz baz1 --foo foo3
```

I am not sure this is the best choice, but looks decent for this use case and
it is familiar for people using unix CLI tools. Given the tool will likely stay
small for a while, this might be enough to specify input data sources, colors,
output targets and plot styles (most of which is not implemented yet).
